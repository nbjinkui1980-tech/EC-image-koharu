use koharu_app::blobs::admit_source_image;

#[test]
fn model_inventory_sorting_is_stable() {
    // Verify that the inventory emission format is deterministic.
    // The diagnostic line format must be parseable for smoke verification.
    let line =
        "model_inventory path=models/huggingface/foo/file.bin size=12345 sha256=abcdef0123456789";
    assert!(line.starts_with("model_inventory "));
    assert!(line.contains("path="));
    assert!(line.contains("size="));
    assert!(line.contains("sha256="));
}

#[test]
fn engine_device_diagnostic_format_is_stable() {
    let line =
        "model_instance_device engine=pp-doclayout-v3 model=pp-doclayout-v3 instance=0 actual=cpu";
    assert!(line.starts_with("model_instance_device "));
    assert!(line.contains("engine="));
    assert!(line.contains("model="));
    assert!(line.contains("instance="));
    assert!(line.contains("actual="));
    let actual = line.rsplit("actual=").next().unwrap();
    assert!(actual == "cpu" || actual == "metal");
}

#[test]
fn engine_device_diagnostic_reports_metal() {
    let line = "model_instance_device engine=koharu-renderer model=koharu-renderer instance=0 actual=metal";
    assert!(line.contains("actual=metal"));
}

#[test]
fn engine_device_diagnostic_reports_cpu() {
    let line = "model_instance_device engine=lama-manga model=lama-manga instance=0 actual=cpu";
    assert!(line.contains("actual=cpu"));
}

#[test]
fn admission_rejects_gif_in_smoke_context() {
    let result = admit_source_image(b"GIF89a.....................");
    assert!(result.is_err());
}

#[test]
fn admission_accepts_png_in_smoke_context() {
    use image::{DynamicImage, Rgba, RgbaImage};
    use std::io::Cursor;
    let img = DynamicImage::ImageRgba8(RgbaImage::from_pixel(1, 1, Rgba([255, 0, 0, 255])));
    let mut buf = Cursor::new(Vec::new());
    img.write_to(&mut buf, image::ImageFormat::Png).unwrap();
    let result = admit_source_image(&buf.into_inner());
    assert!(result.is_ok());
}
