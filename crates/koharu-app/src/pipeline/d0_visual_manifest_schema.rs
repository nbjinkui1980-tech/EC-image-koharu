//! Test-only D0 visual-manifest schema and immutable held-asset loader.
//!
//! This slice intentionally stops before image decoding, dimensions, ROI
//! relationships, and mask pixel semantics.

use std::collections::HashSet;
use std::error::Error;
use std::fmt;
use std::io;
use std::path::Path;

use serde::Deserialize;

use super::d0_held_input::HeldInput;
use super::d0_revision_46_contract::BYTE_CEILING;

const VISUAL_MANIFEST_VERSION: u8 = 1;
const VISUAL_MANIFEST_ENTRY_COUNT: usize = 9;
const MANIFEST_PATH_ERROR: &str = "d0.visual_manifest.path";
const MANIFEST_HASH_ERROR: &str = "d0.visual_manifest.hash";
const MANIFEST_SCHEMA_ERROR: &str = "d0.visual_manifest.schema";
const MANIFEST_VALIDATION_ERROR: &str = "d0.visual_manifest.validation";
const ASSET_PATH_ERROR: &str = "d0.visual_asset.path";
const ASSET_HASH_ERROR: &str = "d0.visual_asset.hash";

type D0Result<T> = Result<T, D0ManifestError>;

#[derive(Debug)]
pub(super) struct D0ManifestError {
    category: &'static str,
    source: Box<dyn Error + Send + Sync>,
}

impl D0ManifestError {
    pub(super) fn category(&self) -> &'static str {
        self.category
    }
}

impl fmt::Display for D0ManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.category, self.source)
    }
}

impl Error for D0ManifestError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct VisualManifestSchema {
    pub(super) version: u8,
    pub(super) entries: Vec<VisualManifestEntry>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct VisualManifestEntry {
    pub(super) id: String,
    pub(super) path: String,
    pub(super) sha256: String,
    pub(super) decoded_rgba_blake3: String,
    pub(super) clean_reference_path: String,
    pub(super) clean_reference_sha256: String,
    pub(super) clean_reference_decoded_rgba_blake3: String,
    pub(super) role: EntryRole,
    pub(super) dimension_bin: DimensionBin,
    pub(super) aspect: Aspect,
    pub(super) background: Background,
    pub(super) targets: Vec<VisualManifestTarget>,
    pub(super) protected_rois: Vec<[u64; 4]>,
    pub(super) multi_node: bool,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct VisualManifestTarget {
    pub(super) id: String,
    pub(super) source_roi: [u64; 4],
    pub(super) clean_reference_edit_roi: [u64; 4],
    pub(super) erase_source_ink_mask_path: String,
    pub(super) erase_source_ink_mask_sha256: String,
    pub(super) residual_source_ink_mask_path: String,
    pub(super) residual_source_ink_mask_sha256: String,
    pub(super) position: Position,
    pub(super) writing: Writing,
    pub(super) effect: Effect,
    pub(super) translation_length: TranslationLength,
    pub(super) expected: Expected,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(super) enum EntryRole {
    Regression,
    Calibration,
    Holdout,
}

#[derive(Debug, Deserialize)]
pub(super) enum DimensionBin {
    #[serde(rename = "lt720")]
    Lt720,
    #[serde(rename = "720_1439")]
    From720To1439,
    #[serde(rename = "1440_2159")]
    From1440To2159,
    #[serde(rename = "gte2160")]
    Gte2160,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Aspect {
    Portrait,
    Landscape,
    SquareOrNear,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Background {
    Pure,
    Gradient,
    Texture,
    Product,
    Person,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Position {
    Interior,
    PageEdge,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum Writing {
    Horizontal,
    Vertical,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub(super) enum Effect {
    Plain,
    Stroke,
    Shadow,
    Glow,
    Decorative,
}

#[derive(Debug, Deserialize)]
pub(super) enum TranslationLength {
    #[serde(rename = "short")]
    Short,
    #[serde(rename = "equal")]
    Equal,
    #[serde(rename = "2x")]
    TwoX,
    #[serde(rename = "3x")]
    ThreeX,
}

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub(super) enum Expected {
    AutomaticStrict,
    ManualOverride,
    UnsupportedSourceColor,
    UnsupportedRotation,
}

pub(super) struct HeldVisualManifestSchema {
    pub(super) schema: VisualManifestSchema,
    pub(super) manifest: HeldInput,
    pub(super) entries: Vec<HeldVisualManifestEntry>,
}

pub(super) struct HeldVisualManifestEntry {
    pub(super) source: HeldInput,
    pub(super) clean_reference: HeldInput,
    pub(super) targets: Vec<HeldVisualManifestTarget>,
}

pub(super) struct HeldVisualManifestTarget {
    pub(super) erase_source_ink_mask: HeldInput,
    pub(super) residual_source_ink_mask: HeldInput,
}

impl HeldVisualManifestSchema {
    pub(super) fn with_revalidated_paths<T>(
        &self,
        success: impl FnOnce() -> io::Result<T>,
    ) -> io::Result<T> {
        self.manifest.with_revalidated_path(|manifest| {
            manifest.with_current_namespace(|| revalidate_entries(&self.entries, success))
        })
    }
}

fn revalidate_entries<T>(
    entries: &[HeldVisualManifestEntry],
    success: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    let Some((entry, remaining)) = entries.split_first() else {
        return success();
    };
    entry.source.with_revalidated_path(|source| {
        source.with_current_namespace(|| {
            entry.clean_reference.with_revalidated_path(|clean| {
                clean.with_current_namespace(|| {
                    revalidate_targets(&entry.targets, || revalidate_entries(remaining, success))
                })
            })
        })
    })
}

fn revalidate_targets<T>(
    targets: &[HeldVisualManifestTarget],
    success: impl FnOnce() -> io::Result<T>,
) -> io::Result<T> {
    let Some((target, remaining)) = targets.split_first() else {
        return success();
    };
    target.erase_source_ink_mask.with_revalidated_path(|erase| {
        erase.with_current_namespace(|| {
            target
                .residual_source_ink_mask
                .with_revalidated_path(|residual| {
                    residual.with_current_namespace(|| revalidate_targets(remaining, success))
                })
        })
    })
}

pub(super) fn load_schema_and_hold_assets(
    manifest_path: &Path,
    expected_manifest_sha256: &str,
    selected_regression_path: &Path,
    approved_regression_decoded_rgba_blake3: &str,
    selected_regression_raw_sha256: &str,
) -> D0Result<HeldVisualManifestSchema> {
    load_schema_and_hold_assets_with_limit(
        manifest_path,
        expected_manifest_sha256,
        selected_regression_path,
        approved_regression_decoded_rgba_blake3,
        selected_regression_raw_sha256,
        BYTE_CEILING,
    )
}

fn load_schema_and_hold_assets_with_limit(
    manifest_path: &Path,
    expected_manifest_sha256: &str,
    selected_regression_path: &Path,
    approved_regression_decoded_rgba_blake3: &str,
    selected_regression_raw_sha256: &str,
    held_input_limit: u64,
) -> D0Result<HeldVisualManifestSchema> {
    let expected_manifest_sha256 = parse_hash(
        expected_manifest_sha256,
        "expected manifest sha256 is invalid",
    )
    .map_err(|source| context(MANIFEST_VALIDATION_ERROR, source))?;
    require_hash(
        approved_regression_decoded_rgba_blake3,
        "approved regression decoded rgba blake3 is invalid",
    )
    .map_err(|source| context(MANIFEST_VALIDATION_ERROR, source))?;
    require_hash(
        selected_regression_raw_sha256,
        "selected regression raw sha256 is invalid",
    )
    .map_err(|source| context(MANIFEST_VALIDATION_ERROR, source))?;

    let mut held_budget = HeldInputBudget::new(held_input_limit);
    let manifest = held_budget.open(manifest_path, MANIFEST_PATH_ERROR)?;
    require(
        manifest.sha256() == expected_manifest_sha256,
        "visual manifest sha256 mismatch",
    )
    .map_err(|source| context(MANIFEST_HASH_ERROR, source))?;
    let schema: VisualManifestSchema = serde_json::from_slice(manifest.bytes())
        .map_err(|source| context(MANIFEST_SCHEMA_ERROR, source))?;
    schema
        .validate(
            selected_regression_path,
            approved_regression_decoded_rgba_blake3,
            selected_regression_raw_sha256,
        )
        .map_err(|source| context(MANIFEST_VALIDATION_ERROR, source))?;

    let mut entries = Vec::with_capacity(schema.entries.len());
    for entry in &schema.entries {
        let source = open_asset(&entry.path, &entry.sha256, &mut held_budget)?;
        let clean_reference = open_asset(
            &entry.clean_reference_path,
            &entry.clean_reference_sha256,
            &mut held_budget,
        )?;
        let mut targets = Vec::with_capacity(entry.targets.len());
        for target in &entry.targets {
            targets.push(HeldVisualManifestTarget {
                erase_source_ink_mask: open_asset(
                    &target.erase_source_ink_mask_path,
                    &target.erase_source_ink_mask_sha256,
                    &mut held_budget,
                )?,
                residual_source_ink_mask: open_asset(
                    &target.residual_source_ink_mask_path,
                    &target.residual_source_ink_mask_sha256,
                    &mut held_budget,
                )?,
            });
        }
        entries.push(HeldVisualManifestEntry {
            source,
            clean_reference,
            targets,
        });
    }

    Ok(HeldVisualManifestSchema {
        schema,
        manifest,
        entries,
    })
}

impl VisualManifestSchema {
    fn validate(
        &self,
        selected_regression_path: &Path,
        approved_regression_decoded_rgba_blake3: &str,
        selected_regression_raw_sha256: &str,
    ) -> io::Result<()> {
        require(
            self.version == VISUAL_MANIFEST_VERSION,
            "visual manifest version must be 1",
        )?;
        require(
            self.entries.len() == VISUAL_MANIFEST_ENTRY_COUNT,
            "visual manifest must contain exactly 9 entries",
        )?;

        let mut entry_ids = HashSet::new();
        let mut source_decoded_hashes = HashSet::new();
        let mut clean_decoded_hashes = HashSet::new();
        let mut calibration_hashes = HashSet::new();
        let mut holdout_hashes = HashSet::new();
        let mut role_counts = [0_usize; 3];
        let mut regression = None;
        let mut has_holdout_strict_stroke = false;

        for entry in &self.entries {
            let _schema_only = (
                &entry.dimension_bin,
                &entry.aspect,
                &entry.background,
                &entry.protected_rois,
                entry.multi_node,
            );
            require_id(&entry.id, "entry id is empty or has surrounding whitespace")?;
            require(entry_ids.insert(entry.id.as_str()), "duplicate entry id")?;
            require(
                !entry.targets.is_empty(),
                "every entry must contain at least one target",
            )?;
            for hash in [
                &entry.sha256,
                &entry.decoded_rgba_blake3,
                &entry.clean_reference_sha256,
                &entry.clean_reference_decoded_rgba_blake3,
            ] {
                require_hash(hash, "visual manifest hash is invalid")?;
            }
            source_decoded_hashes.insert(entry.decoded_rgba_blake3.as_str());
            clean_decoded_hashes.insert(entry.clean_reference_decoded_rgba_blake3.as_str());

            let mut role_hashes = match entry.role {
                EntryRole::Regression => {
                    role_counts[0] += 1;
                    regression = Some(entry);
                    None
                }
                EntryRole::Calibration => {
                    role_counts[1] += 1;
                    Some(&mut calibration_hashes)
                }
                EntryRole::Holdout => {
                    role_counts[2] += 1;
                    Some(&mut holdout_hashes)
                }
            };
            if let Some(role_hashes) = &mut role_hashes {
                role_hashes.extend([
                    entry.sha256.as_str(),
                    entry.decoded_rgba_blake3.as_str(),
                    entry.clean_reference_sha256.as_str(),
                    entry.clean_reference_decoded_rgba_blake3.as_str(),
                ]);
            }

            let mut target_ids = HashSet::new();
            for target in &entry.targets {
                let _schema_only = (
                    target.source_roi,
                    target.clean_reference_edit_roi,
                    &target.position,
                    &target.writing,
                    &target.translation_length,
                );
                require_id(
                    &target.id,
                    "target id is empty or has surrounding whitespace",
                )?;
                require(
                    target_ids.insert(target.id.as_str()),
                    "duplicate entry-local target id",
                )?;
                require_hash(
                    &target.erase_source_ink_mask_sha256,
                    "visual manifest hash is invalid",
                )?;
                require_hash(
                    &target.residual_source_ink_mask_sha256,
                    "visual manifest hash is invalid",
                )?;
                if let Some(role_hashes) = &mut role_hashes {
                    role_hashes.extend([
                        target.erase_source_ink_mask_sha256.as_str(),
                        target.residual_source_ink_mask_sha256.as_str(),
                    ]);
                }
                require(
                    !matches!(target.effect, Effect::Shadow | Effect::Glow)
                        || target.expected == Expected::UnsupportedSourceColor,
                    "shadow or glow target must expect unsupported_source_color",
                )?;
                has_holdout_strict_stroke |= entry.role == EntryRole::Holdout
                    && target.effect == Effect::Stroke
                    && target.expected == Expected::AutomaticStrict;
            }
        }

        require(
            role_counts == [1, 4, 4],
            "visual manifest roles must be exactly 1 regression, 4 calibration, and 4 holdout",
        )?;
        require(
            calibration_hashes.is_disjoint(&holdout_hashes),
            "calibration and holdout raw or decoded hashes overlap",
        )?;
        require(
            source_decoded_hashes.len() == self.entries.len(),
            "duplicate source decoded rgba blake3",
        )?;
        require(
            clean_decoded_hashes.len() == self.entries.len(),
            "duplicate clean decoded rgba blake3",
        )?;
        let regression = regression.ok_or_else(|| invalid_data("regression entry is missing"))?;
        let selected_regression_path = selected_regression_path
            .to_str()
            .ok_or_else(|| invalid_data("selected regression path is not UTF-8"))?;
        require(
            regression.path == selected_regression_path,
            "selected regression path mismatch",
        )?;
        require(
            regression.decoded_rgba_blake3 == approved_regression_decoded_rgba_blake3,
            "regression decoded rgba blake3 mismatch",
        )?;
        require(
            regression.sha256 == selected_regression_raw_sha256,
            "selected regression raw sha256 mismatch",
        )?;
        require(
            has_holdout_strict_stroke,
            "holdout automatic_strict stroke target is missing",
        )
    }
}

struct HeldInputBudget {
    remaining: u64,
}

impl HeldInputBudget {
    fn new(limit: u64) -> Self {
        Self { remaining: limit }
    }

    fn open(&mut self, path: &Path, category: &'static str) -> D0Result<HeldInput> {
        let held = HeldInput::open_bounded(path, self.remaining)
            .map_err(|source| context(category, source))?;
        self.remaining = self
            .remaining
            .checked_sub(held.bytes().len() as u64)
            .ok_or_else(|| context(category, io::Error::other("held input budget underflow")))?;
        Ok(held)
    }
}

fn open_asset(
    path: &str,
    declared_sha256: &str,
    held_budget: &mut HeldInputBudget,
) -> D0Result<HeldInput> {
    let expected = parse_hash(declared_sha256, "visual manifest hash is invalid")
        .map_err(|source| context(MANIFEST_VALIDATION_ERROR, source))?;
    let held = held_budget.open(Path::new(path), ASSET_PATH_ERROR)?;
    require(held.sha256() == expected, "asset raw sha256 mismatch")
        .map_err(|source| context(ASSET_HASH_ERROR, source))?;
    Ok(held)
}

fn context(category: &'static str, source: impl Error + Send + Sync + 'static) -> D0ManifestError {
    D0ManifestError {
        category,
        source: Box::new(source),
    }
}

fn require_hash(value: &str, message: &'static str) -> io::Result<()> {
    parse_hash(value, message).map(|_| ())
}

fn parse_hash(value: &str, message: &'static str) -> io::Result<[u8; 32]> {
    if value.len() != 64
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        return Err(invalid_data(message));
    }
    let mut bytes = [0; 32];
    for (index, pair) in value.as_bytes().chunks_exact(2).enumerate() {
        bytes[index] = (hex_nibble(pair[0]) << 4) | hex_nibble(pair[1]);
    }
    Ok(bytes)
}

fn hex_nibble(byte: u8) -> u8 {
    match byte {
        b'0'..=b'9' => byte - b'0',
        b'a'..=b'f' => byte - b'a' + 10,
        _ => unreachable!("hash syntax checked before decoding"),
    }
}

fn require_id(value: &str, message: &'static str) -> io::Result<()> {
    require(!value.is_empty() && value.trim() == value, message)
}

fn require(condition: bool, message: &'static str) -> io::Result<()> {
    condition.then_some(()).ok_or_else(|| invalid_data(message))
}

fn invalid_data(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::Cell;
    use std::fs;
    use std::os::unix::fs::symlink;
    use std::path::PathBuf;

    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    use tempfile::TempDir;

    struct Fixture {
        _temp: TempDir,
        root: PathBuf,
        manifest_path: PathBuf,
        value: Value,
        approved_regression_decoded: String,
        selected_regression_raw: String,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = TempDir::new().unwrap();
            let root = fs::canonicalize(temp.path()).unwrap();
            let mut entries = Vec::new();
            for index in 0..VISUAL_MANIFEST_ENTRY_COUNT {
                let source = write_asset(&root, &format!("source-{index}"), index, "source");
                let clean = write_asset(&root, &format!("clean-{index}"), index, "clean");
                let erase = write_asset(&root, &format!("erase-{index}"), index, "erase");
                let residual = write_asset(&root, &format!("residual-{index}"), index, "residual");
                let dimension_bin = ["lt720", "720_1439", "1440_2159", "gte2160"][index % 4];
                let aspect = ["portrait", "landscape", "square_or_near"][index % 3];
                let background = ["pure", "gradient", "texture", "product", "person"][index % 5];
                let translation_length = ["short", "equal", "2x", "3x"][index % 4];
                entries.push(json!({
                    "id": format!("entry-{index}"),
                    "path": source.0,
                    "sha256": source.1,
                    "decoded_rgba_blake3": digest(format!("source-decoded-{index}").as_bytes()),
                    "clean_reference_path": clean.0,
                    "clean_reference_sha256": clean.1,
                    "clean_reference_decoded_rgba_blake3":
                        digest(format!("clean-decoded-{index}").as_bytes()),
                    "role": match index {
                        0 => "regression",
                        1..=4 => "calibration",
                        _ => "holdout",
                    },
                    "dimension_bin": dimension_bin,
                    "aspect": aspect,
                    "background": background,
                    "targets": [{
                        "id": "target-0",
                        "source_roi": [0, 0, 1, 1],
                        "clean_reference_edit_roi": [0, 0, 1, 1],
                        "erase_source_ink_mask_path": erase.0,
                        "erase_source_ink_mask_sha256": erase.1,
                        "residual_source_ink_mask_path": residual.0,
                        "residual_source_ink_mask_sha256": residual.1,
                        "position": if index % 2 == 0 { "interior" } else { "page_edge" },
                        "writing": if index % 2 == 0 { "horizontal" } else { "vertical" },
                        "effect": if index == 5 { "stroke" } else { "plain" },
                        "translation_length": translation_length,
                        "expected": "automatic_strict"
                    }],
                    "protected_rois": [[2, 2, 3, 3]],
                    "multi_node": false
                }));
            }
            let approved_regression_decoded = entries[0]["decoded_rgba_blake3"]
                .as_str()
                .unwrap()
                .to_owned();
            let selected_regression_raw = entries[0]["sha256"].as_str().unwrap().to_owned();
            Self {
                _temp: temp,
                manifest_path: root.join("manifest.json"),
                root,
                value: json!({"version": 1, "entries": entries}),
                approved_regression_decoded,
                selected_regression_raw,
            }
        }

        fn store(&self) -> String {
            let bytes = serde_json::to_vec(&self.value).unwrap();
            fs::write(&self.manifest_path, &bytes).unwrap();
            digest(&bytes)
        }

        fn load(&self, expected_manifest_sha256: &str) -> D0Result<HeldVisualManifestSchema> {
            self.load_with_selected_path(expected_manifest_sha256, &self.selected_regression_path())
        }

        fn load_with_selected_path(
            &self,
            expected_manifest_sha256: &str,
            selected_regression_path: &Path,
        ) -> D0Result<HeldVisualManifestSchema> {
            load_schema_and_hold_assets(
                &self.manifest_path,
                expected_manifest_sha256,
                selected_regression_path,
                &self.approved_regression_decoded,
                &self.selected_regression_raw,
            )
        }

        fn selected_regression_path(&self) -> PathBuf {
            PathBuf::from(self.value["entries"][0]["path"].as_str().unwrap())
        }

        fn load_with_limit(
            &self,
            expected_manifest_sha256: &str,
            limit: u64,
        ) -> D0Result<HeldVisualManifestSchema> {
            load_schema_and_hold_assets_with_limit(
                &self.manifest_path,
                expected_manifest_sha256,
                &self.selected_regression_path(),
                &self.approved_regression_decoded,
                &self.selected_regression_raw,
                limit,
            )
        }
    }

    fn write_asset(root: &Path, name: &str, index: usize, kind: &str) -> (String, String) {
        let path = root.join(name);
        let bytes = format!("{kind}-{index}").into_bytes();
        fs::write(&path, &bytes).unwrap();
        (path.to_str().unwrap().to_owned(), digest(&bytes))
    }

    fn digest(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn expect_error(
        result: D0Result<HeldVisualManifestSchema>,
        category: &'static str,
    ) -> D0ManifestError {
        let error = match result {
            Ok(_) => panic!("expected manifest rejection"),
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

    fn assert_error(
        result: D0Result<HeldVisualManifestSchema>,
        category: &'static str,
        source_message: &str,
    ) {
        let error = expect_error(result, category);
        assert_eq!(error.source().unwrap().to_string(), source_message);
    }

    fn assert_io_source(error: &D0ManifestError, kind: io::ErrorKind) {
        let source = error.source().unwrap().downcast_ref::<io::Error>().unwrap();
        assert_eq!(source.kind(), kind);
    }

    fn assert_serde_source(error: &D0ManifestError) {
        assert!(
            error
                .source()
                .unwrap()
                .downcast_ref::<serde_json::Error>()
                .is_some()
        );
    }

    fn copy_overlap(kind: &str, calibration: &Value, holdout: &mut Value) {
        match kind {
            "source raw" => {
                holdout["path"] = calibration["path"].clone();
                holdout["sha256"] = calibration["sha256"].clone();
            }
            "source decoded" => {
                holdout["decoded_rgba_blake3"] = calibration["decoded_rgba_blake3"].clone();
            }
            "clean raw" => {
                holdout["clean_reference_path"] = calibration["clean_reference_path"].clone();
                holdout["clean_reference_sha256"] = calibration["clean_reference_sha256"].clone();
            }
            "clean decoded" => {
                holdout["clean_reference_decoded_rgba_blake3"] =
                    calibration["clean_reference_decoded_rgba_blake3"].clone();
            }
            "erase mask raw" => {
                holdout["targets"][0]["erase_source_ink_mask_path"] =
                    calibration["targets"][0]["erase_source_ink_mask_path"].clone();
                holdout["targets"][0]["erase_source_ink_mask_sha256"] =
                    calibration["targets"][0]["erase_source_ink_mask_sha256"].clone();
            }
            "residual mask raw" => {
                holdout["targets"][0]["residual_source_ink_mask_path"] =
                    calibration["targets"][0]["residual_source_ink_mask_path"].clone();
                holdout["targets"][0]["residual_source_ink_mask_sha256"] =
                    calibration["targets"][0]["residual_source_ink_mask_sha256"].clone();
            }
            _ => panic!("unknown overlap case"),
        }
    }

    #[test]
    fn d0_visual_manifest_schema_accepts_valid_one_four_four_and_holds_all_assets() {
        let fixture = Fixture::new();
        let expected = fixture.store();
        let held = fixture.load(&expected).unwrap();

        assert_eq!(held.schema.version, 1);
        assert_eq!(held.schema.entries.len(), 9);
        assert_eq!(held.entries.len(), 9);
        assert_eq!(held.manifest.sha256(), parse_hash(&expected, "").unwrap());
        assert_eq!(held.entries[0].source.bytes(), b"source-0");
        assert_eq!(held.entries[0].clean_reference.bytes(), b"clean-0");
        assert_eq!(held.entries[0].targets.len(), 1);
        assert_eq!(
            held.entries[0].targets[0].erase_source_ink_mask.bytes(),
            b"erase-0"
        );
        assert_eq!(
            held.entries[0].targets[0].residual_source_ink_mask.bytes(),
            b"residual-0"
        );
        let success_calls = Cell::new(0);
        held.with_revalidated_paths(|| {
            success_calls.set(success_calls.get() + 1);
            Ok(())
        })
        .unwrap();
        assert_eq!(success_calls.get(), 1);
    }

    #[test]
    fn d0_visual_manifest_schema_bounds_cumulative_held_inputs_before_append() {
        let fixture = Fixture::new();
        let expected = fixture.store();
        let manifest_bytes = fs::metadata(&fixture.manifest_path).unwrap().len();
        let first_source = fs::metadata(fixture.value["entries"][0]["path"].as_str().unwrap())
            .unwrap()
            .len();

        for limit in [manifest_bytes, manifest_bytes + first_source] {
            let error = expect_error(fixture.load_with_limit(&expected, limit), ASSET_PATH_ERROR);
            assert_io_source(&error, io::ErrorKind::InvalidData);
            assert_eq!(
                error.source().unwrap().to_string(),
                "held input exceeds byte limit"
            );
        }
    }

    #[test]
    fn d0_visual_manifest_schema_rejects_unknown_and_missing_fields() {
        let mut unknown = Fixture::new();
        unknown.value["unexpected"] = true.into();
        let expected = unknown.store();
        let error = expect_error(unknown.load(&expected), MANIFEST_SCHEMA_ERROR);
        assert_serde_source(&error);

        let mut missing = Fixture::new();
        missing.value["entries"][0]
            .as_object_mut()
            .unwrap()
            .remove("multi_node");
        let expected = missing.store();
        let error = expect_error(missing.load(&expected), MANIFEST_SCHEMA_ERROR);
        assert_serde_source(&error);
    }

    #[test]
    fn d0_visual_manifest_schema_rejects_relative_noncanonical_and_symlink_paths() {
        for replacement in ["relative/source", "/tmp/../tmp/source"] {
            let mut fixture = Fixture::new();
            fixture.value["entries"][0]["path"] = replacement.into();
            let expected = fixture.store();
            let error = expect_error(fixture.load(&expected), ASSET_PATH_ERROR);
            assert_io_source(&error, io::ErrorKind::InvalidInput);
        }

        let mut fixture = Fixture::new();
        let source = PathBuf::from(fixture.value["entries"][0]["path"].as_str().unwrap());
        let link = fixture.root.join("source-link");
        symlink(source, &link).unwrap();
        fixture.value["entries"][0]["path"] = link.to_str().unwrap().into();
        let expected = fixture.store();
        let error = expect_error(fixture.load(&expected), ASSET_PATH_ERROR);
        assert!(
            error
                .source()
                .unwrap()
                .downcast_ref::<io::Error>()
                .is_some()
        );
    }

    #[test]
    fn d0_visual_manifest_schema_preserves_manifest_path_io_source() {
        let fixture = Fixture::new();
        let expected = fixture.store();
        let error = expect_error(
            load_schema_and_hold_assets(
                Path::new("relative-manifest"),
                &expected,
                &fixture.selected_regression_path(),
                &fixture.approved_regression_decoded,
                &fixture.selected_regression_raw,
            ),
            MANIFEST_PATH_ERROR,
        );
        assert_io_source(&error, io::ErrorKind::InvalidInput);
        assert_eq!(
            error.source().unwrap().to_string(),
            "path must be an absolute canonical file path"
        );
    }

    #[test]
    fn d0_visual_manifest_schema_binds_manifest_before_parse() {
        let fixture = Fixture::new();
        let malformed = b"{";
        fs::write(&fixture.manifest_path, malformed).unwrap();
        assert_error(
            fixture.load(&"0".repeat(64)),
            MANIFEST_HASH_ERROR,
            "visual manifest sha256 mismatch",
        );

        let error = expect_error(fixture.load(&digest(malformed)), MANIFEST_SCHEMA_ERROR);
        assert_serde_source(&error);
    }

    #[test]
    fn d0_visual_manifest_schema_rejects_asset_raw_hash_mismatch() {
        let fixture = Fixture::new();
        let expected = fixture.store();
        let source = fixture.value["entries"][0]["path"].as_str().unwrap();
        fs::write(source, b"changed").unwrap();
        assert_error(
            fixture.load(&expected),
            ASSET_HASH_ERROR,
            "asset raw sha256 mismatch",
        );
    }

    #[test]
    fn d0_visual_manifest_schema_rejects_role_count_drift() {
        let mut fixture = Fixture::new();
        fixture.value["entries"][8]["role"] = "calibration".into();
        let expected = fixture.store();
        assert_error(
            fixture.load(&expected),
            MANIFEST_VALIDATION_ERROR,
            "visual manifest roles must be exactly 1 regression, 4 calibration, and 4 holdout",
        );
    }

    #[test]
    fn d0_visual_manifest_schema_rejects_duplicate_entry_and_target_ids() {
        let mut duplicate_entry = Fixture::new();
        duplicate_entry.value["entries"][1]["id"] =
            duplicate_entry.value["entries"][0]["id"].clone();
        let expected = duplicate_entry.store();
        assert_error(
            duplicate_entry.load(&expected),
            MANIFEST_VALIDATION_ERROR,
            "duplicate entry id",
        );

        let mut duplicate_target = Fixture::new();
        let target = duplicate_target.value["entries"][0]["targets"][0].clone();
        duplicate_target.value["entries"][0]["targets"]
            .as_array_mut()
            .unwrap()
            .push(target);
        let expected = duplicate_target.store();
        assert_error(
            duplicate_target.load(&expected),
            MANIFEST_VALIDATION_ERROR,
            "duplicate entry-local target id",
        );
    }

    #[test]
    fn d0_visual_manifest_schema_rejects_invalid_enum_and_hash() {
        let mut invalid_enum = Fixture::new();
        invalid_enum.value["entries"][0]["targets"][0]["expected"] = "success".into();
        let expected = invalid_enum.store();
        let error = expect_error(invalid_enum.load(&expected), MANIFEST_SCHEMA_ERROR);
        assert_serde_source(&error);

        let mut invalid_hash = Fixture::new();
        invalid_hash.value["entries"][0]["sha256"] = "A".repeat(64).into();
        let expected = invalid_hash.store();
        assert_error(
            invalid_hash.load(&expected),
            MANIFEST_VALIDATION_ERROR,
            "visual manifest hash is invalid",
        );
    }

    #[test]
    fn d0_visual_manifest_schema_rejects_all_calibration_holdout_hash_overlap_classes() {
        for kind in [
            "source raw",
            "source decoded",
            "clean raw",
            "clean decoded",
            "erase mask raw",
            "residual mask raw",
        ] {
            let mut fixture = Fixture::new();
            let calibration = fixture.value["entries"][1].clone();
            copy_overlap(kind, &calibration, &mut fixture.value["entries"][5]);
            let expected = fixture.store();
            assert_error(
                fixture.load(&expected),
                MANIFEST_VALIDATION_ERROR,
                "calibration and holdout raw or decoded hashes overlap",
            );
        }
    }

    #[test]
    fn d0_visual_manifest_schema_rejects_regression_identity_mismatch() {
        let fixture = Fixture::new();
        let expected = fixture.store();
        let selected_regression_path = fixture.selected_regression_path();
        assert_error(
            load_schema_and_hold_assets(
                &fixture.manifest_path,
                &expected,
                &selected_regression_path,
                &"0".repeat(64),
                &fixture.selected_regression_raw,
            ),
            MANIFEST_VALIDATION_ERROR,
            "regression decoded rgba blake3 mismatch",
        );
        assert_error(
            load_schema_and_hold_assets(
                &fixture.manifest_path,
                &expected,
                &selected_regression_path,
                &fixture.approved_regression_decoded,
                &"0".repeat(64),
            ),
            MANIFEST_VALIDATION_ERROR,
            "selected regression raw sha256 mismatch",
        );
    }

    #[test]
    fn d0_visual_manifest_schema_binds_exact_selected_regression_path() {
        let fixture = Fixture::new();
        let expected = fixture.store();
        let selected = fixture.selected_regression_path();
        fixture
            .load_with_selected_path(&expected, &selected)
            .unwrap();

        let alternate = fixture.root.join("byte-identical-selected-source");
        fs::copy(&selected, &alternate).unwrap();
        assert_eq!(fs::read(&selected).unwrap(), fs::read(&alternate).unwrap());
        assert_error(
            fixture.load_with_selected_path(&expected, &alternate),
            MANIFEST_VALIDATION_ERROR,
            "selected regression path mismatch",
        );
    }

    #[test]
    fn d0_visual_manifest_schema_rejects_shadow_and_glow_outside_unsupported_source_color() {
        for effect in ["shadow", "glow"] {
            for expected_mode in [
                "automatic_strict",
                "manual_override",
                "unsupported_rotation",
            ] {
                let mut fixture = Fixture::new();
                fixture.value["entries"][0]["targets"][0]["effect"] = effect.into();
                fixture.value["entries"][0]["targets"][0]["expected"] = expected_mode.into();
                let expected = fixture.store();
                assert_error(
                    fixture.load(&expected),
                    MANIFEST_VALIDATION_ERROR,
                    "shadow or glow target must expect unsupported_source_color",
                );
            }
        }
    }

    #[test]
    fn d0_visual_manifest_schema_accepts_unsupported_source_color_shadow_and_glow() {
        for effect in ["shadow", "glow"] {
            let mut fixture = Fixture::new();
            fixture.value["entries"][0]["targets"][0]["effect"] = effect.into();
            fixture.value["entries"][0]["targets"][0]["expected"] =
                "unsupported_source_color".into();
            let expected = fixture.store();
            fixture.load(&expected).unwrap();
        }
    }

    #[test]
    fn d0_visual_manifest_schema_requires_automatic_strict_on_holdout_stroke() {
        for expected_mode in [
            "manual_override",
            "unsupported_source_color",
            "unsupported_rotation",
        ] {
            let mut fixture = Fixture::new();
            fixture.value["entries"][5]["targets"][0]["expected"] = expected_mode.into();
            let expected = fixture.store();
            assert_error(
                fixture.load(&expected),
                MANIFEST_VALIDATION_ERROR,
                "holdout automatic_strict stroke target is missing",
            );
        }
    }

    #[test]
    fn d0_visual_manifest_schema_requires_a_holdout_stroke_target() {
        let mut fixture = Fixture::new();
        fixture.value["entries"][5]["targets"][0]["effect"] = "plain".into();
        let expected = fixture.store();
        assert_error(
            fixture.load(&expected),
            MANIFEST_VALIDATION_ERROR,
            "holdout automatic_strict stroke target is missing",
        );
    }
}
