use std::io::Cursor;

use image::{DynamicImage, Rgba, RgbaImage};

fn one_pixel_rgba() -> DynamicImage {
    DynamicImage::ImageRgba8(RgbaImage::from_pixel(1, 1, Rgba([255, 0, 0, 255])))
}

fn encode_png() -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    one_pixel_rgba().write_to(&mut buf, image::ImageFormat::Png).unwrap();
    buf.into_inner()
}

fn encode_jpeg() -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    one_pixel_rgba().write_to(&mut buf, image::ImageFormat::Jpeg).unwrap();
    buf.into_inner()
}

fn encode_webp() -> Vec<u8> {
    let mut buf = Cursor::new(Vec::new());
    one_pixel_rgba().write_to(&mut buf, image::ImageFormat::WebP).unwrap();
    buf.into_inner()
}

#[test]
fn accepts_valid_png() {
    let bytes = encode_png();
    let result = koharu_app::blobs::admit_source_image(&bytes);
    assert!(result.is_ok(), "PNG must be admitted");
}

#[test]
fn accepts_valid_jpeg() {
    let bytes = encode_jpeg();
    let result = koharu_app::blobs::admit_source_image(&bytes);
    assert!(result.is_ok(), "JPEG must be admitted");
}

#[test]
fn accepts_valid_webp() {
    let bytes = encode_webp();
    let result = koharu_app::blobs::admit_source_image(&bytes);
    assert!(result.is_ok(), "WebP must be admitted");
}

#[test]
fn rejects_empty_bytes() {
    let result = koharu_app::blobs::admit_source_image(&[]);
    assert!(result.is_err(), "empty input must be rejected");
}

#[test]
fn rejects_gif_magic() {
    let result = koharu_app::blobs::admit_source_image(b"GIF89a.....................");
    assert!(result.is_err(), "GIF magic must be rejected");
}

#[test]
fn rejects_bmp_magic() {
    let result = koharu_app::blobs::admit_source_image(b"BM.........................");
    assert!(result.is_err(), "BMP magic must be rejected");
}

#[test]
fn rejects_unknown_magic() {
    let result = koharu_app::blobs::admit_source_image(b"RANDOMBYTES...............");
    assert!(result.is_err(), "unknown magic must be rejected");
}

#[test]
fn rejects_corrupt_png_header() {
    let mut bytes = encode_png();
    bytes[1] = 0xFF; // corrupt the PNG signature
    let result = koharu_app::blobs::admit_source_image(&bytes);
    assert!(result.is_err(), "corrupt PNG header must be rejected");
}

#[test]
fn rejects_extension_spoof() {
    // Wrap JPEG bytes with PNG magic to simulate spoof
    let jpeg = encode_jpeg();
    let mut spoofed = vec![0x89, 0x50, 0x4E, 0x47]; // PNG magic
    spoofed.extend_from_slice(&jpeg[..]);
    let result = koharu_app::blobs::admit_source_image(&spoofed);
    assert!(result.is_err(), "PNG magic + JPEG body must be rejected as corrupt");
}
