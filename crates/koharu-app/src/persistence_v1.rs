//! Pre-v2 postcard wire shapes. Keep field and enum order frozen.
#![allow(clippy::large_enum_variant)] // Boxing would change the frozen postcard wire format.

use std::fmt;
use std::marker::PhantomData;

use koharu_core::{
    BlobRef, FontPrediction, ImageData, ImageDataPatch, MaskData, MaskDataPatch, Node,
    NodeDataPatch, NodeId, NodeKind, NodePatch, Op, Page, PageId, PagePatch, ProjectMeta,
    ProjectMetaPatch, Scene, TextData, TextDataPatch, TextDirection, TextStyle, Transform,
};
use serde::de::{MapAccess, Visitor};
use serde::{Deserialize, Deserializer};

pub(crate) fn decode_snapshot(bytes: &[u8]) -> anyhow::Result<(Scene, u64)> {
    let snapshot: Snapshot = postcard::from_bytes(bytes)?;
    Ok((snapshot.scene.into_current(), snapshot.epoch))
}

pub(crate) fn decode_log_frame(bytes: &[u8]) -> anyhow::Result<(u64, Op)> {
    let frame: LogFrame = postcard::from_bytes(bytes)?;
    Ok((frame.epoch, frame.op.into_current()))
}

#[derive(Deserialize)]
struct Snapshot {
    epoch: u64,
    scene: SceneV1,
}

#[derive(Deserialize)]
struct LogFrame {
    epoch: u64,
    op: OpV1,
}

/// Deserialize a serde map while preserving its encoded insertion order.
struct OrderedMap<K, V>(Vec<(K, V)>);

impl<'de, K, V> Deserialize<'de> for OrderedMap<K, V>
where
    K: Deserialize<'de>,
    V: Deserialize<'de>,
{
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct OrderedMapVisitor<K, V>(PhantomData<(K, V)>);

        impl<'de, K, V> Visitor<'de> for OrderedMapVisitor<K, V>
        where
            K: Deserialize<'de>,
            V: Deserialize<'de>,
        {
            type Value = OrderedMap<K, V>;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("an ordered map")
            }

            fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut entries = Vec::with_capacity(map.size_hint().unwrap_or(0));
                while let Some(entry) = map.next_entry()? {
                    entries.push(entry);
                }
                Ok(OrderedMap(entries))
            }
        }

        deserializer.deserialize_map(OrderedMapVisitor(PhantomData))
    }
}

#[derive(Deserialize)]
struct SceneV1 {
    project: ProjectMeta,
    pages: OrderedMap<PageId, PageV1>,
}

impl SceneV1 {
    fn into_current(self) -> Scene {
        Scene {
            project: self.project,
            pages: self
                .pages
                .0
                .into_iter()
                .map(|(id, page)| (id, page.into_current()))
                .collect(),
        }
    }
}

#[derive(Deserialize)]
struct PageV1 {
    id: PageId,
    name: String,
    width: u32,
    height: u32,
    nodes: OrderedMap<NodeId, NodeV1>,
}

impl PageV1 {
    fn into_current(self) -> Page {
        Page {
            id: self.id,
            name: self.name,
            width: self.width,
            height: self.height,
            nodes: self
                .nodes
                .0
                .into_iter()
                .map(|(id, node)| (id, node.into_current()))
                .collect(),
        }
    }
}

#[derive(Deserialize)]
struct NodeV1 {
    id: NodeId,
    transform: Transform,
    visible: bool,
    kind: NodeKindV1,
}

impl NodeV1 {
    fn into_current(self) -> Node {
        Node {
            id: self.id,
            transform: self.transform,
            visible: self.visible,
            kind: self.kind.into_current(),
        }
    }
}

#[derive(Deserialize)]
enum NodeKindV1 {
    Image(ImageData),
    Text(TextDataV1),
    Mask(MaskData),
}

impl NodeKindV1 {
    fn into_current(self) -> NodeKind {
        match self {
            Self::Image(data) => NodeKind::Image(data),
            Self::Text(data) => NodeKind::Text(data.into_current()),
            Self::Mask(data) => NodeKind::Mask(data),
        }
    }
}

#[derive(Deserialize)]
struct TextDataV1 {
    confidence: f32,
    source_lang: Option<String>,
    source_direction: Option<TextDirection>,
    rendered_direction: Option<TextDirection>,
    line_polygons: Option<Vec<[[f32; 2]; 4]>>,
    rotation_deg: Option<f32>,
    detected_font_size_px: Option<f32>,
    detector: Option<String>,
    text: Option<String>,
    translation: Option<String>,
    style: Option<TextStyle>,
    font_prediction: Option<FontPrediction>,
    sprite: Option<BlobRef>,
    sprite_transform: Option<Transform>,
    lock_layout_box: bool,
}

impl TextDataV1 {
    fn into_current(self) -> TextData {
        TextData {
            confidence: self.confidence,
            source_lang: self.source_lang,
            source_direction: self.source_direction,
            rendered_direction: self.rendered_direction,
            line_polygons: self.line_polygons,
            rotation_deg: self.rotation_deg,
            detected_font_size_px: self.detected_font_size_px,
            detector: self.detector,
            text: self.text,
            translation: self.translation,
            style: self.style,
            font_prediction: self.font_prediction,
            sprite: self.sprite,
            sprite_transform: self.sprite_transform,
            lock_layout_box: self.lock_layout_box,
            typography_plan_verified: false,
        }
    }
}

#[derive(Deserialize)]
enum OpV1 {
    UpdateProjectMeta {
        patch: ProjectMetaPatch,
        prev: ProjectMetaPatch,
    },
    AddPage {
        page: PageV1,
        at: usize,
    },
    RemovePage {
        id: PageId,
        prev_page: PageV1,
        prev_index: usize,
    },
    UpdatePage {
        id: PageId,
        patch: PagePatch,
        prev: PagePatch,
    },
    ReorderPages {
        order: Vec<PageId>,
        prev_order: Vec<PageId>,
    },
    AddNode {
        page: PageId,
        node: NodeV1,
        at: usize,
    },
    RemoveNode {
        page: PageId,
        id: NodeId,
        prev_node: NodeV1,
        prev_index: usize,
    },
    UpdateNode {
        page: PageId,
        id: NodeId,
        patch: NodePatchV1,
        prev: NodePatchV1,
    },
    ReorderNodes {
        page: PageId,
        order: Vec<NodeId>,
        prev_order: Vec<NodeId>,
    },
    Batch {
        ops: Vec<OpV1>,
        label: String,
    },
}

impl OpV1 {
    fn into_current(self) -> Op {
        match self {
            Self::UpdateProjectMeta { patch, prev } => Op::UpdateProjectMeta { patch, prev },
            Self::AddPage { page, at } => Op::AddPage {
                page: page.into_current(),
                at,
            },
            Self::RemovePage {
                id,
                prev_page,
                prev_index,
            } => Op::RemovePage {
                id,
                prev_page: prev_page.into_current(),
                prev_index,
            },
            Self::UpdatePage { id, patch, prev } => Op::UpdatePage { id, patch, prev },
            Self::ReorderPages { order, prev_order } => Op::ReorderPages { order, prev_order },
            Self::AddNode { page, node, at } => Op::AddNode {
                page,
                node: node.into_current(),
                at,
            },
            Self::RemoveNode {
                page,
                id,
                prev_node,
                prev_index,
            } => Op::RemoveNode {
                page,
                id,
                prev_node: prev_node.into_current(),
                prev_index,
            },
            Self::UpdateNode {
                page,
                id,
                patch,
                prev,
            } => Op::UpdateNode {
                page,
                id,
                patch: patch.into_current(),
                prev: prev.into_current(),
            },
            Self::ReorderNodes {
                page,
                order,
                prev_order,
            } => Op::ReorderNodes {
                page,
                order,
                prev_order,
            },
            Self::Batch { ops, label } => Op::Batch {
                ops: ops.into_iter().map(Self::into_current).collect(),
                label,
            },
        }
    }
}

#[derive(Deserialize)]
struct NodePatchV1 {
    transform: Option<Transform>,
    visible: Option<bool>,
    data: Option<NodeDataPatchV1>,
}

impl NodePatchV1 {
    fn into_current(self) -> NodePatch {
        NodePatch {
            transform: self.transform,
            visible: self.visible,
            data: self.data.map(NodeDataPatchV1::into_current),
        }
    }
}

#[derive(Deserialize)]
enum NodeDataPatchV1 {
    Text(TextDataPatchV1),
    Image(ImageDataPatch),
    Mask(MaskDataPatch),
}

impl NodeDataPatchV1 {
    fn into_current(self) -> NodeDataPatch {
        match self {
            Self::Text(patch) => NodeDataPatch::Text(patch.into_current()),
            Self::Image(patch) => NodeDataPatch::Image(patch),
            Self::Mask(patch) => NodeDataPatch::Mask(patch),
        }
    }
}

#[derive(Deserialize)]
struct TextDataPatchV1 {
    confidence: Option<f32>,
    source_lang: Option<Option<String>>,
    source_direction: Option<Option<TextDirection>>,
    rendered_direction: Option<Option<TextDirection>>,
    line_polygons: Option<Option<Vec<[[f32; 2]; 4]>>>,
    rotation_deg: Option<Option<f32>>,
    detected_font_size_px: Option<Option<f32>>,
    detector: Option<Option<String>>,
    text: Option<Option<String>>,
    translation: Option<Option<String>>,
    style: Option<Option<TextStyle>>,
    font_prediction: Option<Option<FontPrediction>>,
    sprite: Option<Option<BlobRef>>,
    sprite_transform: Option<Option<Transform>>,
    lock_layout_box: Option<bool>,
}

impl TextDataPatchV1 {
    fn into_current(self) -> TextDataPatch {
        TextDataPatch {
            confidence: self.confidence,
            source_lang: self.source_lang,
            source_direction: self.source_direction,
            rendered_direction: self.rendered_direction,
            line_polygons: self.line_polygons,
            rotation_deg: self.rotation_deg,
            detected_font_size_px: self.detected_font_size_px,
            detector: self.detector,
            text: self.text,
            translation: self.translation,
            style: self.style,
            font_prediction: self.font_prediction,
            sprite: self.sprite,
            sprite_transform: self.sprite_transform,
            lock_layout_box: self.lock_layout_box,
            typography_plan_verified: None,
        }
    }
}
