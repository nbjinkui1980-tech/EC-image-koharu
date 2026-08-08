//! Content-addressed blob storage for a `ProjectSession`.
//!
//! Blobs live at `.khrproj/blobs/ab/cdef…` (hex blake3 hash, sharded by the
//! first two chars). Immutable: a blob with a given hash is always the same
//! bytes. An in-memory LRU decodes images on demand.
//!
//! `BlobRef` itself lives in `koharu-core::blob`; this module only provides
//! the filesystem store + cache.

use std::io::Cursor;
use std::num::NonZeroUsize;
use std::path::{Path, PathBuf};

#[cfg(test)]
use std::cell::RefCell;

use anyhow::{Context, Result};
use image::{DynamicImage, RgbaImage};
use koharu_core::BlobRef;
use lru::LruCache;
use parking_lot::Mutex;

const RAW_MAGIC: &[u8; 4] = b"RGBA";
const DECODED_RGBA_BUDGET: u64 = 512 * 1024 * 1024; // 512 MiB
const DECODED_IMAGE_CACHE_BUDGET: usize = 512 * 1024 * 1024; // 512 MiB cache budget

#[cfg(test)]
thread_local! {
    static DECODE_TEST_EVENTS: RefCell<Vec<DecodeTestEvent>> = const { RefCell::new(Vec::new()) };
}

#[cfg(test)]
#[derive(Debug, Clone, PartialEq, Eq)]
enum DecodeTestEvent {
    CacheMiss(BlobRef),
    Sniffed(Option<image::ImageFormat>),
    RawLayout {
        width: u32,
        height: u32,
    },
    PixelConstructionStarted,
    Decoded {
        width: u32,
        height: u32,
        has_alpha: bool,
    },
}

#[cfg(test)]
fn record_decode_test_event(event: DecodeTestEvent) {
    DECODE_TEST_EVENTS.with_borrow_mut(|events| events.push(event));
}

#[cfg(test)]
fn take_decode_test_events() -> Vec<DecodeTestEvent> {
    DECODE_TEST_EVENTS.take()
}

/// Content-addressed blob store + decoded-image LRU.
pub struct BlobStore {
    root: PathBuf,
    cache: Mutex<(LruCache<BlobRef, DynamicImage>, usize)>,
}

fn decoded_image_bytes(img: &DynamicImage) -> usize {
    use image::ColorType;
    let channels = img.color().channel_count() as usize;
    let bytes_per_channel = match img.color() {
        ColorType::L8 | ColorType::La8 | ColorType::Rgb8 | ColorType::Rgba8 => 1,
        ColorType::L16 | ColorType::La16 | ColorType::Rgb16 | ColorType::Rgba16 => 2,
        ColorType::Rgb32F | ColorType::Rgba32F => 4,
        _ => 1,
    };
    (img.width() as usize) * (img.height() as usize) * channels * bytes_per_channel
}

impl BlobStore {
    /// Open (or create) the store at `root`. Directory is created if missing.
    pub fn open(root: impl Into<PathBuf>) -> Result<Self> {
        let root = root.into();
        std::fs::create_dir_all(&root)
            .with_context(|| format!("create blob root {}", root.display()))?;
        Ok(Self {
            root,
            cache: Mutex::new((LruCache::unbounded(), 0)),
        })
    }

    /// Root directory on disk.
    pub fn root(&self) -> &Path {
        &self.root
    }

    // --- raw bytes ---------------------------------------------------------

    /// Write raw bytes; return the blake3-derived `BlobRef`.
    pub fn put_bytes(&self, data: &[u8]) -> Result<BlobRef> {
        let hash = blake3::hash(data).to_hex().to_string();
        let path = self.blob_path(&hash);
        if !path.exists() {
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(&path, data).with_context(|| format!("write blob {hash}"))?;
        }
        Ok(BlobRef::new(hash))
    }

    /// Read raw bytes by `BlobRef`.
    pub fn get_bytes(&self, r: &BlobRef) -> Result<Vec<u8>> {
        let path = self.blob_path(r.hash());
        std::fs::read(&path).with_context(|| format!("blob not found: {}", r.hash()))
    }

    /// Whether a blob exists on disk (no decode, no cache touch).
    pub fn exists(&self, r: &BlobRef) -> bool {
        self.blob_path(r.hash()).exists()
    }

    // --- decoded images ----------------------------------------------------

    fn cache_put_with_eviction(&self, r: BlobRef, img: DynamicImage) {
        let bytes = decoded_image_bytes(&img);
        let (ref mut cache, ref mut total) = *self.cache.lock();
        while *total + bytes > DECODED_IMAGE_CACHE_BUDGET {
            if let Some((_, evicted)) = cache.pop_lru() {
                *total = total.saturating_sub(decoded_image_bytes(&evicted));
            } else {
                break;
            }
        }
        cache.put(r, img);
        *total += bytes;
    }

    /// Load and decode an image, using the LRU. Returns a cheap clone.
    pub fn load_image(&self, r: &BlobRef) -> Result<DynamicImage> {
        {
            let (ref mut cache, _) = *self.cache.lock();
            if let Some(img) = cache.get(r) {
                return Ok(img.clone());
            }
        }
        #[cfg(test)]
        record_decode_test_event(DecodeTestEvent::CacheMiss(r.clone()));
        let bytes = self.get_bytes(r)?;
        let img = decode_blob(&bytes)?;
        self.cache_put_with_eviction(r.clone(), img.clone());
        Ok(img)
    }

    /// Encode an image as WebP, store, cache, return ref.
    pub fn put_webp(&self, img: &DynamicImage) -> Result<BlobRef> {
        let mut buf = Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::WebP)?;
        let r = self.put_bytes(&buf.into_inner())?;
        self.cache_put_with_eviction(r.clone(), img.clone());
        Ok(r)
    }

    /// Store an image as raw RGBA with a 12-byte header. Cheap encode, used
    /// for sprites where WebP's compression gain doesn't justify its cost.
    pub fn put_raw(&self, img: &DynamicImage) -> Result<BlobRef> {
        let rgba = img.to_rgba8();
        let (w, h) = (rgba.width(), rgba.height());
        let pixels = rgba.as_raw();
        let mut buf = Vec::with_capacity(12 + pixels.len());
        buf.extend_from_slice(RAW_MAGIC);
        buf.extend_from_slice(&w.to_le_bytes());
        buf.extend_from_slice(&h.to_le_bytes());
        buf.extend_from_slice(pixels);
        let r = self.put_bytes(&buf)?;
        self.cache_put_with_eviction(r.clone(), img.clone());
        Ok(r)
    }

    /// Whether a blob uses our raw-RGBA wrapper (vs a standard image format).
    pub fn is_raw_rgba(&self, r: &BlobRef) -> bool {
        self.get_bytes(r)
            .map(|bytes| bytes.len() >= 4 && &bytes[..4] == RAW_MAGIC)
            .unwrap_or(false)
    }

    // --- internals ---------------------------------------------------------

    fn blob_path(&self, hash: &str) -> PathBuf {
        let (prefix, rest) = hash.split_at(2.min(hash.len()));
        self.root.join(prefix).join(rest)
    }
}

fn decode_blob(bytes: &[u8]) -> Result<DynamicImage> {
    #[cfg(test)]
    record_decode_test_event(DecodeTestEvent::Sniffed(image::guess_format(bytes).ok()));
    if bytes.len() >= 12 && &bytes[..4] == RAW_MAGIC {
        let w = u32::from_le_bytes(bytes[4..8].try_into().unwrap());
        let h = u32::from_le_bytes(bytes[8..12].try_into().unwrap());
        let raw_bytes = (w as u64)
            .checked_mul(h as u64)
            .and_then(|pixels| pixels.checked_mul(4))
            .unwrap_or(u64::MAX);
        anyhow::ensure!(
            raw_bytes <= DECODED_RGBA_BUDGET,
            "raw RGBA dimensions {w}x{h} exceed decoded budget"
        );
        #[cfg(test)]
        {
            record_decode_test_event(DecodeTestEvent::RawLayout {
                width: w,
                height: h,
            });
            record_decode_test_event(DecodeTestEvent::PixelConstructionStarted);
        }
        let pixels = bytes[12..].to_vec();
        let img = RgbaImage::from_raw(w, h, pixels).context("invalid raw RGBA blob dimensions")?;
        #[cfg(test)]
        record_decode_test_event(DecodeTestEvent::Decoded {
            width: img.width(),
            height: img.height(),
            has_alpha: true,
        });
        return     Ok(DynamicImage::ImageRgba8(img));
    }
    let mut limits = image::Limits::default();
    limits.reserve(DECODED_RGBA_BUDGET)
        .context("decoded RGBA budget exceeded")?;
    let img = image::load_from_memory(bytes)?;
    limits.free(DECODED_RGBA_BUDGET);
    #[cfg(test)]
    record_decode_test_event(DecodeTestEvent::Decoded {
        width: img.width(),
        height: img.height(),
        has_alpha: img.color().has_alpha(),
    });
    Ok(img)
}

/// Admit a source image from raw bytes for new ingress only (multipart,
/// path import, CLI `import_page`). Accepts PNG, JPEG, WebP; rejects
/// GIF, BMP, unknown, corrupt, and extension-spoofed input.
///
/// This is intentionally separate from `decode_blob` — existing project
/// blobs (including legacy GIF/BMP) must remain readable through the
/// normal decode path.
pub fn admit_source_image(bytes: &[u8]) -> Result<DynamicImage> {
    if bytes.is_empty() {
        anyhow::bail!("empty source image");
    }
    // Byte-sniff: check known magic bytes
    let format = match bytes {
        [0x89, 0x50, 0x4E, 0x47, ..] => {
            image::ImageFormat::Png
        }
        [0xFF, 0xD8, 0xFF, ..] => {
            image::ImageFormat::Jpeg
        }
        [0x52, 0x49, 0x46, 0x46, _, _, _, _, 0x57, 0x45, 0x42, 0x50, ..] => {
            image::ImageFormat::WebP
        }
        _ => {
            anyhow::bail!("unsupported source image format")
        }
    };
    let mut limits = image::Limits::default();
    limits.reserve(DECODED_RGBA_BUDGET)
        .context("decoded RGBA budget exceeded")?;
    let img = image::load_from_memory(bytes)
        .with_context(|| format!("cannot decode admitted {:?}", format))?;
    limits.free(DECODED_RGBA_BUDGET);
    Ok(img)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn put_and_get_round_trip() {
        let dir = tempdir().unwrap();
        let store = BlobStore::open(dir.path()).unwrap();
        let r = store.put_bytes(b"hello world").unwrap();
        assert!(!r.is_empty());
        let bytes = store.get_bytes(&r).unwrap();
        assert_eq!(bytes, b"hello world");
        assert!(store.exists(&r));
    }

    #[test]
    fn same_bytes_same_ref() {
        let dir = tempdir().unwrap();
        let store = BlobStore::open(dir.path()).unwrap();
        let a = store.put_bytes(b"x").unwrap();
        let b = store.put_bytes(b"x").unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn hanonly_pre_b1_red_t2_blob_decode_budget_contract() {
        const DECODED_RGBA_LIMIT: u64 = 512 * 1024 * 1024;
        const DECODED_IMAGE_CACHE_BUDGET: usize = 512 * 1024 * 1024;
        let dir = tempdir().unwrap();
        let store = BlobStore::open(dir.path()).unwrap();
        let mut violations = Vec::new();
        let rgba = DynamicImage::ImageRgba8(
            RgbaImage::from_raw(2, 1, vec![255, 0, 0, 0, 0, 255, 0, 255]).unwrap(),
        );

        for format in [
            image::ImageFormat::Png,
            image::ImageFormat::Jpeg,
            image::ImageFormat::WebP,
        ] {
            let mut encoded = Cursor::new(Vec::new());
            rgba.write_to(&mut encoded, format).unwrap();
            let blob = store.put_bytes(&encoded.into_inner()).unwrap();
            take_decode_test_events();
            let loaded = store.load_image(&blob);
            let events = take_decode_test_events();
            if loaded.is_err()
                || !events.contains(&DecodeTestEvent::Sniffed(Some(format)))
                || !events.iter().any(|e| {
                    matches!(
                        e,
                        DecodeTestEvent::Decoded {
                            width: 2,
                            height: 1,
                            ..
                        }
                    )
                })
            {
                violations.push(format!(
                    "{format:?} must be sniffed and decoded through load_image"
                ));
            }
            if format == image::ImageFormat::Png
                && !events.iter().any(|e| {
                    matches!(
                        e,
                        DecodeTestEvent::Decoded {
                            has_alpha: true,
                            ..
                        }
                    )
                })
            {
                violations.push("PNG alpha must survive decode".into());
            }
        }

        let mut raw = Vec::from(RAW_MAGIC);
        raw.extend_from_slice(&2_u32.to_le_bytes());
        raw.extend_from_slice(&1_u32.to_le_bytes());
        raw.extend_from_slice(rgba.as_bytes());
        let blob = store.put_bytes(&raw).unwrap();
        take_decode_test_events();
        if store.load_image(&blob).is_err()
            || !take_decode_test_events().iter().any(|e| {
                matches!(
                    e,
                    DecodeTestEvent::RawLayout {
                        width: 2,
                        height: 1,
                        ..
                    }
                )
            })
        {
            violations.push("raw RGBA must use the production raw decode branch".into());
        }

        for format in [image::ImageFormat::Gif, image::ImageFormat::Bmp] {
            let mut encoded = Cursor::new(Vec::new());
            rgba.write_to(&mut encoded, format).unwrap();
            let blob = store.put_bytes(&encoded.into_inner()).unwrap();
            take_decode_test_events();
            if store.load_image(&blob).is_err() {
                violations.push(format!(
                    "{format:?} must remain readable through decode_blob (legacy compatibility)"
                ));
            }
            take_decode_test_events();
        }

        let mut jpeg = Cursor::new(Vec::new());
        rgba.write_to(&mut jpeg, image::ImageFormat::Jpeg).unwrap();
        let mut jpeg = jpeg.into_inner();
        let exif = [
            b'E', b'x', b'i', b'f', 0, 0, b'I', b'I', 42, 0, 8, 0, 0, 0, 1, 0, 0x12, 0x01, 3, 0, 1,
            0, 0, 0, 6, 0, 0, 0, 0, 0, 0, 0,
        ];
        let mut oriented = vec![0xff, 0xd8, 0xff, 0xe1, 0, (exif.len() + 2) as u8];
        oriented.extend_from_slice(&exif);
        oriented.extend_from_slice(&jpeg.split_off(2));
        let blob = store.put_bytes(&oriented).unwrap();
        take_decode_test_events();
        let loaded = store.load_image(&blob);
        let events = take_decode_test_events();
        if loaded.is_err()
            || !events.contains(&DecodeTestEvent::Sniffed(Some(image::ImageFormat::Jpeg)))
        {
            violations.push("JPEG EXIF must be sniffed as JPEG and decoded successfully".into());
        }

        for (name, bytes, expected) in [
            ("exact", Some(DECODED_RGBA_LIMIT), true),
            ("plus-one", DECODED_RGBA_LIMIT.checked_add(1), false),
            ("overflow", u64::MAX.checked_add(1), false),
        ] {
            let approved = bytes.is_some_and(|bytes| {
                let mut limits = image::Limits::default();
                limits.reserve(bytes).is_ok()
            });
            if approved != expected {
                violations.push(format!(
                    "{name} decoded RGBA reservation: expected {expected}, got {approved}"
                ));
            }
        }

        let probe_raw_layout = |width, height| {
            let mut raw = Vec::from(RAW_MAGIC);
            raw.extend_from_slice(&u32::to_le_bytes(width));
            raw.extend_from_slice(&u32::to_le_bytes(height));
            let blob = store.put_bytes(&raw).unwrap();
            take_decode_test_events();
            let _ = store.load_image(&blob);
            take_decode_test_events()
        };
        for (name, events, construction_expected) in [
            ("exact", probe_raw_layout(16_384, 8_192), true),
            ("above-budget", probe_raw_layout(16_385, 8_192), false),
            ("overflow", probe_raw_layout(u32::MAX, u32::MAX), false),
        ] {
            let construction_started = events.contains(&DecodeTestEvent::PixelConstructionStarted);
            if construction_started != construction_expected {
                violations.push(format!(
                    "{name} raw layout pixel construction: expected {construction_expected}, got {construction_started}"
                ));
            }
        }

        let cache_image = DynamicImage::new_rgba8(2048, 2048);
        let decoded_bytes = cache_image.as_bytes().len();
        let image_count = DECODED_IMAGE_CACHE_BUDGET / decoded_bytes + 1;
        let mut encoded = Cursor::new(Vec::new());
        cache_image
            .write_to(&mut encoded, image::ImageFormat::Png)
            .unwrap();
        let encoded = encoded.into_inner();
        let mut refs = Vec::with_capacity(image_count);
        for value in 0..image_count {
            let mut bytes = encoded.clone();
            bytes.extend_from_slice(&(value as u64).to_le_bytes());
            refs.push(store.put_bytes(&bytes).unwrap());
        }
        take_decode_test_events();
        for blob in &refs {
            store.load_image(blob).unwrap();
        }
        take_decode_test_events();
        store.load_image(&refs[0]).unwrap();
        let reload = take_decode_test_events();
        if !reload.contains(&DecodeTestEvent::CacheMiss(refs[0].clone()))
            || !reload.iter().any(|event| {
                matches!(
                    event,
                    DecodeTestEvent::Decoded {
                        width: 2048,
                        height: 2048,
                        ..
                    }
                )
            })
        {
            violations.push(format!(
                "decoded-image cache must miss and re-decode after {} bytes exceed its {DECODED_IMAGE_CACHE_BUDGET}-byte budget",
                image_count * decoded_bytes
            ));
        }

        assert!(
            violations.is_empty(),
            "G002 blob contract violations:\n{}",
            violations.join("\n")
        );
    }
}
