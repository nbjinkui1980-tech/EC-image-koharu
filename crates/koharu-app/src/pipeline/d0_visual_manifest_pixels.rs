//! Test-only D0 dimension, decode, and blind-mask validation.
//!
//! This stage consumes only immutable held bytes. It intentionally does not
//! validate ROI, protected-region, disjointness, Source/Clean edit, M_delta,
//! or unsupported-mode pixel semantics.

use std::collections::HashSet;
use std::error::Error;
use std::fmt;
use std::io::{self, Cursor};

use image::metadata::Orientation;
use image::{DynamicImage, ImageDecoder, ImageFormat, ImageReader, RgbaImage, guess_format};

use super::d0_revision_46_contract::BYTE_CEILING;
use super::d0_visual_manifest_schema::HeldVisualManifestSchema;

const FORMAT_ERROR: &str = "d0.visual_pixels.format";
const PROBE_ERROR: &str = "d0.visual_pixels.probe";
const DIMENSION_ERROR: &str = "d0.visual_pixels.dimensions";
const AGGREGATE_ERROR: &str = "d0.visual_pixels.aggregate";
const BUDGET_ERROR: &str = "d0.visual_pixels.budget";
const DECODE_ERROR: &str = "d0.visual_pixels.decode";
const DECODED_HASH_ERROR: &str = "d0.visual_pixels.decoded_hash";
const MASK_BINARY_ERROR: &str = "d0.visual_pixels.mask_binary";
const MASK_EMPTY_ERROR: &str = "d0.visual_pixels.mask_empty";
const MASK_DISAGREEMENT_ERROR: &str = "d0.visual_pixels.mask_disagreement";

type D0PixelResult<T> = Result<T, D0PixelError>;

#[derive(Debug)]
pub(super) struct D0PixelError {
    category: &'static str,
    source: Box<dyn Error + Send + Sync>,
}

impl D0PixelError {
    pub(super) fn category(&self) -> &'static str {
        self.category
    }
}

impl fmt::Display for D0PixelError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.category, self.source)
    }
}

impl Error for D0PixelError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}

pub(super) struct DimensionAndMaskValidatedManifest {
    pub(super) held_schema: HeldVisualManifestSchema,
    pub(super) entries: Vec<DimensionAndMaskValidatedEntry>,
    pub(super) final_retained_bytes: u64,
    peak_reserved_bytes: u64,
}

pub(super) struct DimensionAndMaskValidatedEntry {
    pub(super) source: RgbaImage,
    pub(super) clean_reference: RgbaImage,
    pub(super) targets: Vec<DimensionAndMaskValidatedTarget>,
}

pub(super) struct DimensionAndMaskValidatedTarget {
    pub(super) agreed_mask: Box<[u8]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AssetProbe {
    format: ImageFormat,
    orientation: Orientation,
    raw_dimensions: (u32, u32),
    oriented_dimensions: (u32, u32),
    rgba_bytes: u64,
    decoder_bytes: u64,
}

struct EntryProbe {
    source: AssetProbe,
    clean_reference: AssetProbe,
    targets: Vec<TargetProbe>,
}

struct TargetProbe {
    erase: AssetProbe,
    residual: AssetProbe,
}

struct DecodedEntry {
    source: RgbaImage,
    clean_reference: RgbaImage,
    source_hash: String,
    clean_hash: String,
}

pub(super) fn canonical_decoded_rgba_blake3(bytes: &[u8]) -> D0PixelResult<String> {
    let mut budget = SharedBudget::new(BYTE_CEILING);
    let probe = probe_asset(bytes, &mut budget)?;
    let rgba = decode_retained_rgba(bytes, probe, &mut budget)?;
    Ok(blake3_hex(&rgba))
}

pub(super) fn validate_dimensions_and_masks(
    held_schema: HeldVisualManifestSchema,
) -> D0PixelResult<DimensionAndMaskValidatedManifest> {
    validate_dimensions_and_masks_with_decoded_limit(held_schema, BYTE_CEILING)
}

fn validate_dimensions_and_masks_with_decoded_limit(
    held_schema: HeldVisualManifestSchema,
    decoded_limit: u64,
) -> D0PixelResult<DimensionAndMaskValidatedManifest> {
    validate_dimensions_and_masks_with_policy(held_schema, decoded_limit, true)
}

fn validate_dimensions_and_masks_with_policy(
    held_schema: HeldVisualManifestSchema,
    decoded_limit: u64,
    release_residual: bool,
) -> D0PixelResult<DimensionAndMaskValidatedManifest> {
    validate_aggregate_encoded_bytes(&held_schema)?;
    let mut budget = SharedBudget::new(decoded_limit);
    let probes = probe_all_assets(&held_schema, &mut budget)?;
    validate_all_dimension_relationships(&probes)?;

    let mut decoded = Vec::with_capacity(held_schema.entries.len());
    for (held_entry, probe) in held_schema.entries.iter().zip(&probes) {
        let source = decode_retained_rgba(held_entry.source.bytes(), probe.source, &mut budget)?;
        let clean_reference = decode_retained_rgba(
            held_entry.clean_reference.bytes(),
            probe.clean_reference,
            &mut budget,
        )?;
        decoded.push(DecodedEntry {
            source_hash: blake3_hex(&source),
            clean_hash: blake3_hex(&clean_reference),
            source,
            clean_reference,
        });
    }
    validate_actual_decoded_hashes(&held_schema, &decoded)?;

    let mut entries = Vec::with_capacity(decoded.len());
    for ((decoded, held_entry), probe) in decoded.into_iter().zip(&held_schema.entries).zip(probes)
    {
        let mut targets = Vec::with_capacity(held_entry.targets.len());
        for (held_target, target_probe) in held_entry.targets.iter().zip(probe.targets) {
            let erase = decode_binary_mask(
                held_target.erase_source_ink_mask.bytes(),
                target_probe.erase,
                &mut budget,
            )?;
            let residual = decode_binary_mask(
                held_target.residual_source_ink_mask.bytes(),
                target_probe.residual,
                &mut budget,
            )?;
            require(
                erase == residual,
                MASK_DISAGREEMENT_ERROR,
                "erase and residual masks disagree",
            )?;
            if release_residual {
                budget
                    .release(target_probe.residual.rgba_bytes / 4)
                    .map_err(|source| context(BUDGET_ERROR, source))?;
            }
            drop(residual);
            targets.push(DimensionAndMaskValidatedTarget { agreed_mask: erase });
        }
        entries.push(DimensionAndMaskValidatedEntry {
            source: decoded.source,
            clean_reference: decoded.clean_reference,
            targets,
        });
    }

    let final_retained_bytes = budget.used;
    let peak_reserved_bytes = budget.peak;
    Ok(DimensionAndMaskValidatedManifest {
        held_schema,
        entries,
        final_retained_bytes,
        peak_reserved_bytes,
    })
}

fn probe_all_assets(
    held: &HeldVisualManifestSchema,
    budget: &mut SharedBudget,
) -> D0PixelResult<Vec<EntryProbe>> {
    let mut probes = Vec::with_capacity(held.entries.len());
    for entry in &held.entries {
        let source = probe_asset(entry.source.bytes(), budget)?;
        let clean_reference = probe_asset(entry.clean_reference.bytes(), budget)?;
        let mut targets = Vec::with_capacity(entry.targets.len());
        for target in &entry.targets {
            targets.push(TargetProbe {
                erase: probe_asset(target.erase_source_ink_mask.bytes(), budget)?,
                residual: probe_asset(target.residual_source_ink_mask.bytes(), budget)?,
            });
        }
        probes.push(EntryProbe {
            source,
            clean_reference,
            targets,
        });
    }
    Ok(probes)
}

fn probe_asset(bytes: &[u8], budget: &mut SharedBudget) -> D0PixelResult<AssetProbe> {
    let format = guess_format(bytes).map_err(|source| context(FORMAT_ERROR, source))?;
    require(
        matches!(
            format,
            ImageFormat::Png | ImageFormat::Jpeg | ImageFormat::WebP
        ),
        FORMAT_ERROR,
        "encoded format is not PNG, JPEG, or WebP",
    )?;
    with_probe_reservation(bytes.len() as u64, budget, |_| {
        let mut decoder = ImageReader::with_format(Cursor::new(bytes), format)
            .into_decoder()
            .map_err(|source| context(PROBE_ERROR, source))?;
        let orientation = decoder
            .orientation()
            .map_err(|source| context(PROBE_ERROR, source))?;
        let raw_dimensions = decoder.dimensions();
        let oriented_dimensions = oriented_dimensions(raw_dimensions, orientation);
        let rgba_bytes = checked_rgba_bytes(
            u64::from(oriented_dimensions.0),
            u64::from(oriented_dimensions.1),
            BYTE_CEILING,
        )
        .map_err(|source| context(DIMENSION_ERROR, source))?;
        Ok(AssetProbe {
            format,
            orientation,
            raw_dimensions,
            oriented_dimensions,
            rgba_bytes,
            decoder_bytes: decoder.total_bytes(),
        })
    })
}

fn with_probe_reservation<T>(
    encoded_bytes: u64,
    budget: &mut SharedBudget,
    create_decoder: impl FnOnce(u64) -> D0PixelResult<T>,
) -> D0PixelResult<T> {
    let temporary =
        checked_probe_temporary(encoded_bytes).map_err(|source| context(BUDGET_ERROR, source))?;
    let reservation = budget
        .reserve(temporary)
        .map_err(|source| context(BUDGET_ERROR, source))?;
    let used_while_reserved = reservation.budget.used;
    let result = create_decoder(used_while_reserved);
    drop(reservation);
    result
}

fn validate_all_dimension_relationships(probes: &[EntryProbe]) -> D0PixelResult<()> {
    for entry in probes {
        require(
            entry.source.oriented_dimensions == entry.clean_reference.oriented_dimensions,
            DIMENSION_ERROR,
            "Source and Clean dimensions differ",
        )?;
        for target in &entry.targets {
            require(
                target.erase.oriented_dimensions == entry.source.oriented_dimensions
                    && target.residual.oriented_dimensions == entry.source.oriented_dimensions,
                DIMENSION_ERROR,
                "mask dimensions differ from page dimensions",
            )?;
        }
    }
    Ok(())
}

fn validate_aggregate_encoded_bytes(held: &HeldVisualManifestSchema) -> D0PixelResult<()> {
    let lengths = held.entries.iter().flat_map(|entry| {
        [
            entry.source.bytes().len(),
            entry.clean_reference.bytes().len(),
        ]
        .into_iter()
        .chain(entry.targets.iter().flat_map(|target| {
            [
                target.erase_source_ink_mask.bytes().len(),
                target.residual_source_ink_mask.bytes().len(),
            ]
        }))
    });
    checked_aggregate_bytes(lengths.map(|length| length as u64), BYTE_CEILING)
        .map(|_| ())
        .map_err(|source| context(AGGREGATE_ERROR, source))
}

fn validate_actual_decoded_hashes(
    held: &HeldVisualManifestSchema,
    decoded: &[DecodedEntry],
) -> D0PixelResult<()> {
    let mut source_hashes = HashSet::new();
    let mut clean_hashes = HashSet::new();
    for entry in decoded {
        require(
            source_hashes.insert(entry.source_hash.as_str()),
            DECODED_HASH_ERROR,
            "actual Source decoded hash is duplicated",
        )?;
        require(
            clean_hashes.insert(entry.clean_hash.as_str()),
            DECODED_HASH_ERROR,
            "actual Clean decoded hash is duplicated",
        )?;
    }
    for (schema, entry) in held.schema.entries.iter().zip(decoded) {
        require(
            schema.decoded_rgba_blake3 == entry.source_hash,
            DECODED_HASH_ERROR,
            "Source decoded rgba blake3 mismatch",
        )?;
        require(
            schema.clean_reference_decoded_rgba_blake3 == entry.clean_hash,
            DECODED_HASH_ERROR,
            "Clean decoded rgba blake3 mismatch",
        )?;
    }
    Ok(())
}

fn decode_retained_rgba(
    bytes: &[u8],
    probe: AssetProbe,
    budget: &mut SharedBudget,
) -> D0PixelResult<RgbaImage> {
    let temporary = checked_decode_peak(bytes.len() as u64, probe, 0)
        .map_err(|source| context(BUDGET_ERROR, source))?;
    let reservation = budget
        .reserve(temporary)
        .map_err(|source| context(BUDGET_ERROR, source))?;
    let rgba = decode_rgba(bytes, probe)?;
    reservation
        .commit(probe.rgba_bytes)
        .map_err(|source| context(BUDGET_ERROR, source))?;
    Ok(rgba)
}

fn decode_binary_mask(
    bytes: &[u8],
    probe: AssetProbe,
    budget: &mut SharedBudget,
) -> D0PixelResult<Box<[u8]>> {
    let pixels = probe
        .rgba_bytes
        .checked_div(4)
        .ok_or_else(|| context(BUDGET_ERROR, io::Error::other("invalid RGBA reservation")))?;
    let temporary = checked_decode_peak(bytes.len() as u64, probe, pixels)
        .map_err(|source| context(BUDGET_ERROR, source))?;
    let reservation = budget
        .reserve(temporary)
        .map_err(|source| context(BUDGET_ERROR, source))?;
    let rgba = decode_rgba(bytes, probe)?;
    let capacity = usize::try_from(pixels).map_err(|source| context(BUDGET_ERROR, source))?;
    let mut mask = Vec::with_capacity(capacity);
    let mut nonempty = false;
    for pixel in rgba.pixels() {
        match pixel.0 {
            [0, 0, 0, 255] => mask.push(0),
            [255, 255, 255, 255] => {
                mask.push(1);
                nonempty = true;
            }
            _ => {
                return Err(failure(
                    MASK_BINARY_ERROR,
                    "mask pixel is not opaque black or opaque white",
                ));
            }
        }
    }
    require(nonempty, MASK_EMPTY_ERROR, "mask foreground is empty")?;
    reservation
        .commit(pixels)
        .map_err(|source| context(BUDGET_ERROR, source))?;
    Ok(mask.into_boxed_slice())
}

fn decode_rgba(bytes: &[u8], probe: AssetProbe) -> D0PixelResult<RgbaImage> {
    let mut decoder = ImageReader::with_format(Cursor::new(bytes), probe.format)
        .into_decoder()
        .map_err(|source| context(DECODE_ERROR, source))?;
    let orientation = decoder
        .orientation()
        .map_err(|source| context(DECODE_ERROR, source))?;
    require(
        orientation == probe.orientation && decoder.dimensions() == probe.raw_dimensions,
        DECODE_ERROR,
        "decoder metadata changed between probe and decode",
    )?;
    let mut image =
        DynamicImage::from_decoder(decoder).map_err(|source| context(DECODE_ERROR, source))?;
    image.apply_orientation(orientation);
    let rgba = image.into_rgba8();
    require(
        rgba.dimensions() == probe.oriented_dimensions
            && rgba.as_raw().len() as u64 == probe.rgba_bytes,
        DECODE_ERROR,
        "decoded RGBA dimensions or length drifted",
    )?;
    Ok(rgba)
}

fn oriented_dimensions(dimensions: (u32, u32), orientation: Orientation) -> (u32, u32) {
    if matches!(
        orientation,
        Orientation::Rotate90
            | Orientation::Rotate270
            | Orientation::Rotate90FlipH
            | Orientation::Rotate270FlipH
    ) {
        (dimensions.1, dimensions.0)
    } else {
        dimensions
    }
}

fn checked_rgba_bytes(width: u64, height: u64, limit: u64) -> io::Result<u64> {
    width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(4))
        .filter(|bytes| *bytes <= limit)
        .ok_or_else(|| io::Error::other("oriented RGBA bytes exceed the shared byte ceiling"))
}

fn checked_aggregate_bytes(lengths: impl IntoIterator<Item = u64>, limit: u64) -> io::Result<u64> {
    lengths
        .into_iter()
        .try_fold(0_u64, |total, length| total.checked_add(length))
        .filter(|total| *total <= limit)
        .ok_or_else(|| io::Error::other("aggregate encoded bytes exceed the shared byte ceiling"))
}

fn checked_decode_peak(encoded_bytes: u64, probe: AssetProbe, mask_bytes: u64) -> io::Result<u64> {
    let probe_or_input = checked_probe_temporary(encoded_bytes)?;
    let decoder = encoded_bytes
        .checked_add(probe.decoder_bytes)
        .ok_or_else(|| io::Error::other("decoder input/output reservation overflow"))?;
    let orientation = if probe.orientation == Orientation::NoTransforms {
        probe.decoder_bytes
    } else {
        probe
            .decoder_bytes
            .checked_mul(2)
            .ok_or_else(|| io::Error::other("orientation reservation overflow"))?
    };
    let rgba = probe
        .decoder_bytes
        .checked_add(probe.rgba_bytes)
        .ok_or_else(|| io::Error::other("native-to-RGBA reservation overflow"))?;
    let mask = probe
        .rgba_bytes
        .checked_add(mask_bytes)
        .ok_or_else(|| io::Error::other("RGBA-to-mask reservation overflow"))?;
    Ok(probe_or_input
        .max(decoder)
        .max(orientation)
        .max(rgba)
        .max(mask))
}

fn checked_probe_temporary(encoded_bytes: u64) -> io::Result<u64> {
    encoded_bytes
        .checked_mul(2)
        .ok_or_else(|| io::Error::other("probe temporary reservation overflow"))
}

fn blake3_hex(image: &RgbaImage) -> String {
    blake3::hash(image.as_raw()).to_hex().to_string()
}

struct SharedBudget {
    used: u64,
    limit: u64,
    peak: u64,
}

impl SharedBudget {
    fn new(limit: u64) -> Self {
        Self {
            used: 0,
            limit,
            peak: 0,
        }
    }

    fn reserve(&mut self, bytes: u64) -> io::Result<BudgetReservation<'_>> {
        let used = self
            .used
            .checked_add(bytes)
            .filter(|used| *used <= self.limit)
            .ok_or_else(|| io::Error::other("shared decoded reservation exceeded"))?;
        self.used = used;
        self.peak = self.peak.max(used);
        Ok(BudgetReservation {
            budget: self,
            temporary: bytes,
        })
    }

    fn release(&mut self, bytes: u64) -> io::Result<()> {
        self.used = self
            .used
            .checked_sub(bytes)
            .ok_or_else(|| io::Error::other("shared decoded reservation release underflow"))?;
        Ok(())
    }
}

struct BudgetReservation<'a> {
    budget: &'a mut SharedBudget,
    temporary: u64,
}

impl BudgetReservation<'_> {
    fn commit(mut self, retained: u64) -> io::Result<()> {
        if retained > self.temporary {
            return Err(io::Error::other(
                "retained bytes exceed temporary reservation",
            ));
        }
        self.budget.used = self
            .budget
            .used
            .checked_sub(self.temporary)
            .and_then(|used| used.checked_add(retained))
            .ok_or_else(|| io::Error::other("shared decoded reservation accounting drift"))?;
        self.temporary = 0;
        Ok(())
    }
}

impl Drop for BudgetReservation<'_> {
    fn drop(&mut self) {
        self.budget.used = self
            .budget
            .used
            .checked_sub(self.temporary)
            .expect("temporary reservation underflow");
    }
}

fn require(condition: bool, category: &'static str, message: &'static str) -> D0PixelResult<()> {
    condition
        .then_some(())
        .ok_or_else(|| failure(category, message))
}

fn failure(category: &'static str, message: &'static str) -> D0PixelError {
    context(
        category,
        io::Error::new(io::ErrorKind::InvalidData, message),
    )
}

fn context(category: &'static str, source: impl Error + Send + Sync + 'static) -> D0PixelError {
    D0PixelError {
        category,
        source: Box::new(source),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    use image::codecs::jpeg::JpegEncoder;
    use image::{ExtendedColorType, ImageEncoder, Rgb, RgbImage, Rgba};
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    use tempfile::TempDir;

    use super::super::d0_visual_manifest_schema::load_schema_and_hold_assets;

    struct Fixture {
        _temp: TempDir,
        manifest_path: PathBuf,
        value: Value,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = TempDir::new().unwrap();
            let root = fs::canonicalize(temp.path()).unwrap();
            let mut entries = Vec::new();
            for index in 0..9 {
                let source_image = source_image(index);
                let clean_image = clean_image(index);
                let mask_image = mask_image(index);
                let source = write_asset(&root, index, "source", &png(&source_image));
                let clean = write_asset(&root, index, "clean", &png(&clean_image));
                let erase = write_asset(&root, index, "erase", &png(&mask_image));
                let residual = write_asset(&root, index, "residual", &png(&mask_image));
                entries.push(json!({
                    "id": format!("entry-{index}"),
                    "path": source.0,
                    "sha256": source.1,
                    "decoded_rgba_blake3": blake3_hex(&source_image),
                    "clean_reference_path": clean.0,
                    "clean_reference_sha256": clean.1,
                    "clean_reference_decoded_rgba_blake3": blake3_hex(&clean_image),
                    "role": match index {
                        0 => "regression",
                        1..=4 => "calibration",
                        _ => "holdout",
                    },
                    "dimension_bin": "lt720",
                    "aspect": "square_or_near",
                    "background": "pure",
                    "targets": [{
                        "id": "target-0",
                        "source_roi": [0, 0, 1, 1],
                        "clean_reference_edit_roi": [0, 0, 1, 1],
                        "erase_source_ink_mask_path": erase.0,
                        "erase_source_ink_mask_sha256": erase.1,
                        "residual_source_ink_mask_path": residual.0,
                        "residual_source_ink_mask_sha256": residual.1,
                        "position": "interior",
                        "writing": "horizontal",
                        "effect": if index == 5 { "stroke" } else { "plain" },
                        "translation_length": "short",
                        "expected": "automatic_strict"
                    }],
                    "protected_rois": [],
                    "multi_node": false
                }));
            }
            Self {
                _temp: temp,
                manifest_path: root.join("manifest.json"),
                value: json!({"version": 1, "entries": entries}),
            }
        }

        fn replace_source(&mut self, index: usize, bytes: &[u8], decoded: Option<&RgbaImage>) {
            replace_asset(&mut self.value["entries"][index], "path", "sha256", bytes);
            if let Some(decoded) = decoded {
                self.value["entries"][index]["decoded_rgba_blake3"] = blake3_hex(decoded).into();
            }
        }

        fn replace_clean(&mut self, index: usize, image: &RgbaImage) {
            self.replace_clean_bytes(index, &png(image), Some(image));
        }

        fn replace_clean_bytes(&mut self, index: usize, bytes: &[u8], decoded: Option<&RgbaImage>) {
            replace_asset(
                &mut self.value["entries"][index],
                "clean_reference_path",
                "clean_reference_sha256",
                bytes,
            );
            if let Some(decoded) = decoded {
                self.value["entries"][index]["clean_reference_decoded_rgba_blake3"] =
                    blake3_hex(decoded).into();
            }
        }

        fn replace_mask(&mut self, index: usize, kind: &str, image: &RgbaImage) {
            let (path_field, hash_field) = match kind {
                "erase" => ("erase_source_ink_mask_path", "erase_source_ink_mask_sha256"),
                "residual" => (
                    "residual_source_ink_mask_path",
                    "residual_source_ink_mask_sha256",
                ),
                _ => panic!("unknown mask kind"),
            };
            replace_asset(
                &mut self.value["entries"][index]["targets"][0],
                path_field,
                hash_field,
                &png(image),
            );
        }

        fn load_schema(&self) -> HeldVisualManifestSchema {
            let bytes = serde_json::to_vec(&self.value).unwrap();
            fs::write(&self.manifest_path, &bytes).unwrap();
            let regression = &self.value["entries"][0];
            load_schema_and_hold_assets(
                &self.manifest_path,
                &sha256(&bytes),
                Path::new(regression["path"].as_str().unwrap()),
                regression["decoded_rgba_blake3"].as_str().unwrap(),
                regression["sha256"].as_str().unwrap(),
            )
            .unwrap()
        }

        fn validate(&self) -> D0PixelResult<DimensionAndMaskValidatedManifest> {
            let schema = self.load_schema();
            validate_dimensions_and_masks(schema)
        }
    }

    fn source_image(index: usize) -> RgbaImage {
        RgbaImage::from_fn(4, 4, |x, y| {
            Rgba([
                (index as u8).wrapping_mul(17).wrapping_add(x as u8),
                20_u8.wrapping_add(y as u8),
                40,
                255,
            ])
        })
    }

    fn clean_image(index: usize) -> RgbaImage {
        RgbaImage::from_fn(4, 4, |x, y| {
            Rgba([
                150_u8.wrapping_add(index as u8).wrapping_add(x as u8),
                80_u8.wrapping_add(y as u8),
                90,
                255,
            ])
        })
    }

    fn mask_image(index: usize) -> RgbaImage {
        RgbaImage::from_fn(4, 4, |x, y| {
            if (y * 4 + x) as usize == index {
                Rgba([255, 255, 255, 255])
            } else {
                Rgba([0, 0, 0, 255])
            }
        })
    }

    fn png(image: &RgbaImage) -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image.clone())
            .write_to(&mut output, ImageFormat::Png)
            .unwrap();
        output.into_inner()
    }

    fn encoded(image: &RgbaImage, format: ImageFormat) -> Vec<u8> {
        let mut output = Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image.clone())
            .write_to(&mut output, format)
            .unwrap();
        output.into_inner()
    }

    fn write_asset(root: &Path, index: usize, kind: &str, bytes: &[u8]) -> (String, String) {
        let path = root.join(format!("{index}-{kind}.bmp"));
        fs::write(&path, bytes).unwrap();
        (path.to_str().unwrap().to_owned(), sha256(bytes))
    }

    fn replace_asset(value: &mut Value, path_field: &str, hash_field: &str, bytes: &[u8]) {
        let path = value[path_field].as_str().unwrap();
        fs::write(path, bytes).unwrap();
        value[hash_field] = sha256(bytes).into();
    }

    fn sha256(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn expect_error<T>(result: D0PixelResult<T>, category: &'static str) -> D0PixelError {
        let error = match result {
            Ok(_) => panic!("expected pixel validation rejection"),
            Err(error) => error,
        };
        assert_eq!(error.category(), category);
        assert!(error.to_string().starts_with(&format!("{category}: ")));
        assert!(
            error
                .source()
                .is_some_and(|source| !source.to_string().is_empty())
        );
        error
    }

    #[test]
    fn d0_visual_manifest_pixels_accepts_valid_nine_and_holds_validated_data() {
        let fixture = Fixture::new();
        let validated = fixture.validate().unwrap();
        assert_eq!(validated.entries.len(), 9);
        assert_eq!(validated.entries[0].source.dimensions(), (4, 4));
        assert_eq!(validated.entries[0].clean_reference.dimensions(), (4, 4));
        assert_eq!(validated.entries[0].targets[0].agreed_mask.len(), 16);
        assert_eq!(
            validated.entries[0].targets[0]
                .agreed_mask
                .iter()
                .sum::<u8>(),
            1
        );
        assert!(!validated.held_schema.manifest.bytes().is_empty());
    }

    #[test]
    fn d0_visual_manifest_pixels_accepts_webp_sniffed_from_mismatched_name() {
        let mut fixture = Fixture::new();
        let source = source_image(1);
        let bytes = encoded(&source, ImageFormat::WebP);
        fixture.replace_source(1, &bytes, Some(&source));
        assert_eq!(
            canonical_decoded_rgba_blake3(&bytes).unwrap(),
            blake3_hex(&source)
        );
        fixture.validate().unwrap();
    }

    #[test]
    fn d0_visual_manifest_pixels_fingerprints_png_jpeg_and_webp_canonical_rgba() {
        let source = source_image(2);
        for format in [ImageFormat::Png, ImageFormat::Jpeg, ImageFormat::WebP] {
            let bytes = encoded(&source, format);
            let decoded = image::load_from_memory(&bytes).unwrap().into_rgba8();
            assert_eq!(
                canonical_decoded_rgba_blake3(&bytes).unwrap(),
                blake3_hex(&decoded),
                "{format:?}"
            );
        }
    }

    #[test]
    fn d0_visual_manifest_pixels_rejects_bmp_unknown_and_corrupt_bytes() {
        let cases = [
            (
                encoded(&source_image(1), ImageFormat::Bmp),
                FORMAT_ERROR,
                false,
            ),
            (b"not-an-image".to_vec(), FORMAT_ERROR, true),
            (b"\x89PNG\r\n\x1a\ncorrupt".to_vec(), PROBE_ERROR, true),
        ];
        for (bytes, category, image_source) in cases {
            let mut fixture = Fixture::new();
            fixture.replace_source(1, &bytes, None);
            let error = expect_error(fixture.validate(), category);
            assert_eq!(
                error
                    .source()
                    .unwrap()
                    .downcast_ref::<image::ImageError>()
                    .is_some(),
                image_source
            );
        }
    }

    #[test]
    fn d0_visual_manifest_pixels_rejects_decoded_hash_mismatch() {
        let mut fixture = Fixture::new();
        fixture.value["entries"][1]["decoded_rgba_blake3"] = "0".repeat(64).into();
        let error = expect_error(fixture.validate(), DECODED_HASH_ERROR);
        assert_eq!(
            error.source().unwrap().to_string(),
            "Source decoded rgba blake3 mismatch"
        );
    }

    #[test]
    fn d0_visual_manifest_pixels_rejects_source_clean_dimension_mismatch() {
        let mut fixture = Fixture::new();
        let clean = RgbaImage::from_pixel(3, 4, Rgba([1, 2, 3, 255]));
        fixture.replace_clean(1, &clean);
        let error = expect_error(fixture.validate(), DIMENSION_ERROR);
        assert_eq!(
            error.source().unwrap().to_string(),
            "Source and Clean dimensions differ"
        );
    }

    #[test]
    fn d0_visual_manifest_pixels_rejects_nonbinary_and_transparent_masks() {
        for pixel in [Rgba([1, 1, 1, 255]), Rgba([255, 255, 255, 0])] {
            let mut fixture = Fixture::new();
            let mut mask = mask_image(1);
            mask.put_pixel(0, 0, pixel);
            fixture.replace_mask(1, "erase", &mask);
            let error = expect_error(fixture.validate(), MASK_BINARY_ERROR);
            assert_eq!(
                error.source().unwrap().to_string(),
                "mask pixel is not opaque black or opaque white"
            );
        }
    }

    #[test]
    fn d0_visual_manifest_pixels_rejects_empty_mask() {
        let mut fixture = Fixture::new();
        let empty = RgbaImage::from_pixel(4, 4, Rgba([0, 0, 0, 255]));
        fixture.replace_mask(1, "erase", &empty);
        fixture.replace_mask(1, "residual", &empty);
        let error = expect_error(fixture.validate(), MASK_EMPTY_ERROR);
        assert_eq!(
            error.source().unwrap().to_string(),
            "mask foreground is empty"
        );
    }

    #[test]
    fn d0_visual_manifest_pixels_rejects_dual_mask_disagreement() {
        let mut fixture = Fixture::new();
        fixture.replace_mask(1, "residual", &mask_image(2));
        let error = expect_error(fixture.validate(), MASK_DISAGREEMENT_ERROR);
        assert_eq!(
            error.source().unwrap().to_string(),
            "erase and residual masks disagree"
        );
    }

    #[test]
    fn d0_visual_manifest_pixels_rejects_actual_source_and_clean_decoded_duplicates() {
        for (kind, expected) in [
            ("source", "actual Source decoded hash is duplicated"),
            ("clean", "actual Clean decoded hash is duplicated"),
        ] {
            let mut fixture = Fixture::new();
            let path_field = if kind == "source" {
                "path"
            } else {
                "clean_reference_path"
            };
            let first_path = fixture.value["entries"][1][path_field].as_str().unwrap();
            let duplicate = fs::read(first_path).unwrap();
            if kind == "source" {
                fixture.replace_source(2, &duplicate, None);
            } else {
                fixture.replace_clean_bytes(2, &duplicate, None);
            }
            let error = expect_error(fixture.validate(), DECODED_HASH_ERROR);
            assert_eq!(error.source().unwrap().to_string(), expected);
        }
    }

    #[test]
    fn d0_visual_manifest_pixels_checks_all_dimensions_before_earlier_mask_binary_semantics() {
        let mut fixture = Fixture::new();
        let mut nonbinary = mask_image(1);
        nonbinary.put_pixel(0, 0, Rgba([1, 1, 1, 255]));
        fixture.replace_mask(1, "erase", &nonbinary);
        let wrong_dimensions = RgbaImage::from_pixel(3, 4, Rgba([255, 255, 255, 255]));
        fixture.replace_mask(8, "erase", &wrong_dimensions);
        let error = expect_error(fixture.validate(), DIMENSION_ERROR);
        assert_eq!(
            error.source().unwrap().to_string(),
            "mask dimensions differ from page dimensions"
        );
    }

    #[test]
    fn d0_visual_manifest_pixels_applies_jpeg_orientations_three_six_and_eight() {
        for exif in [3, 6, 8] {
            let bytes = oriented_jpeg(exif, 3, 2);
            let mut budget = SharedBudget::new(BYTE_CEILING);
            let probe = probe_asset(&bytes, &mut budget).unwrap();
            assert_eq!(probe.orientation.to_exif(), exif);

            let mut decoder = ImageReader::with_format(Cursor::new(&bytes), ImageFormat::Jpeg)
                .into_decoder()
                .unwrap();
            let orientation = decoder.orientation().unwrap();
            let decoded_without_orientation =
                DynamicImage::from_decoder(decoder).unwrap().into_rgba8();
            let expected = manual_exif_orientation(&decoded_without_orientation, exif);

            let actual = decode_retained_rgba(&bytes, probe, &mut budget).unwrap();
            assert_eq!(actual, expected);
            assert_eq!(
                canonical_decoded_rgba_blake3(&bytes).unwrap(),
                blake3_hex(&expected)
            );
            assert_eq!(
                actual.dimensions(),
                oriented_dimensions((3, 2), orientation)
            );
        }
    }

    #[test]
    fn d0_visual_manifest_pixels_accepts_full_manifest_post_orientation_hash() {
        let mut fixture = Fixture::new();
        let bytes = oriented_jpeg(6, 4, 4);
        let mut decoder = ImageReader::with_format(Cursor::new(&bytes), ImageFormat::Jpeg)
            .into_decoder()
            .unwrap();
        assert_eq!(decoder.orientation().unwrap().to_exif(), 6);
        let decoded_without_orientation = DynamicImage::from_decoder(decoder).unwrap().into_rgba8();
        let expected = manual_exif_orientation(&decoded_without_orientation, 6);
        fixture.replace_source(1, &bytes, Some(&expected));

        let validated = fixture.validate().unwrap();
        assert_eq!(validated.entries[1].source, expected);
    }

    #[test]
    fn d0_visual_manifest_pixels_checked_boundaries_do_not_allocate_ceiling() {
        assert!(checked_rgba_bytes(u64::MAX, 2, BYTE_CEILING).is_err());
        assert_eq!(
            checked_rgba_bytes(BYTE_CEILING / 4, 1, BYTE_CEILING).unwrap(),
            BYTE_CEILING
        );
        assert!(checked_aggregate_bytes([BYTE_CEILING, 1], BYTE_CEILING).is_err());
        let overflowing_probe = AssetProbe {
            format: ImageFormat::Png,
            orientation: Orientation::NoTransforms,
            raw_dimensions: (1, 1),
            oriented_dimensions: (1, 1),
            rgba_bytes: 4,
            decoder_bytes: u64::MAX,
        };
        assert!(checked_decode_peak(1, overflowing_probe, 0).is_err());
        assert!(checked_probe_temporary(u64::MAX).is_err());
        for (name, decoder_bytes) in [("Rgba16", 800), ("Rgb16", 600)] {
            let oriented_16_bit = AssetProbe {
                format: ImageFormat::Png,
                orientation: Orientation::Rotate90,
                raw_dimensions: (10, 10),
                oriented_dimensions: (10, 10),
                rgba_bytes: 400,
                decoder_bytes,
            };
            assert_eq!(
                checked_decode_peak(1, oriented_16_bit, 0).unwrap(),
                decoder_bytes * 2,
                "{name} orientation copy must be the peak"
            );
        }

        let mut budget = SharedBudget::new(10);
        with_probe_reservation(5, &mut budget, |used| {
            assert_eq!(used, 10);
            Ok(())
        })
        .unwrap();
        assert_eq!(budget.used, 0);
        assert!(
            with_probe_reservation(5, &mut budget, |_| -> D0PixelResult<()> {
                Err(failure(PROBE_ERROR, "injected probe failure"))
            })
            .is_err()
        );
        assert_eq!(budget.used, 0);
        assert!(budget.release(1).is_err());
    }

    #[test]
    fn d0_visual_manifest_pixels_real_path_releases_residual_retained_bytes() {
        let fixture = Fixture::new();
        let baseline = fixture.validate().unwrap();
        let exact_limit = baseline.peak_reserved_bytes;
        let validated =
            validate_dimensions_and_masks_with_decoded_limit(fixture.load_schema(), exact_limit)
                .unwrap();
        let retained_from_output = validated
            .entries
            .iter()
            .map(|entry| {
                entry.source.as_raw().len() as u64
                    + entry.clean_reference.as_raw().len() as u64
                    + entry
                        .targets
                        .iter()
                        .map(|target| target.agreed_mask.len() as u64)
                        .sum::<u64>()
            })
            .sum::<u64>();
        assert_eq!(validated.final_retained_bytes, retained_from_output);
        assert!(validated.peak_reserved_bytes > retained_from_output);

        let error = expect_error(
            validate_dimensions_and_masks_with_policy(fixture.load_schema(), exact_limit, false),
            BUDGET_ERROR,
        );
        assert_eq!(
            error.source().unwrap().to_string(),
            "shared decoded reservation exceeded"
        );
    }

    fn oriented_jpeg(exif_orientation: u8, width: u32, height: u32) -> Vec<u8> {
        let image = RgbImage::from_fn(width, height, |x, y| {
            Rgb([(x * 80) as u8, (y * 120) as u8, ((x + y) * 40) as u8])
        });
        let mut bytes = Vec::new();
        let mut encoder = JpegEncoder::new_with_quality(&mut bytes, 100);
        encoder
            .set_exif_metadata(exif_bytes(exif_orientation))
            .unwrap();
        encoder
            .write_image(image.as_raw(), width, height, ExtendedColorType::Rgb8)
            .unwrap();
        bytes
    }

    fn manual_exif_orientation(source: &RgbaImage, exif: u8) -> RgbaImage {
        let (width, height) = source.dimensions();
        match exif {
            3 => RgbaImage::from_fn(width, height, |x, y| {
                *source.get_pixel(width - 1 - x, height - 1 - y)
            }),
            6 => RgbaImage::from_fn(height, width, |x, y| *source.get_pixel(y, height - 1 - x)),
            8 => RgbaImage::from_fn(height, width, |x, y| *source.get_pixel(width - 1 - y, x)),
            _ => panic!("unsupported test orientation"),
        }
    }

    fn exif_bytes(orientation: u8) -> Vec<u8> {
        let mut exif = Vec::new();
        exif.extend_from_slice(b"II*\0");
        exif.extend_from_slice(&8_u32.to_le_bytes());
        exif.extend_from_slice(&1_u16.to_le_bytes());
        exif.extend_from_slice(&0x0112_u16.to_le_bytes());
        exif.extend_from_slice(&3_u16.to_le_bytes());
        exif.extend_from_slice(&1_u32.to_le_bytes());
        exif.extend_from_slice(&u16::from(orientation).to_le_bytes());
        exif.extend_from_slice(&0_u16.to_le_bytes());
        exif.extend_from_slice(&0_u32.to_le_bytes());
        exif
    }
}
