use std::collections::{BTreeMap, VecDeque};

use anyhow::{Context, Result, ensure};
use image::DynamicImage;
use koharu_runtime::RuntimeManager;
use ocrs_cjk::{ImageSource, OcrEngine, OcrEngineParams, TextItem};

const HF_REPO: &str = "marsena/paddleocr-onnx-models";
const DETECTION_MODEL: &str = "PP-OCRv5_server_det_infer.onnx";
const RECOGNITION_MODEL: &str = "PP-OCRv5_server_rec_infer.onnx";
const RECOGNITION_CONFIG: &str = "PP-OCRv5_server_rec_infer.yml";
const SMALL_CANDIDATE_HEIGHT: u32 = 160;
const MAX_SOURCE_DIMENSION_FOR_UPSCALE: u32 = 1024;

koharu_runtime::declare_hf_model_package!(
    id: "model:pp-ocr-v5:detector",
    repo: HF_REPO,
    file: DETECTION_MODEL,
    bootstrap: false,
    order: 220,
);
koharu_runtime::declare_hf_model_package!(
    id: "model:pp-ocr-v5:recognizer",
    repo: HF_REPO,
    file: RECOGNITION_MODEL,
    bootstrap: false,
    order: 221,
);
koharu_runtime::declare_hf_model_package!(
    id: "model:pp-ocr-v5:dictionary",
    repo: HF_REPO,
    file: RECOGNITION_CONFIG,
    bootstrap: false,
    order: 222,
);

#[derive(Debug, Clone, PartialEq)]
pub struct PpOcrWordBox {
    pub line_index: usize,
    pub text: String,
    /// Axis-aligned crop-local `[left, top, right, bottom]` coordinates.
    pub bbox: [f32; 4],
    pub confidence: f32,
}

#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub struct PpOcrDetectorOccurrence {
    pub occurrence_index: usize,
    /// Crop-local source-space corners in the detector's original order.
    pub corners: [[f32; 2]; 4],
}

#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub struct PpOcrLineObservation {
    pub detector_indices: Vec<usize>,
    pub recognition: Option<String>,
}

#[doc(hidden)]
#[derive(Debug, Clone, PartialEq)]
pub struct PpOcrV5Observation {
    pub detectors: Vec<PpOcrDetectorOccurrence>,
    pub lines: Vec<PpOcrLineObservation>,
    pub word_boxes: Vec<PpOcrWordBox>,
}

impl PpOcrV5Observation {
    pub fn word_boxes(&self) -> &[PpOcrWordBox] {
        &self.word_boxes
    }
}

pub struct PpOcrV5 {
    engine: OcrEngine,
}

fn word_box_inference_scale(width: u32, height: u32) -> u32 {
    if height < SMALL_CANDIDATE_HEIGHT && width.max(height) <= MAX_SOURCE_DIMENSION_FOR_UPSCALE {
        2
    } else {
        1
    }
}

fn word_box_source_bbox(bbox: [f32; 4], scale: u32) -> [f32; 4] {
    let scale = scale.max(1) as f32;
    [
        (bbox[0] / scale).floor(),
        (bbox[1] / scale).floor(),
        (bbox[2] / scale).ceil(),
        (bbox[3] / scale).ceil(),
    ]
}

fn source_corners(corners: [[f32; 2]; 4], scale: u32) -> [[f32; 2]; 4] {
    let scale = scale.max(1) as f32;
    corners.map(|[x, y]| [x / scale, y / scale])
}

fn corner_bits(corners: [[f32; 2]; 4]) -> [u32; 8] {
    let mut bits = [0; 8];
    for (index, [x, y]) in corners.into_iter().enumerate() {
        bits[index * 2] = x.to_bits();
        bits[index * 2 + 1] = y.to_bits();
    }
    bits
}

fn canonicalize_detection_topology(
    detector_corners: Vec<[[f32; 2]; 4]>,
    line_corners: Vec<Vec<[[f32; 2]; 4]>>,
) -> Result<(Vec<PpOcrDetectorOccurrence>, Vec<Vec<usize>>)> {
    let detectors = detector_corners
        .iter()
        .enumerate()
        .map(|(occurrence_index, corners)| PpOcrDetectorOccurrence {
            occurrence_index,
            corners: *corners,
        })
        .collect::<Vec<_>>();
    let mut available = BTreeMap::<[u32; 8], VecDeque<usize>>::new();
    for detector in &detectors {
        available
            .entry(corner_bits(detector.corners))
            .or_default()
            .push_back(detector.occurrence_index);
    }
    let mut canonical_lines = Vec::with_capacity(line_corners.len());
    for line in line_corners {
        let mut detector_indices = Vec::with_capacity(line.len());
        for corners in line {
            let occurrence_index = available
                .get_mut(&corner_bits(corners))
                .and_then(VecDeque::pop_front)
                .context("canonical PP-OCRv5 line contains an unknown detector occurrence")?;
            detector_indices.push(occurrence_index);
        }
        canonical_lines.push(detector_indices);
    }
    ensure!(
        available.values().all(VecDeque::is_empty),
        "PP-OCRv5 canonical lines omit detector occurrences"
    );
    Ok((detectors, canonical_lines))
}

impl PpOcrV5 {
    pub async fn load(runtime: &RuntimeManager) -> Result<Self> {
        let downloads = runtime.downloads();
        let (detection_path, recognition_path, config_path) = tokio::try_join!(
            downloads.huggingface_model(HF_REPO, DETECTION_MODEL),
            downloads.huggingface_model(HF_REPO, RECOGNITION_MODEL),
            downloads.huggingface_model(HF_REPO, RECOGNITION_CONFIG),
        )?;
        let config = tokio::fs::read_to_string(&config_path)
            .await
            .with_context(|| format!("failed to read `{}`", config_path.display()))?;
        let alphabet_chars = parse_character_dict(&config)?;

        let engine = tokio::task::spawn_blocking(move || -> Result<OcrEngine> {
            let detection_model = rten::Model::load_file(&detection_path)
                .with_context(|| format!("failed to load `{}`", detection_path.display()))?;
            let recognition_model = rten::Model::load_file(&recognition_path)
                .with_context(|| format!("failed to load `{}`", recognition_path.display()))?;
            OcrEngine::new(OcrEngineParams {
                detection_model: Some(detection_model),
                recognition_model: Some(recognition_model),
                alphabet: Some(alphabet_chars.into_iter().collect()),
                ..Default::default()
            })
            .context("failed to initialize PP-OCRv5")
        })
        .await
        .context("failed to join PP-OCRv5 loading task")??;

        Ok(Self { engine })
    }

    pub fn word_boxes(&self, image: &DynamicImage) -> Result<Vec<PpOcrWordBox>> {
        Ok(self.observe(image)?.word_boxes)
    }

    #[doc(hidden)]
    pub fn observe(&self, image: &DynamicImage) -> Result<PpOcrV5Observation> {
        let (width, height) = (image.width(), image.height());
        if width == 0 || height == 0 {
            return Ok(PpOcrV5Observation {
                detectors: Vec::new(),
                lines: Vec::new(),
                word_boxes: Vec::new(),
            });
        }
        let scale = word_box_inference_scale(width, height);
        let resized = (scale > 1).then(|| {
            image.resize_exact(
                width * scale,
                height * scale,
                image::imageops::FilterType::CatmullRom,
            )
        });
        let rgb = resized.as_ref().unwrap_or(image).to_rgb8();
        let inference_size = rgb.dimensions();

        let source = ImageSource::from_bytes(rgb.as_raw(), inference_size)?;
        let input = self.engine.prepare_input(source)?;
        let detected = self.engine.detect_words(&input)?;
        if detected.is_empty() {
            return Ok(PpOcrV5Observation {
                detectors: Vec::new(),
                lines: Vec::new(),
                word_boxes: Vec::new(),
            });
        }
        let lines = self.engine.find_text_lines(&input, &detected);
        let recognized = self.engine.recognize_text(&input, &lines)?;
        ensure!(
            recognized.len() == lines.len(),
            "PP-OCRv5 recognition count differs from canonical line count"
        );

        let detector_corners = detected
            .iter()
            .map(|detector| {
                source_corners(detector.corners().map(|point| [point.x, point.y]), scale)
            })
            .collect::<Vec<_>>();
        let line_corners = lines
            .iter()
            .map(|line| {
                line.iter()
                    .map(|detector| {
                        source_corners(detector.corners().map(|point| [point.x, point.y]), scale)
                    })
                    .collect::<Vec<_>>()
            })
            .collect::<Vec<_>>();
        let (detectors, canonical_lines) =
            canonicalize_detection_topology(detector_corners, line_corners)?;

        let mut words = Vec::new();
        let mut observed_lines = Vec::with_capacity(lines.len());
        for (line_index, (detector_indices, recognition)) in
            canonical_lines.into_iter().zip(recognized).enumerate()
        {
            let recognition_text = recognition.as_ref().map(ToString::to_string);
            if let Some(line) = recognition {
                for segment in line.segments() {
                    let bbox = segment.bounding_rect();
                    let bbox = word_box_source_bbox(
                        [
                            bbox.left() as f32,
                            bbox.top() as f32,
                            bbox.right() as f32,
                            bbox.bottom() as f32,
                        ],
                        scale,
                    );
                    let confidence = segment.confidence();
                    if bbox[0] < bbox[2]
                        && bbox[1] < bbox[3]
                        && bbox[0] >= 0.0
                        && bbox[1] >= 0.0
                        && bbox[2] <= width as f32
                        && bbox[3] <= height as f32
                        && confidence.is_finite()
                    {
                        words.push(PpOcrWordBox {
                            line_index,
                            text: segment.to_string(),
                            bbox,
                            confidence,
                        });
                    }
                }
            }
            observed_lines.push(PpOcrLineObservation {
                detector_indices,
                recognition: recognition_text,
            });
        }
        Ok(PpOcrV5Observation {
            detectors,
            lines: observed_lines,
            word_boxes: words,
        })
    }
}

fn parse_character_dict(config: &str) -> Result<Vec<char>> {
    let mut in_dictionary = false;
    let mut alphabet = Vec::new();
    for line in config.lines() {
        if !in_dictionary {
            in_dictionary = line == "  character_dict:";
            continue;
        }
        let Some(raw) = line.strip_prefix("  - ") else {
            if !line.is_empty() {
                break;
            }
            continue;
        };
        let value = if raw.starts_with('\'') && raw.ends_with('\'') && raw.len() >= 2 {
            raw[1..raw.len() - 1].replace("''", "'")
        } else {
            raw.to_string()
        };
        let mut chars = value.chars();
        let first = chars.next().context("empty PP-OCRv5 dictionary item")?;
        // Paddle can label a grapheme with multiple scalars (regional flags), while
        // ocrs exposes scalar characters. Keep one placeholder so later label indices
        // remain aligned; ecommerce Chinese/Latin recognition never consumes it.
        alphabet.push(if chars.next().is_some() { '�' } else { first });
    }
    ensure!(!alphabet.is_empty(), "PP-OCRv5 character_dict is missing");
    alphabet.push(' ');
    Ok(alphabet)
}

#[cfg(test)]
mod tests {
    use super::{
        canonicalize_detection_topology, parse_character_dict, word_box_inference_scale,
        word_box_source_bbox,
    };

    fn rect(left: f32, top: f32, right: f32, bottom: f32) -> [[f32; 2]; 4] {
        [[left, top], [right, top], [right, bottom], [left, bottom]]
    }

    #[test]
    fn parses_ppocr_dictionary_without_shifting_multiscalar_labels() {
        let config = "PostProcess:\n  character_dict:\n  - 一\n  - ''''\n  - 🇨🇳\n";

        assert_eq!(
            parse_character_dict(config).unwrap(),
            ['一', '\'', '�', ' ']
        );
    }

    #[test]
    fn upscales_small_word_box_candidates_and_maps_boxes_back_to_source_pixels() {
        assert_eq!(word_box_inference_scale(423, 77), 2);
        assert_eq!(word_box_inference_scale(900, 300), 1);
        assert_eq!(
            word_box_source_bbox([58.0, 40.0, 474.0, 116.0], 2),
            [29.0, 20.0, 237.0, 58.0]
        );
    }

    #[test]
    fn observation_retains_raw_detector_order_and_duplicate_occurrences() {
        let first = rect(0.0, 0.0, 8.0, 10.0);
        let second = rect(12.0, 0.0, 20.0, 10.0);
        let lines = vec![vec![first, first], vec![second]];

        let forward =
            canonicalize_detection_topology(vec![first, second, first], lines.clone()).unwrap();
        let reversed = canonicalize_detection_topology(vec![second, first, first], lines).unwrap();

        assert_eq!(
            forward
                .0
                .iter()
                .map(|detector| detector.corners)
                .collect::<Vec<_>>(),
            vec![first, second, first]
        );
        assert_eq!(
            reversed
                .0
                .iter()
                .map(|detector| detector.corners)
                .collect::<Vec<_>>(),
            vec![second, first, first]
        );
        assert_eq!(forward.0.len(), 3);
        assert_eq!(forward.1, vec![vec![0, 2], vec![1]]);
        assert_eq!(reversed.1, vec![vec![1, 2], vec![0]]);
        assert!(canonicalize_detection_topology(vec![first, second], vec![vec![first]]).is_err());
    }

    #[test]
    fn hanonly_pre_b1_red_t2_crop_local_ppocr_contract() {
        const CROP_LOCAL_MIN_INFERENCE_HEIGHT: u32 = 320;
        const CROP_LOCAL_INFERENCE_PIXEL_BUDGET: u64 = 1024 * 1024;
        const CROP_LOCAL_MAX_SCALE: u32 = 4;

        fn crop_local_model_scale(width: u32, height: u32) -> u32 {
            let scale = CROP_LOCAL_MIN_INFERENCE_HEIGHT
                .div_ceil(height.max(1))
                .clamp(1, CROP_LOCAL_MAX_SCALE);
            (1..=scale)
                .rev()
                .find(|scale| {
                    u64::from(width)
                        .checked_mul(u64::from(height))
                        .and_then(|pixels| pixels.checked_mul(u64::from(*scale)))
                        .and_then(|pixels| pixels.checked_mul(u64::from(*scale)))
                        .is_some_and(|pixels| pixels <= CROP_LOCAL_INFERENCE_PIXEL_BUDGET)
                })
                .unwrap_or(1)
        }

        let mut violations = Vec::new();
        for factor in [0.5_f32, 1.0, 2.0, 4.0] {
            let width = (400.0 * factor) as u32;
            let height = (160.0 * factor) as u32;
            let expected = crop_local_model_scale(width, height);
            let actual = word_box_inference_scale(width, height);
            if actual != expected {
                violations.push(format!(
                    "{factor}x crop-local scale {width}x{height}: expected {expected}, got {actual}"
                ));
            }
        }
        for (label, width, height) in [
            ("former-height-below", 1024, 159),
            ("former-height-at", 1024, 160),
            ("former-width-at", 1024, 159),
            ("former-width-above", 1025, 159),
            ("pixel-budget", 1000, 300),
            ("checked-overflow", u32::MAX, u32::MAX),
        ] {
            let expected = crop_local_model_scale(width, height);
            let actual = word_box_inference_scale(width, height);
            if actual != expected {
                violations.push(format!(
                    "{label} crop-local scale {width}x{height}: expected {expected}, got {actual}"
                ));
            }
        }
        for (bbox, scale, source_size) in [
            ([0.5, 0.5, 1.0, 1.0], 1, [2.0, 2.0]),
            ([1.0, 1.0, 3.0, 3.0], 2, [2.0, 2.0]),
            ([2.0, 2.0, 6.0, 6.0], 4, [2.0, 2.0]),
            ([59.0, 41.0, 473.0, 115.0], 2, [300.0, 100.0]),
        ] {
            let scale_f32 = scale as f32;
            let expected = [
                (bbox[0] / scale_f32).floor(),
                (bbox[1] / scale_f32).floor(),
                (bbox[2] / scale_f32).ceil(),
                (bbox[3] / scale_f32).ceil(),
            ];
            let actual = word_box_source_bbox(bbox, scale);
            if actual != expected {
                violations.push(format!(
                    "half-open inverse {bbox:?} / {scale}: expected {expected:?}, got {actual:?}"
                ));
            }
            for axis in 0..4 {
                let source_extent = source_size[axis % 2];
                let inference_extent = source_extent * scale_f32;
                let normalized = bbox[axis] / inference_extent;
                let normalized_floor_or_ceil = actual[axis] / source_extent;
                let tolerance = 1.0 / source_extent;
                if (normalized - normalized_floor_or_ceil).abs() > tolerance {
                    violations.push(format!(
                        "normalized half-open coordinate changed beyond one source pixel at axis {axis}: {normalized} -> {normalized_floor_or_ceil}"
                    ));
                }
            }
        }
        assert!(
            violations.is_empty(),
            "G002 PP-OCR contract violations:\n{}",
            violations.join("\n")
        );
    }
}
