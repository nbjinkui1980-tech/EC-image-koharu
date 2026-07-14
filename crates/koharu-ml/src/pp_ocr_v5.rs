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
    bbox.map(|coordinate| coordinate / scale)
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
        let (width, height) = (image.width(), image.height());
        if width == 0 || height == 0 {
            return Ok(Vec::new());
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
            return Ok(Vec::new());
        }
        let lines = self.engine.find_text_lines(&input, &detected);
        let recognized = self.engine.recognize_text(&input, &lines)?;
        let mut words = Vec::new();
        for (line_index, line) in recognized.into_iter().enumerate() {
            let Some(line) = line else { continue };
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
        Ok(words)
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
    use super::{parse_character_dict, word_box_inference_scale, word_box_source_bbox};

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
}
