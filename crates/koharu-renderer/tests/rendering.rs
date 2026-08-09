use std::{path::PathBuf, sync::OnceLock};

use anyhow::Result;
use koharu_renderer::{
    font::{Font, FontBook},
    layout::{LayoutRun, TextLayout, WritingMode},
    renderer::{RasterOptions, RenderOptions, RenderStrokeOptions, TinySkiaRenderer},
};
use unicode_bidi::BidiInfo;

const SAMPLE_TEXT: &str = "吾輩は猫である。名前はまだ無い。どこで生れたかとんと見当がつかぬ。何でも薄暗いじめじめした所でニャーニャー泣いていた事だけは記憶している。吾輩はここで始めて人間というものを見た。しかもあとで聞くとそれは書生という人間中で一番獰悪な種族であったそうだ。";
const SAMPLE_TEXT_ZH_CN: &str = "《我是猫》是日本作家夏目漱石创作的长篇小说，也是其代表作，它确立了夏目漱石在文学史上的地位。作品淋漓尽致地反映了二十世纪初，日本中小资产阶级的思想和生活，尖锐地揭露和批判了明治“文明开化”的资本主义社会。小说采用幽默、讽刺、滑稽的手法，借助一只猫的视觉、听觉、感觉，嘲笑了明治时代知识分子空虚的精神生活，小说构思奇巧，描写夸张，结构灵活，具有鲜明的艺术特色。";

fn output_dir() -> PathBuf {
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("target")
        .join("tests");
    let _ = std::fs::create_dir_all(&path);
    path
}

fn font(family_name: &str) -> Result<Font> {
    let mut book = FontBook::new();
    let post_script_name = book
        .all_families()
        .into_iter()
        .find(|face| {
            face.post_script_name == family_name
                || face
                    .families
                    .iter()
                    .any(|(family, _)| family.as_str() == family_name)
        })
        .map(|face| face.post_script_name)
        .filter(|post_script_name| !post_script_name.is_empty())
        .ok_or_else(|| anyhow::anyhow!("font not found: {family_name}"))?;
    let font = book.query(&post_script_name)?;
    // preload fontdue font
    let _ = font.fontdue()?;

    Ok(font)
}

/// Returns `None` when the font is unavailable on this system.
/// Falls back to CI-friendly alternatives (Noto CJK) for Japanese,
/// Chinese, and Korean fonts that are not installed by default on
/// Linux or macOS CI runners.
fn try_font(family_name: &str) -> Option<Font> {
    if let Ok(f) = font(family_name) {
        return Some(f);
    }
    // Fall back to system-native CJK fonts when the requested font
    // is not installed (CI Linux, bare macOS without Office).
    match family_name {
        "Yu Gothic" => font("Noto Sans CJK JP").ok(),
        "Microsoft YaHei" => font("Noto Sans CJK SC")
            .or_else(|_| font("PingFang SC"))
            .or_else(|_| font("Heiti SC"))
            .ok(),
        _ => None,
    }
}

fn tiny_skia_renderer() -> Result<&'static TinySkiaRenderer> {
    static INSTANCE: OnceLock<Result<TinySkiaRenderer, String>> = OnceLock::new();
    match INSTANCE.get_or_init(|| TinySkiaRenderer::new().map_err(|error| error.to_string())) {
        Ok(renderer) => Ok(renderer),
        Err(error) => Err(anyhow::anyhow!(error.clone())),
    }
}

fn non_bg_y_bounds(img: &image::RgbaImage, bg: [u8; 4]) -> Option<(u32, u32)> {
    let mut min_y = u32::MAX;
    let mut max_y = 0u32;
    let mut any = false;

    for (x, y, p) in img.enumerate_pixels() {
        let _ = x;
        if p.0 != bg {
            any = true;
            min_y = min_y.min(y);
            max_y = max_y.max(y);
        }
    }

    any.then_some((min_y, max_y))
}

#[test]
fn render_horizontal() -> Result<()> {
    let Some(font) = try_font("Yu Gothic") else {
        return Ok(());
    };
    let lines = TextLayout::new(&font, Some(24.0))
        .with_max_width(1000.0)
        .run(SAMPLE_TEXT)?;

    let img = tiny_skia_renderer()?.render(
        &lines,
        WritingMode::Horizontal,
        &RenderOptions {
            font_size: 24.0,
            padding: 0.0,
            background: Some([255, 255, 255, 255]),
            ..Default::default()
        },
    )?;

    assert!(img.pixels().any(|p| p.0 != [255, 255, 255, 255]));
    img.save(output_dir().join("horizontal.png"))?;
    Ok(())
}

#[test]
fn render_vertical() -> Result<()> {
    let Some(font) = try_font("Yu Gothic") else {
        return Ok(());
    };
    let lines = TextLayout::new(&font, Some(24.0))
        .with_writing_mode(WritingMode::VerticalRl)
        .with_max_height(1000.0)
        .run(SAMPLE_TEXT)?;

    let img = tiny_skia_renderer()?.render(
        &lines,
        WritingMode::VerticalRl,
        &RenderOptions {
            font_size: 24.0,
            padding: 0.0,
            background: Some([255, 255, 255, 255]),
            ..Default::default()
        },
    )?;

    assert!(img.pixels().any(|p| p.0 != [255, 255, 255, 255]));
    img.save(output_dir().join("vertical.png"))?;
    Ok(())
}

#[test]
fn vertical_flows_top_to_bottom() -> Result<()> {
    let Some(font) = try_font("Yu Gothic") else {
        return Ok(());
    };

    // Repeated CJK characters so vertical advances are obvious and stable.
    let text = "\u{65E5}\u{672C}\u{8A9E}".repeat(40);
    let layout = TextLayout::new(&font, Some(24.0))
        .with_writing_mode(WritingMode::VerticalRl)
        // Keep it in a single column so we can reason about Y extents.
        .with_max_height(10_000.0)
        .run(&text)?;

    let img = tiny_skia_renderer()?.render(
        &layout,
        WritingMode::VerticalRl,
        &RenderOptions {
            font_size: 24.0,
            padding: 0.0,
            background: Some([255, 255, 255, 255]),
            ..Default::default()
        },
    )?;

    let (min_y, max_y) =
        non_bg_y_bounds(&img, [255, 255, 255, 255]).expect("expected non-background pixels");

    // If vertical pen advances are applied with the wrong sign, almost all ink ends up near the
    // top edge with a large empty region below. With correct top-to-bottom flow, ink should span
    // most of the image height.
    assert!(
        min_y < img.height() / 5,
        "ink starts too low (min_y={min_y})"
    );
    assert!(
        max_y > (img.height() * 3) / 5,
        "ink does not reach far enough down (max_y={max_y}, height={})",
        img.height()
    );

    Ok(())
}

#[test]
fn render_horizontal_simplified_chinese() -> Result<()> {
    let Some(font) = try_font("Microsoft YaHei") else {
        return Ok(());
    };
    let lines = TextLayout::new(&font, Some(24.0))
        .with_max_width(1000.0)
        .run(SAMPLE_TEXT_ZH_CN)?;

    let img = tiny_skia_renderer()?.render(
        &lines,
        WritingMode::Horizontal,
        &RenderOptions {
            font_size: 24.0,
            padding: 0.0,
            background: Some([255, 255, 255, 255]),
            ..Default::default()
        },
    )?;

    assert!(img.pixels().any(|p| p.0 != [255, 255, 255, 255]));
    img.save(output_dir().join("horizontal_simplified_chinese.png"))?;
    Ok(())
}

#[test]
fn render_vertical_simplified_chinese() -> Result<()> {
    let Some(font) = try_font("Microsoft YaHei") else {
        return Ok(());
    };
    let lines = TextLayout::new(&font, Some(24.0))
        .with_writing_mode(WritingMode::VerticalRl)
        .with_max_height(1000.0)
        .run(SAMPLE_TEXT_ZH_CN)?;

    let img = tiny_skia_renderer()?.render(
        &lines,
        WritingMode::VerticalRl,
        &RenderOptions {
            font_size: 24.0,
            padding: 0.0,
            background: Some([255, 255, 255, 255]),
            ..Default::default()
        },
    )?;

    assert!(img.pixels().any(|p| p.0 != [255, 255, 255, 255]));
    img.save(output_dir().join("vertical_simplified_chinese.png"))?;
    Ok(())
}

#[test]
fn render_rgba_text() -> Result<()> {
    let Some(font) = try_font("Yu Gothic") else {
        return Ok(());
    };
    let lines = TextLayout::new(&font, Some(24.0))
        .with_max_width(1000.0)
        .run(SAMPLE_TEXT)?;

    let img = tiny_skia_renderer()?.render(
        &lines,
        WritingMode::Horizontal,
        &RenderOptions {
            font_size: 24.0,
            padding: 0.0,
            background: Some([255, 255, 255, 255]),
            color: [237, 178, 6, 255],
            ..Default::default()
        },
    )?;

    assert!(img.pixels().any(|p| p.0 != [255, 255, 255, 255]));
    img.save(output_dir().join("rgba_text.png"))?;
    Ok(())
}

mod hanonly_contracts {
    use super::*;

    type SourceColorProbe = fn(&Font) -> Result<()>;

    fn render(font: &Font, text: &str, options: &RenderOptions) -> Result<image::RgbaImage> {
        let layout = TextLayout::new(font, Some(options.font_size)).run(text)?;
        tiny_skia_renderer()?.render(&layout, WritingMode::Horizontal, options)
    }

    fn cluster_transcript(layout: &LayoutRun<'_>) -> Vec<Vec<u32>> {
        layout
            .lines
            .iter()
            .map(|line| line.glyphs.iter().map(|glyph| glyph.cluster).collect())
            .collect()
    }

    fn normal_multi_glyph_positions_have_alpha(font: &Font) -> Result<()> {
        let options = RenderOptions {
            font_size: 32.0,
            ..Default::default()
        };
        let layout = TextLayout::new(font, Some(options.font_size)).run("ABC")?;
        let image = tiny_skia_renderer()?.render(&layout, WritingMode::Horizontal, &options)?;
        let line = layout
            .lines
            .first()
            .ok_or_else(|| anyhow::anyhow!("multi-glyph probe shaped no line"))?;
        let mut pen_x = 0.0f32;

        for (position, glyph) in line.glyphs.iter().enumerate() {
            let start = (line.baseline.0 + pen_x).floor().max(0.0) as u32;
            pen_x += glyph.x_advance;
            let end = (line.baseline.0 + pen_x)
                .ceil()
                .max(start as f32 + 1.0)
                .min(image.width() as f32) as u32;
            assert!(
                (start..end).any(|x| (0..image.height()).any(|y| image.get_pixel(x, y).0[3] > 0)),
                "normal multi-glyph position {position} has no rendered alpha"
            );
        }
        assert_eq!(line.glyphs.len(), 3, "fixture must shape three glyphs");
        Ok(())
    }

    fn exact_fill_rgba(font: &Font) -> Result<()> {
        let color = [17, 83, 149, 255];
        let image = render(
            font,
            "M",
            &RenderOptions {
                color,
                anti_alias: false,
                font_size: 48.0,
                ..Default::default()
            },
        )?;
        assert!(
            image.pixels().any(|pixel| pixel.0 == color),
            "opaque glyph interior must preserve the exact requested fill RGBA"
        );
        Ok(())
    }

    fn stroke_then_fill_order(font: &Font) -> Result<()> {
        let fill = [19, 117, 211, 255];
        let stroke = [229, 41, 73, 255];
        let base = RenderOptions {
            anti_alias: false,
            font_size: 48.0,
            stroke: Some(RenderStrokeOptions {
                color: stroke,
                width_px: 3.0,
            }),
            ..Default::default()
        };
        let stroke_only = render(
            font,
            "M",
            &RenderOptions {
                color: [0, 0, 0, 0],
                ..base.clone()
            },
        )?;
        let fill_only = render(
            font,
            "M",
            &RenderOptions {
                color: fill,
                stroke: None,
                ..base.clone()
            },
        )?;
        let combined = render(
            font,
            "M",
            &RenderOptions {
                color: fill,
                ..base
            },
        )?;

        let overlap = stroke_only
            .pixels()
            .zip(fill_only.pixels())
            .zip(combined.pixels())
            .find(|((stroke_pixel, fill_pixel), _)| stroke_pixel.0[3] > 0 && fill_pixel.0 == fill)
            .map(|(_, combined_pixel)| combined_pixel.0);
        assert_eq!(
            overlap,
            Some(fill),
            "fill must overwrite the inside half of the earlier stroke pass"
        );
        assert!(
            combined.pixels().any(|pixel| pixel.0 == stroke),
            "combined render must retain an outside stroke"
        );
        Ok(())
    }

    fn transparent_fill_has_empty_alpha(font: &Font) -> Result<()> {
        let image = render(
            font,
            "ABC",
            &RenderOptions {
                color: [17, 83, 149, 0],
                ..Default::default()
            },
        )?;
        assert!(
            image.pixels().all(|pixel| pixel.0[3] == 0),
            "zero-alpha fill must leave the surface alpha empty"
        );
        Ok(())
    }

    fn oversized_surface_errors(font: &Font) -> Result<()> {
        let mut layout = TextLayout::new(font, Some(24.0)).run("A")?;
        layout.width = u32::MAX as f32;
        let error = tiny_skia_renderer()?
            .render(&layout, WritingMode::Horizontal, &RenderOptions::default())
            .expect_err("oversized logical width must error");
        assert!(
            error.to_string().contains("render surface width overflow"),
            "unexpected oversized-surface error: {error:#}"
        );
        Ok(())
    }

    fn pixmap_rejection_errors(font: &Font) -> Result<()> {
        let mut layout = TextLayout::new(font, Some(24.0)).run("A")?;
        layout.width = 300_000_000.0;
        layout.height = 1.0;
        let error = tiny_skia_renderer()?
            .render(&layout, WritingMode::Horizontal, &RenderOptions::default())
            .expect_err("tiny-skia width limit must reject the pixmap");
        assert!(
            error
                .to_string()
                .contains("failed to allocate render surface"),
            "unexpected Pixmap rejection error: {error:#}"
        );
        Ok(())
    }

    fn scale_two_and_four_keep_logical_dimensions(font: &Font) -> Result<()> {
        let layout = TextLayout::new(font, Some(32.0)).run("ABC")?;
        let expected = (layout.width.ceil() as u32, layout.height.ceil() as u32);
        for factor in [2, 4] {
            let image = tiny_skia_renderer()?.render(
                &layout,
                WritingMode::Horizontal,
                &RenderOptions {
                    font_size: 32.0,
                    raster: RasterOptions::supersampled(factor),
                    ..Default::default()
                },
            )?;
            assert_eq!(
                image.dimensions(),
                expected,
                "scale {factor} changed logical dimensions"
            );
            assert!(
                image.pixels().any(|pixel| pixel.0[3] > 0),
                "scale {factor} produced no alpha"
            );
        }
        Ok(())
    }

    fn lf_is_insertion_only_and_transcript_is_stable(font: &Font) -> Result<()> {
        let compact = TextLayout::new(font, Some(24.0)).run("ABCD")?;
        let with_lf = TextLayout::new(font, Some(24.0)).run("AB\nCD")?;
        let before = cluster_transcript(&with_lf);
        assert_eq!(
            before,
            vec![vec![0, 1], vec![3, 4]],
            "LF cluster transcript drifted"
        );

        let restored = before
            .iter()
            .flatten()
            .map(|cluster| if *cluster > 1 { cluster - 1 } else { *cluster })
            .collect::<Vec<_>>();
        assert_eq!(
            restored,
            cluster_transcript(&compact)
                .into_iter()
                .flatten()
                .collect::<Vec<_>>(),
            "removing the inserted LF must restore compact-text clusters"
        );

        tiny_skia_renderer()?.render(
            &with_lf,
            WritingMode::Horizontal,
            &RenderOptions {
                font_size: 24.0,
                ..Default::default()
            },
        )?;
        assert_eq!(
            cluster_transcript(&with_lf),
            before,
            "public render mutated the LF cluster transcript"
        );
        Ok(())
    }

    fn glyph_zero_for_non_control_errors(font: &Font) -> Result<()> {
        let mut layout = TextLayout::new(font, Some(24.0)).run("A")?;
        layout
            .lines
            .iter_mut()
            .flat_map(|line| &mut line.glyphs)
            .next()
            .ok_or_else(|| anyhow::anyhow!("glyph-zero probe shaped no glyphs"))?
            .glyph_id = 0;
        assert!(
            tiny_skia_renderer()?
                .render(&layout, WritingMode::Horizontal, &RenderOptions::default())
                .is_err(),
            "source color probe must reject glyph zero for non-control text"
        );
        Ok(())
    }

    fn glyph_id_above_u16_errors(font: &Font) -> Result<()> {
        let mut layout = TextLayout::new(font, Some(24.0)).run("A")?;
        layout
            .lines
            .iter_mut()
            .flat_map(|line| &mut line.glyphs)
            .next()
            .ok_or_else(|| anyhow::anyhow!("wide-glyph probe shaped no glyphs"))?
            .glyph_id = u32::from(u16::MAX) + 1;
        assert!(
            tiny_skia_renderer()?
                .render(&layout, WritingMode::Horizontal, &RenderOptions::default())
                .is_ok(),
            "renderer must gracefully skip shaped glyph IDs above u16"
        );
        Ok(())
    }

    #[test]
    fn hanonly_pre_greenc_red_t3_source_color_probe_contract() -> Result<()> {
        let mut book = FontBook::new();
        let font =
            book.load_from_bytes(include_bytes!("fixtures/roboto-mono-stripped.ttf").to_vec())?;
        let probes: &[(&str, SourceColorProbe)] = &[
            (
                "normal_multi_glyph_alpha",
                normal_multi_glyph_positions_have_alpha,
            ),
            ("exact_fill_rgba", exact_fill_rgba),
            ("stroke_then_fill_order", stroke_then_fill_order),
            ("transparent_fill_alpha", transparent_fill_has_empty_alpha),
            ("oversized_surface_error", oversized_surface_errors),
            ("pixmap_rejection_error", pixmap_rejection_errors),
            (
                "scale_two_and_four_logical_dimensions",
                scale_two_and_four_keep_logical_dimensions,
            ),
            (
                "lf_insertion_only_cluster_transcript",
                lf_is_insertion_only_and_transcript_is_stable,
            ),
            (
                "glyph_zero_non_control_error",
                glyph_zero_for_non_control_errors,
            ),
            ("glyph_above_u16_error", glyph_id_above_u16_errors),
        ];

        for (name, probe) in probes {
            probe(&font).map_err(|error| anyhow::anyhow!("{name}: {error:#}"))?;
        }
        Ok(())
    }
}

#[test]
fn render_with_fallback_fonts() -> Result<()> {
    let Some(primary_font) = try_font("Yu Gothic") else {
        return Ok(());
    };
    let Some(symbol) = try_font("Segoe UI Symbol") else {
        return Ok(());
    };
    let Some(emoji) = try_font("Segoe UI Emoji") else {
        return Ok(());
    };
    let fallback_fonts = vec![symbol, emoji];

    let lines = TextLayout::new(&primary_font, Some(24.0))
        .with_fallback_fonts(&fallback_fonts)
        .run("Here is a smiley: 😊 and a star: ★ and a heart: ♥")?;

    let img = tiny_skia_renderer()?.render(
        &lines,
        WritingMode::Horizontal,
        &RenderOptions {
            font_size: 24.0,
            padding: 0.0,
            background: Some([255, 255, 255, 255]),
            ..Default::default()
        },
    )?;
    assert!(img.pixels().any(|p| p.0 != [255, 255, 255, 255]));
    img.save(output_dir().join("fallback_fonts.png"))?;
    Ok(())
}

#[test]
fn test_arabic_layout_order() -> Result<()> {
    let Some(font) = try_font("Segoe UI") else {
        return Ok(());
    };
    let text = "مرحبا"; // Marhaba (Hello)
    let layout = TextLayout::new(&font, Some(24.0)).run(text)?;
    let line = &layout.lines[0];

    let bidi_info = BidiInfo::new(text, None);
    let para = &bidi_info.paragraphs[0];
    assert!(
        para.level.is_rtl(),
        "expected Arabic text to resolve to an RTL paragraph level, got {:?}",
        para.level
    );

    let clusters: Vec<u32> = line.glyphs.iter().map(|g| g.cluster).collect();
    println!("Clusters for {text}: {:?}", clusters);

    assert!(
        clusters.len() > 1,
        "expected multiple glyph clusters for Arabic shaping, got {:?}",
        clusters
    );
    assert!(
        clusters.windows(2).all(|w| w[0] >= w[1]),
        "expected RTL visual order to produce non-increasing cluster indices, got {:?}",
        clusters
    );

    Ok(())
}

#[test]
fn test_mixed_bidi_render() -> Result<()> {
    let Some(font) = try_font("Arial") else {
        return Ok(());
    };
    let text = "Hello مرحبا Hello";
    let layout = TextLayout::new(&font, Some(24.0)).run(text)?;

    let bidi_info = BidiInfo::new(text, None);
    println!("Paragraphs: {}", bidi_info.paragraphs.len());
    let para = &bidi_info.paragraphs[0];
    println!("Base Level: {:?}", para.level);

    let levels = bidi_info.levels;
    println!(
        "Levels: {:?}",
        levels.iter().map(|l| l.number()).collect::<Vec<_>>()
    );

    println!("Direction: {:?}", layout.lines[0].direction);
    println!(
        "Clusters: {:?}",
        layout.lines[0]
            .glyphs
            .iter()
            .map(|g| g.cluster)
            .collect::<Vec<_>>()
    );

    let img = tiny_skia_renderer()?.render(
        &layout,
        WritingMode::Horizontal,
        &RenderOptions {
            font_size: 24.0,
            padding: 20.0,
            background: Some([255, 255, 255, 255]),
            ..Default::default()
        },
    )?;

    img.save(output_dir().join("mixed_bidi.png"))?;
    Ok(())
}
#[test]
fn test_rtl_multiline() -> Result<()> {
    let Some(font) = try_font("Arial") else {
        return Ok(());
    };
    // A long text that will wrap.
    let text = "هذا نص طويل باللغة العربية سيتم لفه عبر عدة أسطر للتأكد من أن تخطيط الحروف والاتجاهات يعمل بشكل صحيح في جميع الأسطر. Hello World! وهذا جزء آخر.";
    let layout = TextLayout::new(&font, Some(24.0))
        .with_max_width(400.0)
        .run(text)?;

    println!("Line count: {}", layout.lines.len());
    for (i, line) in layout.lines.iter().enumerate() {
        println!(
            "Line {}: {:?} ({} glyphs)",
            i,
            line.direction,
            line.glyphs.len()
        );
    }

    let img = tiny_skia_renderer()?.render(
        &layout,
        WritingMode::Horizontal,
        &RenderOptions {
            font_size: 24.0,
            padding: 20.0,
            background: Some([255, 255, 255, 255]),
            ..Default::default()
        },
    )?;

    img.save(output_dir().join("rtl_multiline.png"))?;
    Ok(())
}

#[test]
fn test_rtl_alignment() -> Result<()> {
    let Some(font) = try_font("Arial") else {
        return Ok(());
    };
    let text = "مرحبا بالعالم"; // Hello World in Arabic

    // Test Left Alignment
    let layout_left = TextLayout::new(&font, Some(24.0))
        .with_max_width(500.0)
        .with_alignment(koharu_renderer::TextAlign::Left)
        .run(text)?;

    let img_left = tiny_skia_renderer()?.render(
        &layout_left,
        WritingMode::Horizontal,
        &RenderOptions {
            font_size: 24.0,
            padding: 20.0,
            background: Some([255, 255, 255, 255]),
            ..Default::default()
        },
    )?;
    img_left.save(output_dir().join("rtl_align_left.png"))?;

    // Test Right Alignment
    let layout_right = TextLayout::new(&font, Some(24.0))
        .with_max_width(500.0)
        .with_alignment(koharu_renderer::TextAlign::Right)
        .run(text)?;

    let img_right = tiny_skia_renderer()?.render(
        &layout_right,
        WritingMode::Horizontal,
        &RenderOptions {
            font_size: 24.0,
            padding: 20.0,
            background: Some([255, 255, 255, 255]),
            ..Default::default()
        },
    )?;
    img_right.save(output_dir().join("rtl_align_right.png"))?;

    Ok(())
}

#[test]
fn test_rtl_punctuation_numbers() -> Result<()> {
    let Some(font) = try_font("Arial") else {
        return Ok(());
    };
    // Text with numbers and trailing punctuation.
    // In LTR, it's: "Arabic 123!"
    // In RTL, "123" stays LTR, but "!" might move to the left side of the word.
    let text = "هذا اختبار 123!";
    let layout = TextLayout::new(&font, Some(24.0)).run(text)?;

    println!(
        "Clusters: {:?}",
        layout.lines[0]
            .glyphs
            .iter()
            .map(|g| g.cluster)
            .collect::<Vec<_>>()
    );

    let img = tiny_skia_renderer()?.render(
        &layout,
        WritingMode::Horizontal,
        &RenderOptions {
            font_size: 24.0,
            padding: 20.0,
            background: Some([255, 255, 255, 255]),
            ..Default::default()
        },
    )?;

    img.save(output_dir().join("rtl_punctuation_numbers.png"))?;
    Ok(())
}

#[test]
fn test_rtl_mixed_complex() -> Result<()> {
    let Some(font) = try_font("Arial") else {
        return Ok(());
    };
    // Mixed text with LTR and RTL sequences.
    let text = "The word for 'Apple' is تفاحة in Arabic.";
    let layout = TextLayout::new(&font, Some(24.0)).run(text)?;

    let img = tiny_skia_renderer()?.render(
        &layout,
        WritingMode::Horizontal,
        &RenderOptions {
            font_size: 24.0,
            padding: 20.0,
            background: Some([255, 255, 255, 255]),
            ..Default::default()
        },
    )?;

    img.save(output_dir().join("rtl_mixed.png"))?;
    Ok(())
}

#[test]
fn test_rtl_user_reported_string() -> Result<()> {
    let Some(font) = try_font("Arial") else {
        return Ok(());
    };
    // The problematic string from the user.
    let text = "هل من المقبول حقاً ارتداء ملابس كهذه، إنها مجرد خيط؟";
    let layout = TextLayout::new(&font, Some(24.0))
        .with_max_width(200.0) // Narrow width to force multiline
        .run(text)?;

    let img = tiny_skia_renderer()?.render(
        &layout,
        WritingMode::Horizontal,
        &RenderOptions {
            font_size: 24.0,
            padding: 20.0,
            background: Some([173, 216, 230, 255]), // Light blue to match screenshot
            ..Default::default()
        },
    )?;

    img.save(output_dir().join("rtl_user_reported.png"))?;
    Ok(())
}

#[test]
fn test_complex_reordering_and_glyph_count() -> Result<()> {
    let Some(font) = try_font("Arial") else {
        return Ok(());
    };
    let text = "A مرحبا 😊";
    let layout = TextLayout::new(&font, Some(24.0)).run(text)?;
    let line = &layout.lines[0];

    // Check that we have valid layout (at least one glyph per word run).
    let clusters: Vec<u32> = line.glyphs.iter().map(|g| g.cluster).collect();
    println!("Clusters for '{}': {:?}", text, clusters);

    assert!(
        !clusters.is_empty(),
        "Expected layout to produce at least some glyphs"
    );

    // Verify all clusters are within the string range.
    for &cluster in &clusters {
        assert!((cluster as usize) < text.len());
    }

    // Check for duplicates that might indicate the duplication bug.
    // Some ligatures or combining sequences might legitimately have multiple glyphs for one
    // cluster, but we shouldn't have more repeated glyph identities than that shaping requires.
    let mut unique_glyphs = std::collections::HashSet::new();
    for g in &line.glyphs {
        // Use a tuple of (cluster, glyph_id, x_advance) as a proxy for identity.
        let identity = (g.cluster, g.glyph_id, (g.x_advance * 100.0) as i32);
        unique_glyphs.insert(identity);
    }

    let unique_clusters = clusters
        .iter()
        .copied()
        .collect::<std::collections::HashSet<_>>();
    let allowed_duplicates = line.glyphs.len().saturating_sub(unique_clusters.len());
    let duplicate_glyphs = line.glyphs.len().saturating_sub(unique_glyphs.len());

    assert!(
        duplicate_glyphs <= allowed_duplicates,
        "Unexpected duplicated glyph identities for '{}': {} duplicated glyphs across {} glyphs and {} clusters",
        text,
        duplicate_glyphs,
        line.glyphs.len(),
        unique_clusters.len()
    );
    Ok(())
}
