use koharu_ml::pp_ocr_v5::PpOcrWordBox;

use crate::pipeline::engines::support::{contains_han, contains_protected_latin_word};

const MIN_WORD_CONFIDENCE: f32 = 0.5;

#[derive(Clone, Debug, PartialEq)]
pub(super) struct ValidatedWord {
    pub(super) line_index: usize,
    pub(super) text: String,
    pub(super) bbox: [f32; 4],
    pub(super) protected: bool,
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct SourceTarget {
    pub(super) text: String,
    pub(super) bbox: [f32; 4],
    pub(super) line_polygon: [[f32; 2]; 4],
}

#[derive(Clone, Debug, PartialEq)]
pub(super) struct SourceSelection {
    pub(super) targets: Vec<SourceTarget>,
    pub(super) protected_lines: Vec<(String, [f32; 4])>,
}

pub(super) fn pp_may_contain_han(words: &[PpOcrWordBox]) -> bool {
    words.iter().any(|word| contains_han(&word.text))
}

fn bbox_quad([left, top, right, bottom]: [f32; 4]) -> [[f32; 2]; 4] {
    [[left, top], [right, top], [right, bottom], [left, bottom]]
}

fn bbox_union(words: &[ValidatedWord], indices: &[usize]) -> Option<[f32; 4]> {
    let first = words.get(*indices.first()?)?.bbox;
    Some(indices.iter().skip(1).fold(first, |mut bbox, index| {
        let item = words[*index].bbox;
        bbox[0] = bbox[0].min(item[0]);
        bbox[1] = bbox[1].min(item[1]);
        bbox[2] = bbox[2].max(item[2]);
        bbox[3] = bbox[3].max(item[3]);
        bbox
    }))
}

fn bboxes_intersect(a: [f32; 4], b: [f32; 4]) -> bool {
    a[0].max(b[0]) < a[2].min(b[2]) && a[1].max(b[1]) < a[3].min(b[3])
}

pub(super) fn validate_pp_vl_alignment(
    vl_text: &str,
    words: &[PpOcrWordBox],
    crop_bounds: [u32; 4],
    image_width: u32,
    image_height: u32,
) -> Option<Vec<ValidatedWord>> {
    let [crop_left, crop_top, crop_right, crop_bottom] = crop_bounds;
    if crop_left >= crop_right
        || crop_top >= crop_bottom
        || crop_right > image_width
        || crop_bottom > image_height
        || words.is_empty()
    {
        return None;
    }

    let vl_chars = vl_text
        .chars()
        .filter(|ch| !ch.is_whitespace())
        .collect::<Vec<_>>();
    let crop_width = (crop_right - crop_left) as f32;
    let crop_height = (crop_bottom - crop_top) as f32;
    let mut validated = Vec::with_capacity(words.len());
    let mut vl_offset = 0_usize;
    let mut previous_line = None;
    let mut previous_right = 0.0;

    for word in words {
        let pp_chars = word
            .text
            .chars()
            .filter(|ch| !ch.is_whitespace())
            .collect::<Vec<_>>();
        let end = vl_offset.checked_add(pp_chars.len())?;
        let authoritative = vl_chars.get(vl_offset..end)?;
        if pp_chars.is_empty()
            || !word.confidence.is_finite()
            || word.confidence < MIN_WORD_CONFIDENCE
            || !pp_chars.iter().zip(authoritative).all(|(pp, vl)| {
                pp == vl || (contains_han(&pp.to_string()) && contains_han(&vl.to_string()))
            })
        {
            return None;
        }

        let [left, top, right, bottom] = word.bbox;
        if word.bbox.iter().any(|value| !value.is_finite())
            || left < 0.0
            || top < 0.0
            || right > crop_width
            || bottom > crop_height
            || left >= right
            || top >= bottom
        {
            return None;
        }
        if let Some(line_index) = previous_line
            && (word.line_index < line_index
                || (word.line_index == line_index && left < previous_right))
        {
            return None;
        }
        previous_line = Some(word.line_index);
        previous_right = right;
        vl_offset = end;

        let text = authoritative.iter().collect::<String>();
        let protected = contains_protected_latin_word(&text);
        if protected && contains_han(&text) {
            return None;
        }
        validated.push(ValidatedWord {
            line_index: word.line_index,
            text,
            bbox: [
                crop_left as f32 + left,
                crop_top as f32 + top,
                crop_left as f32 + right,
                crop_top as f32 + bottom,
            ],
            protected,
        });
    }

    (vl_offset == vl_chars.len()).then_some(validated)
}

pub(super) fn select_chinese_target(
    vl_text: &str,
    words: &[PpOcrWordBox],
    crop_bounds: [u32; 4],
    image_width: u32,
    image_height: u32,
) -> Option<SourceSelection> {
    let validated =
        validate_pp_vl_alignment(vl_text, words, crop_bounds, image_width, image_height)?;
    let mut selected = vec![false; validated.len()];
    let mut indexed_targets = Vec::new();
    let mut line_start = 0;

    while line_start < validated.len() {
        let line_index = validated[line_start].line_index;
        let mut line_end = line_start + 1;
        while line_end < validated.len() && validated[line_end].line_index == line_index {
            line_end += 1;
        }
        let line = &validated[line_start..line_end];
        if line.iter().any(|word| contains_han(&word.text)) {
            let target_indices = if line.iter().all(|word| !word.protected) {
                (line_start..line_end).collect::<Vec<_>>()
            } else {
                let mut han_runs = Vec::new();
                let mut run_start = line_start;
                while run_start < line_end {
                    while run_start < line_end && validated[run_start].protected {
                        run_start += 1;
                    }
                    if run_start == line_end {
                        break;
                    }
                    let mut run_end = run_start + 1;
                    while run_end < line_end && !validated[run_end].protected {
                        run_end += 1;
                    }
                    if validated[run_start..run_end]
                        .iter()
                        .any(|word| contains_han(&word.text))
                    {
                        han_runs.push((run_start..run_end).collect::<Vec<_>>());
                    }
                    run_start = run_end;
                }
                if han_runs.len() != 1 {
                    return None;
                }
                han_runs.pop()?
            };
            let bbox = bbox_union(&validated, &target_indices)?;
            let text = target_indices
                .iter()
                .map(|index| validated[*index].text.as_str())
                .collect::<String>();
            if text.is_empty() || !contains_han(&text) {
                return None;
            }
            for index in &target_indices {
                selected[*index] = true;
            }
            indexed_targets.push((
                line_index,
                bbox[0],
                SourceTarget {
                    text,
                    bbox,
                    line_polygon: bbox_quad(bbox),
                },
            ));
        }
        line_start = line_end;
    }

    if indexed_targets.is_empty() {
        return None;
    }
    let protected_lines = validated
        .iter()
        .enumerate()
        .filter(|(index, _)| !selected[*index])
        .map(|(_, word)| (word.text.clone(), word.bbox))
        .collect::<Vec<_>>();
    if indexed_targets.iter().any(|(_, _, target)| {
        protected_lines
            .iter()
            .any(|(_, bbox)| bboxes_intersect(target.bbox, *bbox))
    }) {
        return None;
    }

    indexed_targets.sort_by(|a, b| {
        a.0.cmp(&b.0)
            .then_with(|| a.1.partial_cmp(&b.1).unwrap_or(std::cmp::Ordering::Equal))
    });
    Some(SourceSelection {
        targets: indexed_targets
            .into_iter()
            .map(|(_, _, target)| target)
            .collect(),
        protected_lines,
    })
}

#[cfg(test)]
mod tests {
    use koharu_ml::pp_ocr_v5::PpOcrWordBox;

    use super::*;

    fn word(
        text: &str,
        line_index: usize,
        left: f32,
        top: f32,
        right: f32,
        bottom: f32,
    ) -> PpOcrWordBox {
        PpOcrWordBox {
            line_index,
            text: text.into(),
            bbox: [left, top, right, bottom],
            confidence: 0.9,
        }
    }

    #[test]
    fn gate_pp_prefilter_rejects_pure_english_without_vl() {
        let words = [
            word("SLENDER", 0, 0.0, 0.0, 40.0, 20.0),
            word("WAIST", 0, 45.0, 0.0, 80.0, 20.0),
        ];
        assert!(!pp_may_contain_han(&words));
    }

    #[test]
    fn gate_vl_validation_keeps_same_line_single_label_and_excludes_other_lines() {
        let same_line = select_chinese_target(
            "S型曲线",
            &[
                word("S", 0, 0.0, 0.0, 8.0, 20.0),
                word("型曲线", 0, 8.0, 0.0, 40.0, 20.0),
            ],
            [10, 20, 110, 70],
            200,
            100,
        )
        .unwrap();
        assert_eq!(same_line.targets.len(), 1);
        assert_eq!(same_line.targets[0].text, "S型曲线");
        assert_eq!(same_line.targets[0].bbox, [10.0, 20.0, 50.0, 40.0]);

        let other_line = select_chinese_target(
            "S\n中文",
            &[
                word("S", 0, 0.0, 0.0, 8.0, 10.0),
                word("中文", 1, 10.0, 15.0, 40.0, 35.0),
            ],
            [10, 20, 110, 70],
            200,
            100,
        )
        .unwrap();
        assert_eq!(other_line.targets.len(), 1);
        assert_eq!(other_line.targets[0].text, "中文");
        assert_eq!(other_line.targets[0].bbox, [20.0, 35.0, 50.0, 55.0]);
        assert_eq!(
            other_line.protected_lines,
            vec![("S".into(), [10.0, 20.0, 18.0, 30.0])]
        );
    }

    #[test]
    fn gate_vl_validation_keeps_only_han_beside_complete_english() {
        let target = select_chinese_target(
            "Peach蜜桃臀",
            &[
                word("Peach", 0, 0.0, 0.0, 40.0, 20.0),
                word("蜜桃臀", 0, 45.0, 0.0, 100.0, 20.0),
            ],
            [10, 20, 110, 70],
            200,
            100,
        )
        .unwrap();
        assert_eq!(target.targets.len(), 1);
        assert_eq!(target.targets[0].text, "蜜桃臀");
        assert_eq!(target.targets[0].bbox, [55.0, 20.0, 110.0, 40.0]);
        assert_eq!(
            target.protected_lines,
            vec![("Peach".into(), [10.0, 20.0, 50.0, 40.0])]
        );
    }

    #[test]
    fn gate_vl_validation_rejects_mismatch_unseparated_and_invalid_geometry() {
        assert!(
            select_chinese_target(
                "Peach蜜桃臀",
                &[
                    word("Beach", 0, 0.0, 0.0, 40.0, 20.0),
                    word("蜜桃臀", 0, 45.0, 0.0, 100.0, 20.0),
                ],
                [0, 0, 100, 50],
                100,
                50,
            )
            .is_none()
        );
        assert!(
            select_chinese_target(
                "AI智能塑形",
                &[word("AI智能塑形", 0, 0.0, 0.0, 100.0, 20.0)],
                [0, 0, 100, 50],
                100,
                50,
            )
            .is_none()
        );
        assert!(
            select_chinese_target(
                "中文",
                &[word("中文", 0, f32::NAN, 0.0, 40.0, 20.0)],
                [0, 0, 100, 50],
                100,
                50,
            )
            .is_none()
        );
    }

    #[test]
    fn gate_keeps_protected_runs_disjoint_on_both_sides_of_han() {
        let selection = select_chinese_target(
            "Slim蜜桃臀Fit",
            &[
                word("Slim", 0, 0.0, 0.0, 25.0, 20.0),
                word("蜜桃臀", 0, 30.0, 0.0, 65.0, 20.0),
                word("Fit", 0, 70.0, 0.0, 95.0, 20.0),
            ],
            [10, 20, 110, 70],
            200,
            100,
        )
        .unwrap();
        assert_eq!(selection.targets[0].bbox, [40.0, 20.0, 75.0, 40.0]);
        assert_eq!(selection.protected_lines.len(), 2);
        assert!(selection.protected_lines.iter().all(|(_, bbox)| {
            bbox[2] <= selection.targets[0].bbox[0] || bbox[0] >= selection.targets[0].bbox[2]
        }));
    }

    #[test]
    fn gate_splits_han_lines_around_an_english_line() {
        let selection = select_chinese_target(
            "中文一\nEnglish\n中文二",
            &[
                word("中文一", 0, 0.0, 0.0, 40.0, 15.0),
                word("English", 1, 0.0, 20.0, 60.0, 35.0),
                word("中文二", 2, 0.0, 40.0, 40.0, 55.0),
            ],
            [10, 20, 110, 90],
            200,
            120,
        )
        .unwrap();
        assert_eq!(selection.targets.len(), 2);
        assert_eq!(selection.targets[0].bbox, [10.0, 20.0, 50.0, 35.0]);
        assert_eq!(selection.targets[1].bbox, [10.0, 60.0, 50.0, 75.0]);
        assert_eq!(selection.protected_lines.len(), 1);
    }
}
