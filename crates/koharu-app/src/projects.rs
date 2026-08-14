//! Managed-projects directory under `{data.path}/projects/`.
//!
//! Every `.khrproj/` lives here. Clients address projects by `id` (the
//! directory basename without the `.khrproj` extension). No path handling on
//! the client side — all operations (create, open, list, import) resolve
//! paths through these helpers.
//!
//! Thread-safety: directory allocation uses atomic `create_dir` so concurrent
//! clients never collide on the same name.

use std::fs;
use std::time::UNIX_EPOCH;

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use koharu_core::ProjectSummary;
use uuid::Uuid;

use crate::AppConfig;

pub const PROJECT_EXT: &str = "khrproj";

/// The managed projects directory. Created on first call.
pub fn projects_dir(config: &AppConfig) -> Result<Utf8PathBuf> {
    let root = config.data.path.join("projects");
    fs::create_dir_all(root.as_std_path())
        .with_context(|| format!("create projects root {root}"))?;
    Ok(root)
}

/// Resolve an `id` (directory basename, no extension) to its absolute path.
/// Only exact canonical ids are accepted: anything slugify would alter
/// (symbols, case, whitespace, encoded separators) is rejected so a crafted
/// id can never resolve onto a different project.
pub fn project_path(config: &AppConfig, id: &str) -> Result<Utf8PathBuf> {
    if id.is_empty() || slugify(id) != id {
        anyhow::bail!("invalid project id: {id}");
    }
    Ok(projects_dir(config)?.join(format!("{id}.{PROJECT_EXT}")))
}

/// Pick a fresh `{projects_dir}/{slug}.khrproj` path for a new project with
/// the given display name. Sanitises the name, retries with `-2`, `-3`, … on
/// collision. Creates the directory atomically so concurrent callers never
/// land on the same path.
pub fn allocate_named(config: &AppConfig, name: &str) -> Result<Utf8PathBuf> {
    let root = projects_dir(config)?;
    let base = {
        let s = slugify(name);
        if s.is_empty() {
            "untitled".to_string()
        } else {
            s
        }
    };
    for attempt in 0..1024 {
        let filename = if attempt == 0 {
            format!("{base}.{PROJECT_EXT}")
        } else {
            format!("{base}-{attempt}.{PROJECT_EXT}")
        };
        let candidate = root.join(&filename);
        match fs::create_dir(candidate.as_std_path()) {
            Ok(()) => return Ok(candidate),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(anyhow::Error::new(e).context(format!("create {candidate}"))),
        }
    }
    anyhow::bail!("could not allocate a fresh project directory (1024 collisions)")
}

/// Pick a fresh path for an imported archive. Uses the archive-provided name
/// as the base when it's non-empty; otherwise `imported`.
pub fn allocate_imported(
    config: &AppConfig,
    name_hint: Option<&str>,
) -> Result<(Utf8PathBuf, Utf8PathBuf)> {
    let root = projects_dir(config)?;
    let base = match name_hint.map(slugify).filter(|name| !name.is_empty()) {
        Some(name) => name,
        None => "imported".to_string(),
    };
    loop {
        let id = Uuid::new_v4().simple();
        let staging = root.join(format!(".import-{id}.staging"));
        let final_path = root.join(format!("{base}-{id}.{PROJECT_EXT}"));
        if final_path.exists() {
            continue;
        }
        match fs::create_dir(staging.as_std_path()) {
            Ok(()) => return Ok((staging, final_path)),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(error) => {
                return Err(anyhow::Error::new(error).context(format!("create {staging}")));
            }
        }
    }
}

/// List every `.khrproj/` directory under the managed projects root. Reads
/// `project.toml` to derive the display name; falls back to the directory
/// slug if the file can't be read. Sorted by `updated_at_ms` descending.
pub fn list_projects(config: &AppConfig) -> Result<Vec<ProjectSummary>> {
    let root = projects_dir(config)?;
    let mut out: Vec<ProjectSummary> = Vec::new();
    let entries = match fs::read_dir(root.as_std_path()) {
        Ok(it) => it,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(out),
        Err(e) => return Err(anyhow::Error::new(e)),
    };
    for entry in entries.flatten() {
        let ftype = match entry.file_type() {
            Ok(t) => t,
            Err(_) => continue,
        };
        if !ftype.is_dir() {
            continue;
        }
        let name_os = entry.file_name();
        let Some(filename) = name_os.to_str() else {
            continue;
        };
        let Some(id) = filename.strip_suffix(&format!(".{PROJECT_EXT}")) else {
            continue;
        };
        let abs = root.join(filename);
        let display = read_project_name(&abs).unwrap_or_else(|| id.to_string());
        let updated_at_ms = entry
            .metadata()
            .ok()
            .and_then(|m| m.modified().ok())
            .and_then(|m| m.duration_since(UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        out.push(ProjectSummary {
            id: id.to_string(),
            name: display,
            path: abs.to_string(),
            updated_at_ms,
        });
    }
    out.sort_by_key(|project| std::cmp::Reverse(project.updated_at_ms));
    Ok(out)
}

/// Produce an id for a `.khrproj/` directory basename.
pub fn id_from_dir(dir: &Utf8Path) -> Option<String> {
    dir.file_name()
        .and_then(|n| n.strip_suffix(&format!(".{PROJECT_EXT}")))
        .map(|s| s.to_string())
}

// ---------------------------------------------------------------------------
// Internals
// ---------------------------------------------------------------------------

fn read_project_name(dir: &Utf8Path) -> Option<String> {
    let toml_path = dir.join("project.toml");
    let text = fs::read_to_string(toml_path.as_std_path()).ok()?;
    // Minimal parse: look for `name = "..."` on a line. Avoids adding a toml
    // dependency here; the real config is parsed inside the session.
    for line in text.lines() {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("name") {
            let rest = rest.trim_start();
            let Some(rest) = rest.strip_prefix('=') else {
                continue;
            };
            let rest = rest.trim();
            let name = rest.trim_matches(|c| c == '"' || c == '\'').to_string();
            if !name.is_empty() {
                return Some(name);
            }
        }
    }
    None
}

/// Lowercase + keep ASCII alphanumerics + `-` + `_`; collapse whitespace to
/// `-`. Keeps the result filesystem-safe across Win/Mac/Linux without needing
/// heavier slug libraries.
fn slugify(input: &str) -> String {
    let mut out = String::with_capacity(input.len());
    let mut prev_dash = false;
    for ch in input.chars() {
        let c = ch.to_ascii_lowercase();
        if c.is_ascii_alphanumeric() {
            out.push(c);
            prev_dash = false;
        } else if c == '-' || c == '_' {
            if !out.is_empty() && !prev_dash {
                out.push('-');
                prev_dash = true;
            }
        } else if c.is_whitespace() && !out.is_empty() && !prev_dash {
            out.push('-');
            prev_dash = true;
        }
        // Other chars dropped silently.
    }
    while out.ends_with('-') {
        out.pop();
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("My Project"), "my-project");
        assert_eq!(slugify("  leading and trailing  "), "leading-and-trailing");
        assert_eq!(slugify("under_score_already"), "under-score-already");
        assert_eq!(slugify("你好 hello"), "hello");
        assert_eq!(slugify("--dashes--"), "dashes");
    }

    // AR13-T02 RED: open/delete must accept only exact canonical ids; any
    // input that slugify would alter (symbols, case, whitespace, encoded
    // separators) must be rejected instead of mapped onto a real project.
    #[test]
    fn project_path_rejects_non_canonical_id() {
        let tmp = tempdir().unwrap();
        let mut config = AppConfig::default();
        config.data.path = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        for id in [
            "my-project!",
            "My-Project",
            "my project",
            "my%2Fproject",
            "",
        ] {
            assert!(
                project_path(&config, id).is_err(),
                "non-canonical id must be rejected: {id:?}"
            );
        }
    }

    // Lock: an exact canonical id still resolves.
    #[test]
    fn project_path_accepts_exact_canonical_id() {
        let tmp = tempdir().unwrap();
        let mut config = AppConfig::default();
        config.data.path = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();
        let path = project_path(&config, "my-project").expect("canonical id resolves");
        assert!(path.as_str().ends_with("my-project.khrproj"));
    }

    #[test]
    fn incomplete_import_staging_is_not_listed_or_openable_as_managed_project() {
        let tmp = tempdir().unwrap();
        let mut config = AppConfig::default();
        config.data.path = Utf8PathBuf::from_path_buf(tmp.path().to_path_buf()).unwrap();

        let (staging, final_path) = allocate_imported(&config, Some("imported")).unwrap();
        std::fs::write(staging.join("partial").as_std_path(), b"partial").unwrap();

        assert!(staging.exists());
        assert!(!final_path.exists());
        assert!(id_from_dir(&staging).is_none());
        assert!(list_projects(&config).unwrap().is_empty());

        let staging_name = staging.file_name().unwrap();
        // Exact-match contract: a staging dir name is not a canonical id, so
        // it is rejected outright (stronger than the old slugify remap).
        assert!(project_path(&config, staging_name).is_err());
    }
}
