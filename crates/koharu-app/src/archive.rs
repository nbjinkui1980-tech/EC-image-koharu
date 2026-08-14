//! `.khr` archive = zip of `.khrproj/` minus `cache/` and `.lock`.
//!
//! Blobs are already compressed (webp/jpg/webp-sprite), so they go in as
//! `Stored`. Text/metadata files (`project.toml`, `scene.bin`, `history.log`)
//! use `Deflated`.

use std::fs::File;
use std::io::{Cursor, Read, Seek, Write};

use anyhow::{Context, Result};
use atomicwrites::{AtomicFile, OverwriteBehavior};
use camino::{Utf8Path, Utf8PathBuf};
use walkdir::WalkDir;
use zip::{CompressionMethod, ZipArchive, ZipWriter, write::SimpleFileOptions};

const SKIP_DIRS: &[&str] = &["cache", ".lock"];

/// Approved `.khr` extraction read budgets (AR05-T02 decision point).
#[derive(Debug, Clone, Copy)]
pub(crate) struct ArchiveBudgets {
    pub max_entries: u32,
    pub max_entry_bytes: u64,
    pub max_total_bytes: u64,
    pub max_ratio: u64,
    pub ratio_floor_bytes: u64,
}

pub(crate) const DEFAULT_ARCHIVE_BUDGETS: ArchiveBudgets = ArchiveBudgets {
    max_entries: 10_000,
    max_entry_bytes: 256 * 1024 * 1024,
    max_total_bytes: 4 * 1024 * 1024 * 1024,
    max_ratio: 100,
    ratio_floor_bytes: 1024 * 1024,
};

/// Pack `project_dir` (`.khrproj/`) into `out_khr` as a `.khr` archive.
pub fn export_khr(project_dir: &Utf8Path, out_khr: &Utf8Path) -> Result<()> {
    let project_dir_std = project_dir.as_std_path().to_path_buf();
    let out_std = out_khr.as_std_path().to_path_buf();

    AtomicFile::new(out_std, OverwriteBehavior::AllowOverwrite)
        .write(move |f| -> Result<()> {
            write_khr_zip(&project_dir_std, f)?;
            Ok(())
        })
        .map_err(|e| match e {
            atomicwrites::Error::Internal(io) => anyhow::Error::new(io),
            atomicwrites::Error::User(e) => e,
        })?;
    Ok(())
}

/// Pack `project_dir` into an in-memory `.khr` zip. Used by the HTTP export
/// route that streams bytes to the client instead of writing to disk.
pub fn export_khr_bytes(project_dir: &Utf8Path) -> Result<Vec<u8>> {
    let project_dir_std = project_dir.as_std_path().to_path_buf();
    let mut cursor = Cursor::new(Vec::new());
    write_khr_zip(&project_dir_std, &mut cursor)?;
    Ok(cursor.into_inner())
}

fn write_khr_zip<W: Write + Seek>(project_dir_std: &std::path::Path, w: W) -> Result<()> {
    let mut zip = ZipWriter::new(w);
    for entry in WalkDir::new(project_dir_std)
        .follow_links(false)
        .into_iter()
        .filter_map(|e| e.ok())
    {
        let path = entry.path();
        if path == project_dir_std {
            continue;
        }
        let rel = path
            .strip_prefix(project_dir_std)
            .expect("walkdir starts at root")
            .to_path_buf();
        if should_skip(&rel) {
            continue;
        }
        let rel_str = rel
            .components()
            .map(|c| c.as_os_str().to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join("/");
        if entry.file_type().is_dir() {
            zip.add_directory(&rel_str, SimpleFileOptions::default())?;
            continue;
        }
        let method = if rel_str.starts_with("blobs/") {
            CompressionMethod::Stored
        } else {
            CompressionMethod::Deflated
        };
        zip.start_file(
            &rel_str,
            SimpleFileOptions::default().compression_method(method),
        )?;
        let mut src = File::open(path)?;
        std::io::copy(&mut src, &mut zip)?;
    }
    zip.finish()?;
    Ok(())
}

/// Read bytes of a `.khr` archive and extract into `project_dir`. Symmetrical
/// with `export_khr_bytes`: used by the HTTP `/projects/import` route.
pub fn import_khr_bytes(bytes: &[u8], project_dir: &Utf8Path) -> Result<Utf8PathBuf> {
    if project_dir.exists() {
        anyhow::bail!("destination already exists: {project_dir}");
    }
    std::fs::create_dir_all(project_dir.as_std_path())?;
    extract_khr_bytes(bytes, project_dir)?;
    Ok(project_dir.to_path_buf())
}

/// Extract into an already-reserved staging directory without deleting or
/// recreating it. The directory must exist and be empty.
pub fn import_khr_bytes_into_empty_staging(
    bytes: &[u8],
    staging_dir: &Utf8Path,
) -> Result<Utf8PathBuf> {
    if !staging_dir.is_dir() {
        anyhow::bail!("staging directory does not exist: {staging_dir}");
    }
    if std::fs::read_dir(staging_dir.as_std_path())?
        .next()
        .is_some()
    {
        anyhow::bail!("staging directory is not empty: {staging_dir}");
    }
    extract_khr_bytes(bytes, staging_dir)?;
    Ok(staging_dir.to_path_buf())
}

fn extract_khr_bytes(bytes: &[u8], project_dir: &Utf8Path) -> Result<()> {
    extract_khr_bytes_with_budgets(bytes, project_dir, &DEFAULT_ARCHIVE_BUDGETS)
}

fn extract_khr_bytes_with_budgets(
    bytes: &[u8],
    project_dir: &Utf8Path,
    budgets: &ArchiveBudgets,
) -> Result<()> {
    let mut archive = ZipArchive::new(Cursor::new(bytes)).context("open zip archive")?;
    anyhow::ensure!(
        archive.len() as u64 <= u64::from(budgets.max_entries),
        "archive entry count {} exceeds budget {}",
        archive.len(),
        budgets.max_entries
    );
    let mut total_bytes = 0u64;
    for i in 0..archive.len() {
        let mut entry = archive.by_index(i)?;
        let Some(enclosed) = entry.enclosed_name() else {
            continue;
        };
        let rel = Utf8PathBuf::from_path_buf(enclosed.to_path_buf())
            .map_err(|p| anyhow::anyhow!("archive entry not UTF-8: {}", p.display()))?;
        let target = project_dir.join(&rel);
        if entry.is_dir() {
            std::fs::create_dir_all(target.as_std_path())?;
            continue;
        }
        let declared = entry.size();
        anyhow::ensure!(
            declared <= budgets.max_entry_bytes,
            "archive entry {rel} declares {declared} bytes, exceeding per-entry budget {}",
            budgets.max_entry_bytes
        );
        let compressed = entry.compressed_size();
        if let Some(parent) = target.parent() {
            std::fs::create_dir_all(parent.as_std_path())?;
        }
        let mut out =
            File::create(target.as_std_path()).with_context(|| format!("create {target}"))?;
        let mut entry_bytes = 0u64;
        let mut limited = entry.by_ref().take(budgets.max_entry_bytes + 1);
        let mut buf = [0u8; 64 * 1024];
        loop {
            let read = limited.read(&mut buf)?;
            if read == 0 {
                break;
            }
            entry_bytes += read as u64;
            anyhow::ensure!(
                entry_bytes <= budgets.max_entry_bytes,
                "archive entry {rel} expands beyond per-entry budget {}",
                budgets.max_entry_bytes
            );
            total_bytes += read as u64;
            anyhow::ensure!(
                total_bytes <= budgets.max_total_bytes,
                "archive expands beyond total budget {}",
                budgets.max_total_bytes
            );
            out.write_all(&buf[..read])?;
        }
        if entry_bytes > budgets.ratio_floor_bytes {
            let allowed = compressed
                .saturating_mul(budgets.max_ratio)
                .max(budgets.ratio_floor_bytes);
            anyhow::ensure!(
                entry_bytes <= allowed,
                "archive entry {rel} compression ratio exceeds {}:1",
                budgets.max_ratio
            );
        }
    }
    Ok(())
}

/// Unpack `khr_path` into `project_dir`. `project_dir` must not exist yet.
pub fn import_khr(khr_path: &Utf8Path, project_dir: &Utf8Path) -> Result<Utf8PathBuf> {
    let bytes = std::fs::read(khr_path.as_std_path())
        .with_context(|| format!("read archive {khr_path}"))?;
    import_khr_bytes(&bytes, project_dir)
}

/// Pack `(filename, bytes)` pairs into a `Deflated` zip in memory. Used by
/// the HTTP export route when a format produces multiple files (per-page PSD,
/// per-page PNG). Filenames are used verbatim — caller decides structure.
pub fn zip_files_to_bytes(files: &[(String, Vec<u8>)]) -> Result<Vec<u8>> {
    let mut cursor = Cursor::new(Vec::new());
    {
        let mut zip = ZipWriter::new(&mut cursor);
        for (name, bytes) in files {
            zip.start_file(
                name,
                SimpleFileOptions::default().compression_method(CompressionMethod::Deflated),
            )?;
            zip.write_all(bytes)?;
        }
        zip.finish()?;
    }
    Ok(cursor.into_inner())
}

fn should_skip(rel: &std::path::Path) -> bool {
    rel.components()
        .any(|c| SKIP_DIRS.contains(&c.as_os_str().to_string_lossy().as_ref()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use camino::Utf8PathBuf;
    use tempfile::tempdir;

    #[test]
    fn export_then_import_round_trips_files() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();

        // Build a fake project.
        let proj = root.join("proj.khrproj");
        std::fs::create_dir_all(proj.join("blobs/ab").as_std_path()).unwrap();
        std::fs::create_dir_all(proj.join("cache").as_std_path()).unwrap();
        std::fs::write(proj.join("project.toml").as_std_path(), b"name = \"x\"\n").unwrap();
        std::fs::write(proj.join("blobs/ab/cdef").as_std_path(), b"blob bytes").unwrap();
        std::fs::write(proj.join("cache/thumb.webp").as_std_path(), b"cached").unwrap();

        let khr = root.join("out.khr");
        export_khr(&proj, &khr).unwrap();

        let restored = root.join("restored.khrproj");
        import_khr(&khr, &restored).unwrap();
        assert!(restored.join("project.toml").exists());
        assert!(restored.join("blobs/ab/cdef").exists());
        assert!(
            !restored.join("cache/thumb.webp").exists(),
            "cache excluded"
        );
    }

    #[test]
    fn archive_extracts_into_reserved_empty_staging_and_rejects_non_empty_staging() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let source = root.join("source.khrproj");
        std::fs::create_dir_all(source.as_std_path()).unwrap();
        std::fs::write(source.join("project.toml").as_std_path(), b"name = \"x\"\n").unwrap();
        let bytes = export_khr_bytes(&source).unwrap();

        let staging = root.join(".import-test.staging");
        std::fs::create_dir(staging.as_std_path()).unwrap();
        import_khr_bytes_into_empty_staging(&bytes, &staging).unwrap();
        assert!(staging.join("project.toml").exists());

        let non_empty = root.join(".import-non-empty.staging");
        std::fs::create_dir(non_empty.as_std_path()).unwrap();
        std::fs::write(non_empty.join("sentinel").as_std_path(), b"keep").unwrap();
        let error = import_khr_bytes_into_empty_staging(&bytes, &non_empty).unwrap_err();
        assert!(error.to_string().contains("not empty"));
        assert_eq!(std::fs::read(non_empty.join("sentinel")).unwrap(), b"keep");
        assert!(!non_empty.join("project.toml").exists());
    }

    fn zip_of(entries: &[(&str, &[u8])], method: CompressionMethod) -> Vec<u8> {
        let mut cursor = Cursor::new(Vec::new());
        {
            let mut zip = ZipWriter::new(&mut cursor);
            for (name, data) in entries {
                zip.start_file(
                    name,
                    SimpleFileOptions::default().compression_method(method),
                )
                .unwrap();
                zip.write_all(data).unwrap();
            }
            zip.finish().unwrap();
        }
        cursor.into_inner()
    }

    fn patch_central_declared_size(mut bytes: Vec<u8>, declared: u32) -> Vec<u8> {
        let sig = b"PK\x01\x02";
        let pos = bytes
            .windows(4)
            .position(|w| w == sig)
            .expect("central directory");
        bytes[pos + 24..pos + 28].copy_from_slice(&declared.to_le_bytes());
        bytes
    }

    fn staging_dir(root: &Utf8Path) -> Utf8PathBuf {
        let dir = root.join("budget.khrproj");
        std::fs::create_dir(dir.as_std_path()).unwrap();
        dir
    }

    #[test]
    fn archive_rejects_entry_count_above_budget() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let proj = staging_dir(&root);
        let bytes = zip_of(
            &[
                ("a", b"1".as_slice()),
                ("b", b"2".as_slice()),
                ("c", b"3".as_slice()),
                ("d", b"4".as_slice()),
            ],
            CompressionMethod::Stored,
        );
        let budgets = ArchiveBudgets {
            max_entries: 3,
            ..DEFAULT_ARCHIVE_BUDGETS
        };
        let error = extract_khr_bytes_with_budgets(&bytes, &proj, &budgets).unwrap_err();
        assert!(error.to_string().contains("entry count"), "{error}");
    }

    #[test]
    fn archive_rejects_forged_entry_size_above_budget_before_allocating() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let proj = staging_dir(&root);
        let bytes = zip_of(
            &[("project.toml", b"name = \"x\"\n".as_slice())],
            CompressionMethod::Stored,
        );
        let forged = patch_central_declared_size(bytes, 2048);
        let budgets = ArchiveBudgets {
            max_entry_bytes: 1024,
            ..DEFAULT_ARCHIVE_BUDGETS
        };
        let error = extract_khr_bytes_with_budgets(&forged, &proj, &budgets).unwrap_err();
        assert!(error.to_string().contains("per-entry budget"), "{error}");
    }

    #[test]
    fn archive_rejects_single_entry_bytes_above_budget() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let proj = staging_dir(&root);
        let blob = vec![7u8; 2048];
        let bytes = zip_of(
            &[("blobs/ab/big.webp", blob.as_slice())],
            CompressionMethod::Stored,
        );
        let budgets = ArchiveBudgets {
            max_entry_bytes: 1024,
            ..DEFAULT_ARCHIVE_BUDGETS
        };
        let error = extract_khr_bytes_with_budgets(&bytes, &proj, &budgets).unwrap_err();
        assert!(error.to_string().contains("per-entry budget"), "{error}");
    }

    #[test]
    fn archive_rejects_total_expanded_bytes_above_budget() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let proj = staging_dir(&root);
        let blob = vec![9u8; 3072];
        let bytes = zip_of(
            &[
                ("blobs/01", blob.as_slice()),
                ("blobs/02", blob.as_slice()),
                ("blobs/03", blob.as_slice()),
            ],
            CompressionMethod::Stored,
        );
        let budgets = ArchiveBudgets {
            max_entry_bytes: 4096,
            max_total_bytes: 6 * 1024,
            ..DEFAULT_ARCHIVE_BUDGETS
        };
        let error = extract_khr_bytes_with_budgets(&bytes, &proj, &budgets).unwrap_err();
        assert!(error.to_string().contains("total budget"), "{error}");
    }

    #[test]
    fn archive_rejects_compression_ratio_above_100_to_1() {
        let tmp = tempdir().unwrap();
        let root = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let proj = staging_dir(&root);
        let zeros = vec![0u8; 8192];
        let bytes = zip_of(
            &[("history.log", zeros.as_slice())],
            CompressionMethod::Deflated,
        );
        let budgets = ArchiveBudgets {
            max_ratio: 100,
            ratio_floor_bytes: 1024,
            ..DEFAULT_ARCHIVE_BUDGETS
        };
        let error = extract_khr_bytes_with_budgets(&bytes, &proj, &budgets).unwrap_err();
        assert!(error.to_string().contains("ratio"), "{error}");
    }
}
