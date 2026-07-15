use std::collections::{HashMap, HashSet};

use anyhow::{Context, Result};
use camino::{Utf8Path, Utf8PathBuf};
use koharu_core::{FontFaceInfo, FontSource, GoogleFontCatalog, GoogleFontEntry};
use parking_lot::RwLock;
use tracing::debug;

const CATALOG_JSON: &str = include_str!("../data/google-fonts-catalog.json");

const RECOMMENDED_FAMILIES: &[&str] = &[
    "Noto Sans SC",
    "Noto Sans TC",
    "Noto Sans JP",
    "Noto Sans KR",
    "Noto Serif SC",
    "Roboto",
    "Inter",
    "Open Sans",
    "Montserrat",
    "Poppins",
    "ZCOOL XiaoWei",
    "Ma Shan Zheng",
];

/// On-demand Google Fonts service with persistent disk caching.
pub struct GoogleFontService {
    catalog: GoogleFontCatalog,
    cache_dir: Utf8PathBuf,
    /// Tracks which families have been downloaded to disk.
    cached_families: RwLock<HashMap<String, Vec<Utf8PathBuf>>>,
}

impl GoogleFontService {
    pub fn new(app_data_root: &Utf8Path) -> Result<Self> {
        let catalog: GoogleFontCatalog =
            serde_json::from_str(CATALOG_JSON).context("failed to parse Google Fonts catalog")?;
        let cache_dir = app_data_root.join("fonts").join("google");
        std::fs::create_dir_all(cache_dir.as_std_path())
            .context("failed to create Google Fonts cache dir")?;

        // Scan existing cache to populate known cached families
        let mut cached_families = HashMap::new();
        for entry in &catalog.fonts {
            let family_dir = cache_dir.join(normalize_family_dir(&entry.family));
            if family_dir.exists() {
                let paths: Vec<Utf8PathBuf> = entry
                    .variants
                    .iter()
                    .map(|v| family_dir.join(&v.filename))
                    .filter(|p| p.exists())
                    .collect();
                if !paths.is_empty() {
                    cached_families.insert(entry.family.clone(), paths);
                }
            }
        }

        Ok(Self {
            catalog,
            cache_dir,
            cached_families: RwLock::new(cached_families),
        })
    }

    /// Returns the full catalog for browsing.
    pub fn catalog(&self) -> &GoogleFontCatalog {
        &self.catalog
    }

    /// Returns the list of recommended font family names.
    pub fn recommended_families(&self) -> &[&str] {
        RECOMMENDED_FAMILIES
    }

    /// Checks if a family has been cached to disk.
    pub async fn is_cached(&self, family: &str) -> bool {
        self.cached_families.read().contains_key(family)
    }

    /// Returns the small default Google Fonts set for the combined `/fonts` response.
    pub fn default_faces(&self) -> Vec<FontFaceInfo> {
        let cached_families = self.cached_families.read();
        let mut seen = HashSet::new();
        let mut faces = Vec::new();

        for entry in &self.catalog.fonts {
            let recommended = if RECOMMENDED_FAMILIES.contains(&entry.family.as_str()) {
                entry
                    .variants
                    .iter()
                    .find(|variant| variant.weight == 400 && variant.style == "normal")
                    .or_else(|| entry.variants.first())
            } else {
                None
            };
            let cached_paths = cached_families.get(&entry.family);

            for variant in &entry.variants {
                let path = self
                    .cache_dir
                    .join(normalize_family_dir(&entry.family))
                    .join(&variant.filename);
                let cached = cached_paths.is_some_and(|paths| paths.contains(&path));
                let is_recommended_default = recommended.is_some_and(|default| {
                    default.weight == variant.weight
                        && default.style == variant.style
                        && default.filename == variant.filename
                });
                if !cached && !is_recommended_default {
                    continue;
                }

                let post_script_name = format!(
                    "{}:{}{}",
                    entry.family,
                    variant.weight,
                    if variant.style == "italic" { "i" } else { "" }
                );
                if seen.insert(post_script_name.clone()) {
                    faces.push(FontFaceInfo {
                        family_name: entry.family.clone(),
                        post_script_name,
                        source: FontSource::Google,
                        category: Some(entry.category.clone()),
                        cached,
                    });
                }
            }
        }

        faces
    }

    /// Downloads a font family's regular variant to disk cache.
    /// Returns the path to the cached .ttf file.
    /// No-op if already cached.
    pub async fn fetch_family(
        &self,
        family: &str,
        http: &reqwest_middleware::ClientWithMiddleware,
    ) -> Result<Utf8PathBuf> {
        self.fetch_variant(family, 400, "normal", http).await
    }

    /// Downloads a specific variant to disk cache.
    pub async fn fetch_variant(
        &self,
        family: &str,
        weight: u16,
        style: &str,
        http: &reqwest_middleware::ClientWithMiddleware,
    ) -> Result<Utf8PathBuf> {
        let entry = self
            .catalog
            .fonts
            .iter()
            .find(|e| e.family == family)
            .with_context(|| format!("font family not found in catalog: {family}"))?;

        let variant = entry
            .variants
            .iter()
            .find(|v| v.weight == weight && v.style == style)
            .or_else(|| {
                // Fallback to regular if requested variant not found
                entry
                    .variants
                    .iter()
                    .find(|v| v.weight == 400 && v.style == "normal")
            })
            .or_else(|| entry.variants.first())
            .context("font has no variants")?;

        let family_dir_name = normalize_family_dir(&entry.family);
        let file_path = self
            .cache_dir
            .join(&family_dir_name)
            .join(&variant.filename);

        // Check cache first
        if file_path.exists() {
            let mut cached = self.cached_families.write();
            let entries = cached.entry(family.to_string()).or_default();
            if !entries.contains(&file_path) {
                entries.push(file_path.clone());
            }
            return Ok(file_path);
        }

        // Try different license categories on Google Fonts GitHub
        let categories = ["ofl", "apache", "ufl"];
        let mut last_error = None;

        for category in categories {
            let url = format!(
                "https://raw.githubusercontent.com/google/fonts/main/{}/{}/{}",
                category, family_dir_name, variant.filename
            );

            debug!(%family, %url, "trying to download Google Font");
            match http.get(&url).send().await {
                Ok(resp) if resp.status().is_success() => {
                    let bytes = resp.bytes().await.context("failed to read font bytes")?;
                    std::fs::create_dir_all(file_path.parent().unwrap())?;
                    std::fs::write(&file_path, &bytes)?;

                    // Update in-memory cache tracking
                    let mut cached = self.cached_families.write();
                    let entries = cached.entry(family.to_string()).or_default();
                    if !entries.contains(&file_path) {
                        entries.push(file_path.clone());
                    }

                    return Ok(file_path);
                }
                Ok(resp) if resp.status() == 404 => {
                    // If exact filename failed, it might be a naming mismatch on the CDN
                    // This is rare for the main repo but happens with some older fonts
                    last_error = Some(anyhow::anyhow!(
                        "Font file {} not found in {}",
                        variant.filename,
                        category
                    ));
                    continue;
                }
                Ok(resp) => {
                    last_error = Some(anyhow::anyhow!("CDN returned {}", resp.status()));
                }
                Err(e) => {
                    last_error = Some(e.into());
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("Font not found in any known category")))
    }

    /// Reads the cached font file bytes. Returns None if not cached.
    pub fn read_cached_file(&self, family: &str) -> Result<Option<Vec<u8>>> {
        self.read_cached_variant(family, 400, "normal")
    }

    /// Reads a specific cached variant.
    pub fn read_cached_variant(
        &self,
        family: &str,
        weight: u16,
        style: &str,
    ) -> Result<Option<Vec<u8>>> {
        let entry = self.catalog.fonts.iter().find(|e| e.family == family);
        let Some(entry) = entry else {
            return Ok(None);
        };
        let variant = entry
            .variants
            .iter()
            .find(|v| v.weight == weight && v.style == style);

        let Some(variant) = variant else {
            // If the specific variant isn't in the catalog, we can't load it
            return Ok(None);
        };
        let file_path = self
            .cache_dir
            .join(normalize_family_dir(&entry.family))
            .join(&variant.filename);
        if !file_path.exists() {
            return Ok(None);
        }
        let data = std::fs::read(file_path.as_std_path()).context("failed to read cached font")?;
        Ok(Some(data))
    }

    /// Find catalog entry by family name.
    pub fn find_entry(&self, family: &str) -> Option<&GoogleFontEntry> {
        self.catalog.fonts.iter().find(|e| e.family == family)
    }
}

/// Converts family name to directory name (lowercase, spaces to empty).
/// e.g. "Comic Neue" -> "comicneue"
fn normalize_family_dir(family: &str) -> String {
    family.to_lowercase().replace(' ', "")
}

/// Parses a variant query string like "Family:700i" into (family, weight, style).
pub fn parse_variant_query(query: &str) -> (&str, u16, &str) {
    if let Some((family, variant_str)) = query.split_once(':') {
        let weight = variant_str
            .chars()
            .filter(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse::<u16>()
            .unwrap_or(400);
        let style = if variant_str.contains('i') {
            "italic"
        } else {
            "normal"
        };
        (family, weight, style)
    } else {
        (query, 400, "normal")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_faces_include_only_recommended_and_cached_variants() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = Utf8Path::from_path(temp.path()).expect("temp path should be UTF-8");
        let cached_path = root
            .join("fonts/google/abeezee")
            .join("ABeeZee-Regular.ttf");
        std::fs::create_dir_all(
            cached_path
                .parent()
                .expect("cached font should have a parent"),
        )?;
        std::fs::write(&cached_path, b"cached")?;

        let service = GoogleFontService::new(root)?;
        let faces = service.default_faces();

        assert!(!faces.iter().any(|face| face.family_name == "Abel"));
        let cached = faces
            .iter()
            .find(|face| face.post_script_name == "ABeeZee:400")
            .expect("cached non-recommended variant should be included");
        assert!(cached.cached);

        for family in RECOMMENDED_FAMILIES {
            let family_faces = faces
                .iter()
                .filter(|face| face.family_name == *family)
                .collect::<Vec<_>>();
            assert_eq!(
                family_faces.len(),
                1,
                "{family} should expose one default face"
            );

            let entry = service
                .find_entry(family)
                .unwrap_or_else(|| panic!("recommended family missing from catalog: {family}"));
            let expected = entry
                .variants
                .iter()
                .find(|variant| variant.weight == 400 && variant.style == "normal")
                .or_else(|| entry.variants.first())
                .expect("recommended family should have a variant");
            assert_eq!(
                family_faces[0].post_script_name,
                format!(
                    "{}:{}{}",
                    entry.family,
                    expected.weight,
                    if expected.style == "italic" { "i" } else { "" }
                )
            );
            assert!(!family_faces[0].cached);
        }

        let mut post_script_names = faces
            .iter()
            .map(|face| face.post_script_name.as_str())
            .collect::<Vec<_>>();
        post_script_names.sort_unstable();
        post_script_names.dedup();
        assert_eq!(post_script_names.len(), faces.len());
        Ok(())
    }

    #[tokio::test]
    async fn fetch_variant_existing_file_updates_cache_index() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let root = Utf8Path::from_path(temp.path()).expect("temp path should be UTF-8");
        let service = GoogleFontService::new(root)?;
        let entry = service
            .find_entry("ABeeZee")
            .expect("ABeeZee should exist in the catalog");
        let variant = entry
            .variants
            .iter()
            .find(|variant| variant.weight == 400 && variant.style == "normal")
            .expect("ABeeZee regular should exist");
        let cached_path = root.join("fonts/google/abeezee").join(&variant.filename);
        std::fs::create_dir_all(
            cached_path
                .parent()
                .expect("cached font should have a parent"),
        )?;
        std::fs::write(&cached_path, b"cached")?;

        let http = reqwest_middleware::ClientBuilder::new(
            reqwest_middleware::reqwest::Client::builder()
                .proxy(reqwest_middleware::reqwest::Proxy::all(
                    "http://127.0.0.1:9",
                )?)
                .build()?,
        )
        .build();

        assert_eq!(
            service
                .fetch_variant("ABeeZee", 400, "normal", &http)
                .await?,
            cached_path
        );
        assert!(
            service
                .default_faces()
                .iter()
                .any(|face| face.post_script_name == "ABeeZee:400" && face.cached)
        );
        Ok(())
    }
}
