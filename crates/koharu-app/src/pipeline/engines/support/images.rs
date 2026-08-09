//! Image and mask node helpers for pipeline engines.

use crate::blobs::BlobStore;
use anyhow::Context;
use anyhow::Result;
use image::DynamicImage;
use image::GrayImage;
use image::Luma;
use koharu_core::{
    BlobRef, ImageData, ImageRole, MaskRole, NodeId, NodeKind, PageId, Scene, TextData, Transform,
};

pub fn source_node(scene: &Scene, page: PageId) -> Result<(NodeId, &ImageData)> {
    let page = scene
        .page(page)
        .with_context(|| format!("page {} not found", page))?;
    for (id, node) in page.nodes.iter() {
        if let NodeKind::Image(img) = &node.kind
            && img.role == ImageRole::Source
        {
            return Ok((*id, img));
        }
    }
    anyhow::bail!("page has no Source image node")
}

/// Load the source image bytes + decoded image for `page`.
pub fn load_source_image(scene: &Scene, page: PageId, blobs: &BlobStore) -> Result<DynamicImage> {
    let (_, img_data) = source_node(scene, page)?;
    blobs.load_image(&img_data.blob)
}

/// Zero every mask pixel outside `region` while preserving the source
/// dimensions. Inpainting engines use this to limit repair-brush work.
pub fn clip_mask_to_region(mask: &DynamicImage, region: &koharu_core::Region) -> DynamicImage {
    DynamicImage::ImageLuma8(clip_gray_mask_to_region(&mask.to_luma8(), region))
}

/// Gray-image variant of [`clip_mask_to_region`].
pub fn clip_gray_mask_to_region(src: &GrayImage, region: &koharu_core::Region) -> GrayImage {
    let (width, height) = src.dimensions();
    let x0 = region.x.min(width);
    let y0 = region.y.min(height);
    let x1 = region.x.saturating_add(region.width).min(width);
    let y1 = region.y.saturating_add(region.height).min(height);

    let mut clipped = GrayImage::new(width, height);
    for y in y0..y1 {
        for x in x0..x1 {
            clipped.put_pixel(x, y, Luma([src.get_pixel(x, y).0[0]]));
        }
    }
    clipped
}

/// Find a node of `Image { role }` on `page`, if any.
pub fn find_image_node(scene: &Scene, page: PageId, role: ImageRole) -> Option<(NodeId, BlobRef)> {
    let page = scene.page(page)?;
    page.nodes.iter().find_map(|(id, node)| match &node.kind {
        NodeKind::Image(img) if img.role == role => Some((*id, img.blob.clone())),
        _ => None,
    })
}

/// Find a node of `Mask { role }` on `page`, if any.
pub fn find_mask_node(scene: &Scene, page: PageId, role: MaskRole) -> Option<(NodeId, BlobRef)> {
    let page = scene.page(page)?;
    page.nodes.iter().find_map(|(id, node)| match &node.kind {
        NodeKind::Mask(mask) if mask.role == role => Some((*id, mask.blob.clone())),
        _ => None,
    })
}

/// Collect `(NodeId, &Transform, &TextData)` for every text node on `page`,
/// in stacking order.
pub fn text_nodes(scene: &Scene, page: PageId) -> Vec<(NodeId, &Transform, &TextData)> {
    let Some(page) = scene.page(page) else {
        return Vec::new();
    };
    page.nodes
        .iter()
        .filter(|(_, node)| node.visible)
        .filter_map(|(id, node)| match &node.kind {
            NodeKind::Text(t) => Some((*id, &node.transform, t)),
            _ => None,
        })
        .collect()
}

pub fn contains_han(text: &str) -> bool {
    use icu_properties::{CodePointMapData, props::Script};

    let scripts = CodePointMapData::<Script>::new();
    text.chars().any(|ch| scripts.get(ch) == Script::Han)
}

pub fn contains_protected_latin_word(text: &str) -> bool {
    use icu_properties::{CodePointMapData, props::Script};

    let scripts = CodePointMapData::<Script>::new();
    let mut letters = 0;
    for ch in text.chars() {
        if scripts.get(ch) == Script::Latin && ch.is_alphabetic() {
            letters += 1;
        } else if matches!(ch, '-' | '\'' | '’') && letters > 0 {
            continue;
        } else {
            if letters >= 2 {
                return true;
            }
            letters = 0;
        }
    }
    letters >= 2
}
