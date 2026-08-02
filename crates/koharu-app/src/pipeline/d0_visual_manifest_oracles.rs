//! Test-only D0 ROI and immutable Source/Clean oracle validation.
//!
//! This stage retains all upstream held and decoded data. It does not open
//! runtime output, load a model, mutate Scene state, or validate runtime cells.

use std::error::Error;
use std::fmt;
use std::io;

use super::d0_revision_46_contract::BYTE_CEILING;
use super::d0_visual_manifest_pixels::DimensionAndMaskValidatedManifest;
use super::d0_visual_manifest_schema::Expected;

const GEOMETRY_ERROR: &str = "d0.visual_oracles.geometry";
const DISJOINT_ERROR: &str = "d0.visual_oracles.disjoint";
const MASK_GEOMETRY_ERROR: &str = "d0.visual_oracles.mask_geometry";
const BUDGET_ERROR: &str = "d0.visual_oracles.budget";
const SUCCESS_DELTA_ERROR: &str = "d0.visual_oracles.success_delta";
const UNSUPPORTED_EQUALITY_ERROR: &str = "d0.visual_oracles.unsupported_equality";
const PROTECTED_EQUALITY_ERROR: &str = "d0.visual_oracles.protected_equality";
const OUTSIDE_SUCCESS_ERROR: &str = "d0.visual_oracles.outside_success";

type OracleResult<T> = Result<T, D0OracleError>;

#[derive(Debug)]
pub(super) struct D0OracleError {
    category: &'static str,
    source: Box<dyn Error + Send + Sync>,
}

impl D0OracleError {
    pub(super) fn category(&self) -> &'static str {
        self.category
    }
}

impl fmt::Display for D0OracleError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{}: {}", self.category, self.source)
    }
}

impl Error for D0OracleError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        Some(self.source.as_ref())
    }
}

pub(super) struct OracleValidatedManifest {
    pub(super) upstream: DimensionAndMaskValidatedManifest,
    pub(super) entries: Vec<OracleValidatedEntry>,
    pub(super) final_oracle_retained_bytes: u64,
}

pub(super) struct OracleValidatedEntry {
    pub(super) protected_rois: Vec<ValidatedHalfOpenRect>,
    pub(super) targets: Vec<OracleValidatedTarget>,
}

pub(super) struct OracleValidatedTarget {
    pub(super) source_roi: ValidatedHalfOpenRect,
    pub(super) edit_roi: ValidatedHalfOpenRect,
    pub(super) delta_mask: Box<[u8]>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct ValidatedHalfOpenRect {
    pub(super) left: u32,
    pub(super) top: u32,
    pub(super) right: u32,
    pub(super) bottom: u32,
}

impl ValidatedHalfOpenRect {
    fn width(self) -> u64 {
        u64::from(self.right - self.left)
    }

    fn height(self) -> u64 {
        u64::from(self.bottom - self.top)
    }

    fn intersects(self, other: Self) -> bool {
        self.left < other.right
            && other.left < self.right
            && self.top < other.bottom
            && other.top < self.bottom
    }

    fn contains(self, x: u32, y: u32) -> bool {
        self.left <= x && x < self.right && self.top <= y && y < self.bottom
    }
}

struct GeometryPlan {
    entries: Vec<GeometryEntry>,
    total_delta_bytes: u64,
    max_page_workspace: u64,
}

struct GeometryEntry {
    width: u32,
    height: u32,
    page_pixels: usize,
    delta_bytes: u64,
    protected_rois: Vec<ValidatedHalfOpenRect>,
    targets: Vec<GeometryTarget>,
}

struct GeometryTarget {
    source_roi: ValidatedHalfOpenRect,
    edit_roi: ValidatedHalfOpenRect,
    expected: Expected,
    delta_bytes: usize,
}

pub(super) fn validate_visual_oracles(
    upstream: DimensionAndMaskValidatedManifest,
) -> OracleResult<OracleValidatedManifest> {
    validate_visual_oracles_with_limit(upstream, BYTE_CEILING)
}

fn validate_visual_oracles_with_limit(
    upstream: DimensionAndMaskValidatedManifest,
    limit: u64,
) -> OracleResult<OracleValidatedManifest> {
    let plan = build_geometry_plan(&upstream)?;
    validate_disjoint_geometry(&plan)?;
    validate_mask_geometry(&upstream, &plan)?;
    let (final_retained, workspace_len) = checked_oracle_preflight(
        upstream.final_retained_bytes,
        plan.total_delta_bytes,
        plan.max_page_workspace,
        limit,
    )
    .map_err(|source| context(BUDGET_ERROR, source))?;

    let mut workspace = Vec::new();
    workspace
        .try_reserve_exact(workspace_len)
        .map_err(|source| context(BUDGET_ERROR, source))?;
    workspace.resize(workspace_len, 0);
    let entries = validate_pixel_oracles(&upstream, plan.entries, &mut workspace)?;
    drop(workspace);

    Ok(OracleValidatedManifest {
        upstream,
        entries,
        final_oracle_retained_bytes: final_retained,
    })
}

fn build_geometry_plan(upstream: &DimensionAndMaskValidatedManifest) -> OracleResult<GeometryPlan> {
    require(
        upstream.entries.len() == upstream.held_schema.schema.entries.len(),
        GEOMETRY_ERROR,
        "schema and decoded entry counts differ",
    )?;
    let mut entries = Vec::with_capacity(upstream.entries.len());
    let mut total_delta_bytes = 0_u64;
    let mut max_page_workspace = 0_u64;

    for (decoded, schema) in upstream
        .entries
        .iter()
        .zip(&upstream.held_schema.schema.entries)
    {
        let (width, height) = decoded.source.dimensions();
        let page_pixels_u64 = checked_area(u64::from(width), u64::from(height))
            .map_err(|source| context(GEOMETRY_ERROR, source))?;
        let page_pixels =
            usize::try_from(page_pixels_u64).map_err(|source| context(GEOMETRY_ERROR, source))?;
        require(
            decoded.targets.len() == schema.targets.len(),
            GEOMETRY_ERROR,
            "schema and decoded target counts differ",
        )?;

        let protected_rois = schema
            .protected_rois
            .iter()
            .map(|raw| parse_rect(*raw, width, height))
            .collect::<OracleResult<Vec<_>>>()?;
        let mut targets = Vec::with_capacity(schema.targets.len());
        let mut entry_delta_bytes = 0_u64;
        for target in &schema.targets {
            let source_roi = parse_rect(target.source_roi, width, height)?;
            let edit_roi = parse_rect(target.clean_reference_edit_roi, width, height)?;
            let delta_bytes_u64 = checked_area(edit_roi.width(), edit_roi.height())
                .map_err(|source| context(GEOMETRY_ERROR, source))?;
            let delta_bytes = usize::try_from(delta_bytes_u64)
                .map_err(|source| context(GEOMETRY_ERROR, source))?;
            entry_delta_bytes = entry_delta_bytes
                .checked_add(delta_bytes_u64)
                .ok_or_else(|| failure(GEOMETRY_ERROR, "entry ROI area overflow"))?;
            targets.push(GeometryTarget {
                source_roi,
                edit_roi,
                expected: target.expected,
                delta_bytes,
            });
        }
        total_delta_bytes = total_delta_bytes
            .checked_add(entry_delta_bytes)
            .ok_or_else(|| failure(GEOMETRY_ERROR, "total ROI area overflow"))?;
        max_page_workspace = max_page_workspace.max(page_pixels_u64);
        entries.push(GeometryEntry {
            width,
            height,
            page_pixels,
            delta_bytes: entry_delta_bytes,
            protected_rois,
            targets,
        });
    }
    Ok(GeometryPlan {
        entries,
        total_delta_bytes,
        max_page_workspace,
    })
}

fn parse_rect(
    raw: [u64; 4],
    page_width: u32,
    page_height: u32,
) -> OracleResult<ValidatedHalfOpenRect> {
    let [left, top, right, bottom] = raw.map(|coordinate| {
        u32::try_from(coordinate).map_err(|source| context(GEOMETRY_ERROR, source))
    });
    let rect = ValidatedHalfOpenRect {
        left: left?,
        top: top?,
        right: right?,
        bottom: bottom?,
    };
    require(
        rect.left < rect.right && rect.top < rect.bottom,
        GEOMETRY_ERROR,
        "ROI is empty or reversed",
    )?;
    require(
        rect.right <= page_width && rect.bottom <= page_height,
        GEOMETRY_ERROR,
        "ROI exceeds oriented page bounds",
    )?;
    checked_area(rect.width(), rect.height()).map_err(|source| context(GEOMETRY_ERROR, source))?;
    Ok(rect)
}

fn validate_disjoint_geometry(plan: &GeometryPlan) -> OracleResult<()> {
    for entry in &plan.entries {
        for (index, target) in entry.targets.iter().enumerate() {
            require(
                entry
                    .protected_rois
                    .iter()
                    .all(|protected| !target.edit_roi.intersects(*protected)),
                DISJOINT_ERROR,
                "target edit ROI overlaps protected ROI",
            )?;
            require(
                entry.targets[index + 1..]
                    .iter()
                    .all(|other| !target.edit_roi.intersects(other.edit_roi)),
                DISJOINT_ERROR,
                "target edit ROIs overlap",
            )?;
        }
    }
    for entry in &plan.entries {
        let page_pixels =
            u64::try_from(entry.page_pixels).map_err(|source| context(BUDGET_ERROR, source))?;
        require(
            entry.delta_bytes <= page_pixels,
            BUDGET_ERROR,
            "entry ROI area exceeds page pixels",
        )?;
    }
    Ok(())
}

fn validate_mask_geometry(
    upstream: &DimensionAndMaskValidatedManifest,
    plan: &GeometryPlan,
) -> OracleResult<()> {
    for (decoded, geometry) in upstream.entries.iter().zip(&plan.entries) {
        for (target, geometry_target) in decoded.targets.iter().zip(&geometry.targets) {
            for (index, value) in target.agreed_mask.iter().copied().enumerate() {
                if value == 0 {
                    continue;
                }
                let (x, y) = checked_coordinates(index, geometry.width, geometry.height)
                    .map_err(|source| context(MASK_GEOMETRY_ERROR, source))?;
                require(
                    geometry_target.edit_roi.contains(x, y),
                    MASK_GEOMETRY_ERROR,
                    "agreed mask foreground lies outside edit ROI",
                )?;
            }
        }
        // Pairwise-disjoint edit ROIs plus foreground containment above
        // mathematically imply that different targets' masks cannot overlap.
    }
    Ok(())
}

fn validate_pixel_oracles(
    upstream: &DimensionAndMaskValidatedManifest,
    geometry_entries: Vec<GeometryEntry>,
    workspace: &mut [u8],
) -> OracleResult<Vec<OracleValidatedEntry>> {
    let mut entries = Vec::with_capacity(geometry_entries.len());
    for (decoded, geometry) in upstream.entries.iter().zip(&geometry_entries) {
        let mut targets = Vec::with_capacity(geometry.targets.len());
        for (mask, target) in decoded.targets.iter().zip(&geometry.targets) {
            let mut delta = Vec::new();
            delta
                .try_reserve_exact(target.delta_bytes)
                .map_err(|source| context(BUDGET_ERROR, source))?;
            let mut nonempty = false;
            for y in target.edit_roi.top..target.edit_roi.bottom {
                for x in target.edit_roi.left..target.edit_roi.right {
                    let index = checked_page_index(x, y, geometry.width, geometry.height)
                        .map_err(|source| context(BUDGET_ERROR, source))?;
                    let changed = pixels_differ(decoded, index)?;
                    delta.push(u8::from(changed));
                    nonempty |= changed;
                    if is_success(target.expected) && mask.agreed_mask[index] != 0 {
                        require(
                            changed,
                            SUCCESS_DELTA_ERROR,
                            "successful mask pixel is unchanged in Clean",
                        )?;
                    }
                    if !is_success(target.expected) {
                        require(
                            !changed,
                            UNSUPPORTED_EQUALITY_ERROR,
                            "unsupported edit ROI differs from Source",
                        )?;
                    }
                }
            }
            if is_success(target.expected) {
                require(
                    nonempty,
                    SUCCESS_DELTA_ERROR,
                    "successful edit ROI has empty delta",
                )?;
            }
            debug_assert_eq!(delta.len(), target.delta_bytes);
            targets.push(OracleValidatedTarget {
                source_roi: target.source_roi,
                edit_roi: target.edit_roi,
                delta_mask: delta.into_boxed_slice(),
            });
        }
        entries.push(OracleValidatedEntry {
            protected_rois: geometry.protected_rois.clone(),
            targets,
        });
    }

    for (decoded, geometry) in upstream.entries.iter().zip(&geometry_entries) {
        let page_workspace = workspace
            .get_mut(..geometry.page_pixels)
            .ok_or_else(|| failure(BUDGET_ERROR, "page workspace is too small"))?;
        page_workspace.fill(0);
        for target in &geometry.targets {
            if is_success(target.expected) {
                mark_rect(
                    page_workspace,
                    geometry.width,
                    geometry.height,
                    target.edit_roi,
                )?;
            }
        }
        for protected in &geometry.protected_rois {
            for_each_index(*protected, geometry.width, geometry.height, |index| {
                require(
                    !pixels_differ(decoded, index)?,
                    PROTECTED_EQUALITY_ERROR,
                    "protected ROI differs from Source",
                )
            })?;
        }
        for (index, in_success) in page_workspace.iter().copied().enumerate() {
            if in_success == 0 {
                require(
                    !pixels_differ(decoded, index)?,
                    OUTSIDE_SUCCESS_ERROR,
                    "Clean differs outside successful edit ROIs",
                )?;
            }
        }
    }
    Ok(entries)
}

fn pixels_differ(
    entry: &super::d0_visual_manifest_pixels::DimensionAndMaskValidatedEntry,
    pixel_index: usize,
) -> OracleResult<bool> {
    let offset = pixel_index
        .checked_mul(4)
        .ok_or_else(|| failure(BUDGET_ERROR, "RGBA index overflow"))?;
    let end = offset
        .checked_add(4)
        .ok_or_else(|| failure(BUDGET_ERROR, "RGBA index overflow"))?;
    let source = entry
        .source
        .as_raw()
        .get(offset..end)
        .ok_or_else(|| failure(BUDGET_ERROR, "Source RGBA index out of bounds"))?;
    let clean = entry
        .clean_reference
        .as_raw()
        .get(offset..end)
        .ok_or_else(|| failure(BUDGET_ERROR, "Clean RGBA index out of bounds"))?;
    Ok(source != clean)
}

fn mark_rect(
    workspace: &mut [u8],
    width: u32,
    height: u32,
    rect: ValidatedHalfOpenRect,
) -> OracleResult<()> {
    for_each_index(rect, width, height, |index| {
        let value = workspace
            .get_mut(index)
            .ok_or_else(|| failure(BUDGET_ERROR, "workspace index out of bounds"))?;
        *value = 1;
        Ok(())
    })
}

fn for_each_index(
    rect: ValidatedHalfOpenRect,
    width: u32,
    height: u32,
    mut operation: impl FnMut(usize) -> OracleResult<()>,
) -> OracleResult<()> {
    for y in rect.top..rect.bottom {
        for x in rect.left..rect.right {
            operation(
                checked_page_index(x, y, width, height)
                    .map_err(|source| context(BUDGET_ERROR, source))?,
            )?;
        }
    }
    Ok(())
}

fn checked_page_index(x: u32, y: u32, width: u32, height: u32) -> io::Result<usize> {
    if x >= width || y >= height {
        return Err(io::Error::other("page coordinate is out of bounds"));
    }
    let index = u64::from(y)
        .checked_mul(u64::from(width))
        .and_then(|row| row.checked_add(u64::from(x)))
        .ok_or_else(|| io::Error::other("page index overflow"))?;
    usize::try_from(index).map_err(io::Error::other)
}

fn checked_coordinates(index: usize, width: u32, height: u32) -> io::Result<(u32, u32)> {
    let page_pixels = checked_area(u64::from(width), u64::from(height))?;
    let index_u64 = u64::try_from(index).map_err(io::Error::other)?;
    if index_u64 >= page_pixels || width == 0 {
        return Err(io::Error::other("mask index is out of page bounds"));
    }
    let width_u64 = u64::from(width);
    Ok((
        u32::try_from(index_u64 % width_u64).map_err(io::Error::other)?,
        u32::try_from(index_u64 / width_u64).map_err(io::Error::other)?,
    ))
}

fn checked_area(width: u64, height: u64) -> io::Result<u64> {
    width
        .checked_mul(height)
        .ok_or_else(|| io::Error::other("pixel area overflow"))
}

fn checked_oracle_preflight(
    upstream_retained: u64,
    total_delta_bytes: u64,
    page_workspace: u64,
    limit: u64,
) -> io::Result<(u64, usize)> {
    let final_retained = upstream_retained
        .checked_add(total_delta_bytes)
        .ok_or_else(|| io::Error::other("oracle retained-byte overflow"))?;
    final_retained
        .checked_add(page_workspace)
        .filter(|peak| *peak <= limit)
        .ok_or_else(|| io::Error::other("oracle reservation exceeds shared byte ceiling"))?;
    Ok((
        final_retained,
        usize::try_from(page_workspace).map_err(io::Error::other)?,
    ))
}

fn is_success(expected: Expected) -> bool {
    matches!(
        expected,
        Expected::AutomaticStrict | Expected::ManualOverride
    )
}

fn require(condition: bool, category: &'static str, message: &'static str) -> OracleResult<()> {
    condition
        .then_some(())
        .ok_or_else(|| failure(category, message))
}

fn failure(category: &'static str, message: &'static str) -> D0OracleError {
    context(
        category,
        io::Error::new(io::ErrorKind::InvalidData, message),
    )
}

fn context(category: &'static str, source: impl Error + Send + Sync + 'static) -> D0OracleError {
    D0OracleError {
        category,
        source: Box::new(source),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::path::{Path, PathBuf};

    use image::{DynamicImage, ImageFormat, Rgba, RgbaImage};
    use serde_json::{Value, json};
    use sha2::{Digest, Sha256};
    use tempfile::TempDir;

    use super::super::d0_visual_manifest_pixels::validate_dimensions_and_masks;
    use super::super::d0_visual_manifest_schema::load_schema_and_hold_assets;

    const PAGE: u32 = 6;

    struct Fixture {
        _temp: TempDir,
        root: PathBuf,
        manifest_path: PathBuf,
        value: Value,
    }

    impl Fixture {
        fn new() -> Self {
            let temp = TempDir::new().unwrap();
            let root = fs::canonicalize(temp.path()).unwrap();
            let mut entries = Vec::new();
            for index in 0..9 {
                let (x, y) = target_origin(index);
                let source = source_image(index);
                let mut clean = source.clone();
                clean.put_pixel(x, y, changed_pixel(clean.get_pixel(x, y)));
                let mask = mask_at(x, y);
                let source_asset = write_asset(&root, index, "source", &png(&source));
                let clean_asset = write_asset(&root, index, "clean", &png(&clean));
                let erase = write_asset(&root, index, "erase", &png(&mask));
                let residual = write_asset(&root, index, "residual", &png(&mask));
                entries.push(json!({
                    "id": format!("entry-{index}"),
                    "path": source_asset.0,
                    "sha256": source_asset.1,
                    "decoded_rgba_blake3": blake3_hex(&source),
                    "clean_reference_path": clean_asset.0,
                    "clean_reference_sha256": clean_asset.1,
                    "clean_reference_decoded_rgba_blake3": blake3_hex(&clean),
                    "role": if index == 0 {"regression"} else if index <= 4 {"calibration"} else {"holdout"},
                    "dimension_bin": "lt720",
                    "aspect": "square_or_near",
                    "background": "pure",
                    "targets": [target_json(
                        "target-0",
                        [x, y, x + 2, y + 2],
                        erase,
                        residual,
                        if index == 5 {"stroke"} else {"plain"},
                        "automatic_strict",
                    )],
                    "protected_rois": [],
                    "multi_node": false
                }));
            }
            Self {
                _temp: temp,
                manifest_path: root.join("manifest.json"),
                root,
                value: json!({"version": 1, "entries": entries}),
            }
        }

        fn validate(&self) -> OracleResult<OracleValidatedManifest> {
            validate_visual_oracles(self.load_pixels())
        }

        fn load_pixels(&self) -> DimensionAndMaskValidatedManifest {
            let bytes = serde_json::to_vec(&self.value).unwrap();
            fs::write(&self.manifest_path, &bytes).unwrap();
            let regression = &self.value["entries"][0];
            let held = load_schema_and_hold_assets(
                &self.manifest_path,
                &sha256(&bytes),
                Path::new(regression["path"].as_str().unwrap()),
                regression["decoded_rgba_blake3"].as_str().unwrap(),
                regression["sha256"].as_str().unwrap(),
            )
            .unwrap();
            validate_dimensions_and_masks(held).unwrap()
        }

        fn source(&self, entry: usize) -> RgbaImage {
            decode(self.value["entries"][entry]["path"].as_str().unwrap())
        }

        fn set_clean(&mut self, entry: usize, clean: &RgbaImage) {
            replace_image(
                &mut self.value["entries"][entry],
                "clean_reference_path",
                "clean_reference_sha256",
                "clean_reference_decoded_rgba_blake3",
                clean,
            );
        }

        fn add_target(
            &mut self,
            entry: usize,
            id: &str,
            roi: [u32; 4],
            foreground: (u32, u32),
            expected: &str,
        ) {
            let mask = mask_at(foreground.0, foreground.1);
            let erase = write_asset(&self.root, entry, &format!("{id}-erase"), &png(&mask));
            let residual = write_asset(&self.root, entry, &format!("{id}-residual"), &png(&mask));
            self.value["entries"][entry]["targets"]
                .as_array_mut()
                .unwrap()
                .push(target_json(id, roi, erase, residual, "plain", expected));
        }
    }

    fn target_json(
        id: &str,
        roi: [u32; 4],
        erase: (String, String),
        residual: (String, String),
        effect: &str,
        expected: &str,
    ) -> Value {
        json!({
            "id": id,
            "source_roi": roi,
            "clean_reference_edit_roi": roi,
            "erase_source_ink_mask_path": erase.0,
            "erase_source_ink_mask_sha256": erase.1,
            "residual_source_ink_mask_path": residual.0,
            "residual_source_ink_mask_sha256": residual.1,
            "position": "interior",
            "writing": "horizontal",
            "effect": effect,
            "translation_length": "short",
            "expected": expected
        })
    }

    fn target_origin(index: usize) -> (u32, u32) {
        (((index % 3) as u32) * 2, ((index / 3) as u32) * 2)
    }

    fn source_image(index: usize) -> RgbaImage {
        RgbaImage::from_fn(PAGE, PAGE, |x, y| {
            Rgba([
                20_u8.wrapping_add(index as u8),
                (x * 9) as u8,
                (y * 11) as u8,
                255,
            ])
        })
    }

    fn changed_pixel(pixel: &Rgba<u8>) -> Rgba<u8> {
        Rgba([pixel[0].wrapping_add(1), pixel[1], pixel[2], pixel[3]])
    }

    fn mask_at(x: u32, y: u32) -> RgbaImage {
        RgbaImage::from_fn(PAGE, PAGE, |px, py| {
            if (px, py) == (x, y) {
                Rgba([255, 255, 255, 255])
            } else {
                Rgba([0, 0, 0, 255])
            }
        })
    }

    fn write_asset(root: &Path, entry: usize, kind: &str, bytes: &[u8]) -> (String, String) {
        let path = root.join(format!("{entry}-{kind}.png"));
        fs::write(&path, bytes).unwrap();
        (path.to_str().unwrap().to_owned(), sha256(bytes))
    }

    fn replace_image(
        entry: &mut Value,
        path: &str,
        raw_hash: &str,
        decoded_hash: &str,
        image: &RgbaImage,
    ) {
        let bytes = png(image);
        fs::write(entry[path].as_str().unwrap(), &bytes).unwrap();
        entry[raw_hash] = sha256(&bytes).into();
        entry[decoded_hash] = blake3_hex(image).into();
    }

    fn png(image: &RgbaImage) -> Vec<u8> {
        let mut output = std::io::Cursor::new(Vec::new());
        DynamicImage::ImageRgba8(image.clone())
            .write_to(&mut output, ImageFormat::Png)
            .unwrap();
        output.into_inner()
    }

    fn decode(path: &str) -> RgbaImage {
        image::load_from_memory(&fs::read(path).unwrap())
            .unwrap()
            .into_rgba8()
    }

    fn sha256(bytes: &[u8]) -> String {
        format!("{:x}", Sha256::digest(bytes))
    }

    fn blake3_hex(image: &RgbaImage) -> String {
        blake3::hash(image.as_raw()).to_hex().to_string()
    }

    fn expect_error<T>(result: OracleResult<T>, category: &'static str) -> D0OracleError {
        let error = match result {
            Ok(_) => panic!("expected oracle validation rejection"),
            Err(error) => error,
        };
        assert_eq!(error.category(), category);
        assert!(error.source().is_some());
        error
    }

    #[test]
    fn d0_visual_manifest_oracles_accepts_valid_nine_and_retains_local_delta() {
        let fixture = Fixture::new();
        let validated = fixture.validate().unwrap();
        assert_eq!(validated.entries.len(), 9);
        assert!(validated.entries[0].protected_rois.is_empty());
        assert_eq!(&*validated.entries[0].targets[0].delta_mask, &[1, 0, 0, 0]);
        assert_eq!(
            validated.final_oracle_retained_bytes,
            validated.upstream.final_retained_bytes + 36
        );
        assert!(!validated.upstream.held_schema.manifest.bytes().is_empty());
    }

    #[test]
    fn d0_visual_manifest_oracles_rejects_invalid_rectangles() {
        for (field, raw) in [
            ("source", json!([0, 0, 0, 1])),
            ("source", json!([2, 0, 1, 1])),
            ("source", json!([0, 0, 7, 1])),
            ("source", json!([0, 0, u64::MAX, 1])),
            ("edit", json!([0, 0, 0, 1])),
            ("edit", json!([2, 0, 1, 1])),
            ("edit", json!([0, 0, 7, 1])),
            ("edit", json!([0, 0, u64::MAX, 1])),
            ("protected", json!([0, 0, 0, 1])),
            ("protected", json!([2, 0, 1, 1])),
            ("protected", json!([0, 0, 7, 1])),
            ("protected", json!([0, 0, u64::MAX, 1])),
        ] {
            let mut fixture = Fixture::new();
            match field {
                "source" => fixture.value["entries"][8]["targets"][0]["source_roi"] = raw,
                "edit" => {
                    fixture.value["entries"][8]["targets"][0]["clean_reference_edit_roi"] = raw
                }
                "protected" => fixture.value["entries"][8]["protected_rois"] = json!([raw]),
                _ => unreachable!(),
            }
            expect_error(fixture.validate(), GEOMETRY_ERROR);
        }
    }

    #[test]
    fn d0_visual_manifest_oracles_rejects_edit_protected_and_implied_mask_overlap() {
        for protected in [false, true] {
            let mut fixture = Fixture::new();
            if protected {
                fixture.value["entries"][0]["protected_rois"] = json!([[1, 1, 3, 3]]);
            } else {
                fixture.add_target(0, "overlap", [0, 0, 1, 1], (0, 0), "manual_override");
            }
            expect_error(fixture.validate(), DISJOINT_ERROR);
        }
    }

    #[test]
    fn d0_visual_manifest_oracles_rejects_mask_outside_edit() {
        let mut fixture = Fixture::new();
        fixture.value["entries"][0]["targets"][0]["clean_reference_edit_roi"] = json!([1, 1, 2, 2]);
        expect_error(fixture.validate(), MASK_GEOMETRY_ERROR);
    }

    #[test]
    fn d0_visual_manifest_oracles_rejects_successful_empty_or_incomplete_delta() {
        for delta_elsewhere in [false, true] {
            let mut fixture = Fixture::new();
            let mut clean = fixture.source(0);
            if delta_elsewhere {
                clean.put_pixel(1, 1, changed_pixel(clean.get_pixel(1, 1)));
            }
            fixture.set_clean(0, &clean);
            expect_error(fixture.validate(), SUCCESS_DELTA_ERROR);
        }
    }

    #[test]
    fn d0_visual_manifest_oracles_rejects_outside_and_protected_clean_changes() {
        for protected in [false, true] {
            let mut fixture = Fixture::new();
            let mut clean = decode(
                fixture.value["entries"][0]["clean_reference_path"]
                    .as_str()
                    .unwrap(),
            );
            clean.put_pixel(5, 5, changed_pixel(clean.get_pixel(5, 5)));
            fixture.set_clean(0, &clean);
            if protected {
                fixture.value["entries"][0]["protected_rois"] = json!([[4, 4, 6, 6]]);
            }
            expect_error(
                fixture.validate(),
                if protected {
                    PROTECTED_EQUALITY_ERROR
                } else {
                    OUTSIDE_SUCCESS_ERROR
                },
            );
        }
    }

    #[test]
    fn d0_visual_manifest_oracles_rejects_unsupported_edit_changes() {
        for expected in ["unsupported_source_color", "unsupported_rotation"] {
            let mut fixture = Fixture::new();
            let mut clean = fixture.source(0);
            clean.put_pixel(1, 1, changed_pixel(clean.get_pixel(1, 1)));
            fixture.set_clean(0, &clean);
            fixture.value["entries"][0]["targets"][0]["expected"] = expected.into();
            expect_error(fixture.validate(), UNSUPPORTED_EQUALITY_ERROR);
        }
    }

    #[test]
    fn d0_visual_manifest_oracles_preserves_row_major_target_alignment() {
        let mut fixture = Fixture::new();
        fixture.add_target(0, "non-square", [2, 1, 5, 3], (2, 1), "manual_override");
        let mut clean = fixture.source(0);
        for (x, y) in [(0, 0), (2, 1), (4, 1), (3, 2)] {
            clean.put_pixel(x, y, changed_pixel(clean.get_pixel(x, y)));
        }
        fixture.set_clean(0, &clean);

        let validated = fixture.validate().unwrap();
        let targets = &validated.entries[0].targets;
        assert_eq!(targets[0].source_roi, rect(0, 0, 2, 2));
        assert_eq!(targets[0].edit_roi, rect(0, 0, 2, 2));
        assert_eq!(&*targets[0].delta_mask, &[1, 0, 0, 0]);
        assert_eq!(targets[1].source_roi, rect(2, 1, 5, 3));
        assert_eq!(targets[1].edit_roi, rect(2, 1, 5, 3));
        assert_eq!(&*targets[1].delta_mask, &[1, 0, 1, 0, 1, 0]);
    }

    #[test]
    fn d0_visual_manifest_oracles_accepts_mixed_successful_and_unsupported_targets() {
        let mut fixture = Fixture::new();
        fixture.add_target(
            0,
            "unsupported",
            [4, 4, 6, 6],
            (5, 5),
            "unsupported_rotation",
        );
        let validated = fixture.validate().unwrap();
        assert_eq!(
            validated.entries[0].targets[1]
                .delta_mask
                .iter()
                .sum::<u8>(),
            0
        );
    }

    #[test]
    fn d0_visual_manifest_oracles_checks_geometry_and_disjoint_before_pixels() {
        let mut geometry = Fixture::new();
        let source = geometry.source(0);
        geometry.set_clean(0, &source);
        geometry.value["entries"][8]["targets"][0]["source_roi"] = json!([0, 0, 0, 1]);
        expect_error(geometry.validate(), GEOMETRY_ERROR);

        let mut disjoint = Fixture::new();
        let source = disjoint.source(0);
        disjoint.set_clean(0, &source);
        disjoint.add_target(8, "overlap", [3, 3, 5, 5], (3, 3), "manual_override");
        expect_error(disjoint.validate(), DISJOINT_ERROR);

        let mut semantic = Fixture::new();
        let mut clean = decode(
            semantic.value["entries"][0]["clean_reference_path"]
                .as_str()
                .unwrap(),
        );
        clean.put_pixel(5, 5, changed_pixel(clean.get_pixel(5, 5)));
        semantic.set_clean(0, &clean);
        semantic.value["entries"][8]["targets"][0]["expected"] = "unsupported_rotation".into();
        expect_error(semantic.validate(), UNSUPPORTED_EQUALITY_ERROR);
    }

    #[test]
    fn d0_visual_manifest_oracles_checked_arithmetic_does_not_allocate() {
        assert!(checked_area(u64::MAX, 2).is_err());
        assert!(checked_oracle_preflight(u64::MAX, 1, 0, BYTE_CEILING).is_err());
        assert!(checked_oracle_preflight(BYTE_CEILING, 0, 1, BYTE_CEILING).is_err());
        assert_eq!(
            checked_oracle_preflight(BYTE_CEILING - 1, 0, 1, BYTE_CEILING).unwrap(),
            (BYTE_CEILING - 1, 1)
        );
        assert!(checked_page_index(u32::MAX, 0, u32::MAX, 1).is_err());
    }

    #[test]
    fn d0_visual_manifest_oracles_real_budget_exactly_admits_fixture() {
        let mut fixture = Fixture::new();
        let upstream = fixture.load_pixels();
        let exact = fixture_requirement(&upstream);
        let validated = validate_visual_oracles_with_limit(upstream, exact).unwrap();
        assert_eq!(
            validated.final_oracle_retained_bytes + u64::from(PAGE * PAGE),
            exact
        );

        let source = fixture.source(0);
        fixture.set_clean(0, &source);
        expect_error(
            validate_visual_oracles_with_limit(fixture.load_pixels(), exact - 1),
            BUDGET_ERROR,
        );
    }

    fn rect(left: u32, top: u32, right: u32, bottom: u32) -> ValidatedHalfOpenRect {
        ValidatedHalfOpenRect {
            left,
            top,
            right,
            bottom,
        }
    }

    fn fixture_requirement(upstream: &DimensionAndMaskValidatedManifest) -> u64 {
        let delta = upstream
            .held_schema
            .schema
            .entries
            .iter()
            .flat_map(|entry| &entry.targets)
            .map(|target| {
                let [left, top, right, bottom] = target.clean_reference_edit_roi;
                (right - left) * (bottom - top)
            })
            .sum::<u64>();
        let workspace = upstream
            .entries
            .iter()
            .map(|entry| u64::from(entry.source.width()) * u64::from(entry.source.height()))
            .max()
            .unwrap();
        upstream.final_retained_bytes + delta + workspace
    }
}
