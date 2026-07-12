# 电商图片仅中文翻译与渲染质量 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 默认工作流只擦除、翻译和重绘含 Unicode Han 字符且具备可靠逐行几何的中文内容，严格保留英文原像素和译文显式行数。

**Architecture:** `PipelineConfig` 提供唯一的 `SourceTextPolicy`。`HanOnly` 通过一个共享行解析边界驱动 Segment、严格逐行翻译、三种 Inpainter 和 Renderer；缺少可验证、可安全映射到现有 `Transform` 的逐行几何时安全跳过。普通 mask 在 expansion 前后都限制到中文 support，所有 Inpainter 最终恢复 mask 外原像素，Flux2 额外关闭内部重复扩张；`AllText` 保持现有节点级 Provider、节点 transform 和通用布局。

**Tech Stack:** Rust、Serde/TOML、Utoipa/OpenAPI、`icu_properties`、`image`/`imageproc`、React、TypeScript、Vitest、Bun、Tauri。

---

## 执行规则与不变量

- 使用 `@test-driven-development`：每个行为先写失败测试，再做最小实现，再验证通过。
- 完成前使用 `@verification-before-completion` 执行 Task 6 全部门禁。
- 不给 `StartPipelineRequest`、MCP 或 CLI 增加逐请求策略字段；入口从当前服务端配置继承策略。
- 不新增外部包；只把 workspace 已有的 `icu_properties`、`imageproc` 声明为 `koharu-app` 直接依赖。
- `HanOnly`：纯英文在 inference/provider/render 调用前短路，不进入 Segment、Translate、Inpaint、Render；混合节点只有安全逐行 polygons 才处理中文行。
- 混合节点缺 polygons、polygon 数量不匹配、非有限、退化、完全越界或无法安全映射到现有矩形 `Transform` 时返回 unsupported；禁止等高或旋转猜测。本次只接受轴对齐 quad，并把最终逐行几何规范化为 `quad bbox ∩ node rect ∩ image rect`，后续阶段不得再读取原始越界 quad；旋转/斜切混合行留待独立几何设计。
- `AllText`：保持当前一节点一个 Provider block、legacy fallback、节点 transform 和通用布局。
- `text_node_ids`：HanOnly 同时限制 Provider、回组、cleanup 和 Renderer；AllText 保持当前 translator scope，full-page Renderer 继续忽略该字段。
- `region: Some(_)` 表示 Repair Brush：不做语言过滤，但 expansion 后必须重新限制到用户 region。
- `Artifact::ready()` 当前未被 pipeline driver 调用，本计划不改变其签名或把它当作 HanOnly 完成判据。
- UI 数组只选择 engines；执行顺序由 artifact DAG 决定。
- 旧项目若已擦除英文，必须从 Source 完整重跑，不能只重新 Render。
- 空 eligible lines、空 strict sources 和空最终 mask 必须在模型/Provider closure 前返回；自动化测试不得下载或加载模型，模型集成测试继续 `#[ignore]`。
- Lama、AOT、Flux2 及 Repair Brush 的最终输出在 mask/region 外必须逐像素等于各自输入基面。
- 停止条件：自动化门禁通过，当前问题图从 Source 完整重跑并通过人工验收。

### Task 1: 增加策略配置、入口继承与 API 兼容

**Files:**

- Modify: `crates/koharu-app/src/config.rs:55-105,385-470`
- Modify: `crates/koharu-app/src/pipeline/engine.rs:52-68`
- Modify: `crates/koharu-rpc/src/routes/pipelines.rs:67-92`
- Modify: `crates/koharu-rpc/src/routes/pages.rs:531-553`
- Modify: `crates/koharu-rpc/src/mcp/mod.rs:174-196`
- Modify: `crates/koharu-app/bin/pipeline.rs:124-233`
- Test: `crates/koharu-rpc/tests/openapi.rs`
- Generated: `ui/openapi.json`
- Generated: `ui/lib/api/generated.ts`
- Generated: `ui/lib/api/schemas/*`

**Step 1: 写配置默认值与 OpenAPI 边界失败测试**

在 `config.rs` 增加：

```rust
#[test]
fn old_config_defaults_source_text_policy_to_han_only() {
    let config: AppConfig = toml::from_str("[pipeline]\ndetector = 'pp-doclayout-v3'")
        .expect("old config must deserialize");
    assert_eq!(config.pipeline.source_text_policy, SourceTextPolicy::HanOnly);
}

#[test]
fn all_text_source_text_policy_round_trips() {
    let config: AppConfig = toml::from_str("[pipeline]\nsource_text_policy = 'all_text'")
        .expect("policy must deserialize");
    assert_eq!(config.pipeline.source_text_policy, SourceTextPolicy::AllText);
    assert!(toml::to_string(&config)
        .expect("config must serialize")
        .contains("source_text_policy = \"all_text\""));
}
```

Run: `bun cargo test -p koharu-app source_text_policy`

Expected: FAIL，类型或字段不存在。

同时在 `crates/koharu-rpc/tests/openapi.rs` 断言 `PipelineConfig` schema 包含 `source_text_policy`、`StartPipelineRequest` 不包含该字段、HTTP 路径快照不变。

Run: `bun cargo test -p koharu-rpc --test openapi`

Expected: FAIL，`PipelineConfig` schema 尚无新字段。

**Step 2: 实现最小策略类型**

```rust
#[derive(Debug, Copy, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SourceTextPolicy {
    HanOnly,
    AllText,
}

impl Default for SourceTextPolicy {
    fn default() -> Self {
        Self::HanOnly
    }
}
```

给 `PipelineConfig` 和 `PipelineRunOptions` 增加 `source_text_policy`；两者默认值均为 `HanOnly`。不得修改 `PipelineConfigPatch` 或 `StartPipelineRequest`。

Run: `bun cargo test -p koharu-app source_text_policy`

Run: `bun cargo test -p koharu-rpc --test openapi`

Expected: 全部 PASS。

**Step 3: 写四入口继承失败测试**

只提取四个私有 options builder：

- HTTP：`options_from_request(req: &StartPipelineRequest, config: &AppConfig)`。
- MCP：`options_from_input(input: &StartPipelineInput, config: &AppConfig)`。
- Repair Brush：`repair_options(region: Region, config: &AppConfig)`。
- CLI：`pipeline_options(cli: &Cli, config: &AppConfig)`。

测试统一以 `*_inherits_source_text_policy` 命名：策略改为 `AllText` 后断言 options 为 `AllText`；HTTP/MCP builder 返回后继续读取原 request/input 的 `steps` 和 `pages`；Repair Brush 同时断言 `region: Some(region)`；CLI 同时断言原字符串 options 被正确复制。

Run: `bun cargo test -p koharu-rpc inherits_source_text_policy`

Expected: FAIL，三个 RPC builder 不存在。

Run: `bun cargo test -p koharu-app inherits_source_text_policy`

Expected: FAIL，CLI builder 不存在。

**Step 4: 接入四个 builder**

HTTP、MCP、Repair Brush 使用 `&app.config.load()`；CLI 在 `cfg` 移入 `App::new` 前构造 options。builder 借用输入，只克隆 options 拥有的字符串，不克隆完整 request。

Run: `bun cargo test -p koharu-rpc inherits_source_text_policy`

Run: `bun cargo test -p koharu-app inherits_source_text_policy`

Expected: 全部 PASS。

**Step 5: 重新验证 OpenAPI 边界并生成客户端**

确认 Step 1 已锁定：

- `PipelineConfig` schema 包含 `source_text_policy`。
- `StartPipelineRequest` schema 不包含该字段。
- HTTP 路径快照不变。

Run: `bun cargo test -p koharu-rpc --test openapi`

Expected: PASS。

Run: `bun run generate:api`

Expected: 只增加 `PipelineConfig.source_text_policy`、`SourceTextPolicy` schema、`ui/lib/api/schemas/sourceTextPolicy.ts` 和 schemas index 导出。

Run: `git status --short -- ui/openapi.json ui/lib/api/generated.ts ui/lib/api/schemas`

Run: `git diff --check -- ui/openapi.json ui/lib/api/generated.ts ui/lib/api/schemas`

Expected: 只列预期生成物；`StartPipelineRequest` 不变化。此时不运行 `check:generated`，因为预期生成物尚未提交。

**Step 6: 提交 Task 1**

```bash
git add crates/koharu-app/src/config.rs \
  crates/koharu-app/src/pipeline/engine.rs \
  crates/koharu-rpc/src/routes/pipelines.rs \
  crates/koharu-rpc/src/routes/pages.rs \
  crates/koharu-rpc/src/mcp/mod.rs \
  crates/koharu-app/bin/pipeline.rs \
  crates/koharu-rpc/tests/openapi.rs \
  ui/openapi.json ui/lib/api/generated.ts ui/lib/api/schemas
git commit -m "feat(pipeline): add source text policy"
```

### Task 2: 建立可靠的 Han-only 行几何边界

**Files:**

- Modify: `crates/koharu-app/Cargo.toml`
- Modify: `crates/koharu-app/src/pipeline/engines/support.rs:1-125`
- Modify: `crates/koharu-app/src/pipeline/mod.rs:120-260,355-380`
- Test: `crates/koharu-app/src/pipeline/engines/support.rs`
- Test: `crates/koharu-app/src/pipeline/mod.rs`

**Step 1: 声明已有依赖**

```toml
icu_properties = { workspace = true, features = ["compiled_data"] }
imageproc = { workspace = true }
```

不修改 workspace 版本，不新增 lockfile 包。

**Step 2: 写行解析失败测试**

测试统一以 `eligible_` 开头，覆盖：

- 纯中文、纯英文、中文加数字标点。
- `S-CURVE\nS型曲线` 有两个绝对坐标 polygons 时只返回第二行。
- polygon 数量与 OCR 非空行数不匹配的混合节点返回 `None`。
- 水平、竖排、任意旋转角度的混合节点只要缺 polygons 都返回 `None`。
- 混合节点的 NaN、Infinity、零面积、完全越界、非零节点 rotation、旋转或斜切 quad 返回 `None`；轴对齐 quad 与节点/图像相交后，只保留三者交集形成的规范化 bbox/quad。
- oversized quad 即使只与节点相交 1 px，也不得在节点外产生允许区域；规范化 quad 的所有点必须位于节点与图像交集内。
- 纯中文多行缺 polygons 时每行保留原 `line_index`，共享经过有限性、正面积和图像裁剪检查的节点 region；无效节点 region 返回 `None`。
- 纯英文缺 polygons 返回 `Some(Vec::new())`。

`EligibleTextLine` 不派生 `PartialEq`；分别比较字段，禁止修改 `TextRegion` 只为测试方便。

Run: `bun cargo test -p koharu-app pipeline::engines::support::tests::eligible_`

Expected: FAIL，目标类型和函数不存在。

**Step 3: 实现最小行解析契约**

```rust
#[derive(Clone, Debug)]
pub struct EligibleTextLine {
    pub line_index: usize,
    pub text: String,
    pub region: TextRegion,
}

pub fn contains_han(text: &str) -> bool {
    use icu_properties::{CodePointMapData, props::Script};
    let scripts = CodePointMapData::<Script>::new();
    text.chars().any(|ch| scripts.get(ch) == Script::Han)
}
```

`eligible_text_lines(transform, text, image_width, image_height) -> Option<Vec<EligibleTextLine>>` 固定语义：

1. 用 `text.lines().enumerate()` 保留原 index，忽略空白行。
2. 只定义一个 `safe_mixed_line_bbox(quad, transform, image_width, image_height) -> Option<[f32; 4]>`：要求 transform 的 `x/y/width/height/rotation_deg` 全部有限、宽高为正且 `rotation_deg == 0.0`；检查四点有限、shoelace 面积非零且四条边轴对齐。依次求 quad bbox 与节点矩形、图像矩形的交集，任一交集为空即返回 `None`；返回最终绝对坐标交集，不增加新 geometry 类型或角度容差配置。
3. safe polygons 数量等于非空行数时，按最终交集逐行建立 bbox，并用 `[x1,y1]、[x2,y1]、[x2,y2]、[x1,y2]` 重建唯一规范化 quad；每个 `TextRegion.line_polygons` 只保存这个规范化 quad，禁止复制原始 quad。
4. 全部非空行含 Han 且 polygons 缺失或不匹配时，每行共享经过同样有限性、正面积与图像交集检查的节点级 region，并将 `line_polygons` 设为 `None`；节点不存在英文，不会扩大英文处理范围。节点 region 无效时返回 `None`。
5. 混合节点只有全部非空行都通过 `safe_mixed_line_bbox()` 才返回 Han 行；否则返回 `None`。
6. 没有 Han 的合法节点返回 `Some(Vec::new())`。
7. 不接收 `SourceTextPolicy`；AllText 调用方继续走原节点级路径。

Run: `bun cargo test -p koharu-app pipeline::engines::support::tests::eligible_`

Expected: PASS。

**Step 4: 写 collector 与 warning 去重失败测试**

删除 `EligiblePageLines` DTO，collector 直接返回 tuple：

```rust
#[derive(Clone, Debug)]
pub struct UnsupportedTextGeometry {
    pub node_id: NodeId,
    pub direction: Option<koharu_core::TextDirection>,
    pub rotation_deg: f32,
    pub line_count: usize,
}

pub fn eligible_lines_for_page(
    scene: &Scene,
    page: PageId,
) -> (
    Vec<(NodeId, EligibleTextLine)>,
    Vec<UnsupportedTextGeometry>,
) {
    let Some(page_ref) = scene.page(page) else {
        return (Vec::new(), Vec::new());
    };
    let (image_width, image_height) = (page_ref.width, page_ref.height);
    let mut lines = Vec::new();
    let mut unsupported = Vec::new();
    for (id, transform, text) in text_nodes(scene, page) {
        match eligible_text_lines(transform, text, image_width, image_height) {
            Some(found) => lines.extend(found.into_iter().map(|line| (id, line))),
            None => unsupported.push(UnsupportedTextGeometry {
                node_id: id,
                direction: text.source_direction,
                rotation_deg: text.rotation_deg.unwrap_or(transform.rotation_deg),
                line_count: text
                    .text
                    .as_deref()
                    .map(|body| body.lines().count())
                    .unwrap_or(0),
            }),
        }
    }
    (lines, unsupported)
}
```

在 `pipeline/mod.rs` 测试 `unsupported_geometry_warning_is_emitted_once`：同一 node 第一次返回 warning，第二次为空；scene 新增另一个 unsupported node 后只返回新 node；元数据不包含 OCR 正文。

Run: `bun cargo test -p koharu-app unsupported_geometry_warning_is_emitted_once`

Expected: FAIL，driver 去重函数不存在。

**Step 5: 接入每页局部 warning 去重**

在 `pipeline::run` 每个 page 建立 `HashSet<NodeId>`：仅 HanOnly 在进入 engine 循环前扫描一次，之后只在成功 apply 改变 scene 后调用私有 `new_unsupported_geometry()`。只记录首次 node 的 `node_id`、direction、rotation、line_count；不增加 `warning_count`，不使用失败专用 `WarningSink`。engine 返回空 ops 或失败不会改变 scene，因此不重复扫描；OCR 成功 apply 后的扫描覆盖本次新 OCR。

Run: `bun cargo test -p koharu-app pipeline::engines::support::tests`

Run: `bun cargo test -p koharu-app unsupported_geometry_warning_is_emitted_once`

Expected: 全部 PASS。

**Step 6: 提交 Task 2**

```bash
git add crates/koharu-app/Cargo.toml \
  crates/koharu-app/src/pipeline/engines/support.rs \
  crates/koharu-app/src/pipeline/mod.rs
git commit -m "feat(pipeline): centralize safe Han line targets"
```

### Task 3: 统一 Segment、最终 Inpaint Mask、Flux2、DAG 与 UI 分组

**Files:**

- Modify: `crates/koharu-app/src/pipeline/engines/support.rs`
- Modify: `crates/koharu-app/src/pipeline/engines/ctd_segment.rs:1-53`
- Modify: `crates/koharu-app/src/pipeline/engines/lama.rs:20-95`
- Modify: `crates/koharu-app/src/pipeline/engines/aot.rs:20-86`
- Modify: `crates/koharu-app/src/pipeline/engines/flux2_klein.rs:1-82`
- Modify: `crates/koharu-app/src/pipeline/engine.rs:165-205`
- Modify: `crates/koharu-ml/src/inpainting/strategy.rs:119-293,538-750`
- Modify: `crates/koharu-ml/src/flux2_klein/mod.rs:289-450,538-636`
- Modify: `crates/koharu-ml/src/flux2_klein/latents.rs:56-66,114-132`
- Modify: `ui/components/canvas/CanvasToolbar.tsx:104-148`
- Modify: `ui/components/MenuBar.tsx:86-141`
- Test: `ui/tests/components/MenuBar.test.tsx`

**Step 1: 写安全 support rasterization 失败测试**

测试统一以 `line_support_mask_` 开头，覆盖零尺寸、空 lines、正常规范化 polygon、部分越界、完全越界、NaN/Infinity、零面积 polygon，以及纯中文 `line_polygons = None` 的节点 bbox fallback。完全越界、非有限和退化输入必须得到全零 support；部分越界不得 panic且只能在图内产生像素；合法 bbox fallback 必须在裁剪后的节点矩形内产生非零 support。另增加 oversized 原始 quad 的回归测试：先经过 `safe_mixed_line_bbox()` 规范化，再 rasterize，节点外英文 ROI 必须全零。

Run: `bun cargo test -p koharu-app line_support_mask_`

Expected: FAIL，support rasterizer 不存在。

**Step 2: 实现最小 rasterization 与 mask intersection**

`line_support_mask()` 只消费 `EligibleTextLine.region`：存在单个规范化 `line_polygons` 时验证其有限性、图像交集和 clamp 后面积再绘制；`line_polygons = None` 时把有限且正面积的 `region.x/y/width/height` 裁剪到图内并绘制该矩形。其他 polygon 数量视为无效并跳过。混合行的原始 quad 已在 Task 2 丢弃，rasterizer 不再自行信任或扩大它。只使用 `imageproc::drawing::draw_polygon_mut`，不增加 geometry 类型或 trait。

```rust
pub fn intersect_gray_masks(source: &GrayImage, allowed: &GrayImage) -> GrayImage {
    assert_eq!(source.dimensions(), allowed.dimensions());
    GrayImage::from_fn(source.width(), source.height(), |x, y| {
        Luma([if allowed.get_pixel(x, y).0[0] == 0 {
            0
        } else {
            source.get_pixel(x, y).0[0]
        }])
    })
}
```

Run: `bun cargo test -p koharu-app line_support_mask_`

Expected: PASS。

**Step 3: 写共享最终 mask 与生产 dispatch 失败测试**

在 `support.rs` 用表驱动测试覆盖 `expand_mask_for_inpainting` 和 `expand_mask_to_bubble_region_for_inpainting`：旧 mask 同时覆盖英文/中文；准备结果的英文 ROI 为 0、中文 ROI 非零。Repair Brush 断言 expansion 后 region 外为 0，blocks 仍是 all blocks；空 eligible lines、空旧 mask、expansion 后空 mask 都返回 `None`。

在 Lama/AOT/Flux2 各保留一个最小 backend-local dispatch 测试，命名为 `lama_inpaint_dispatch_receives_final_mask`、`aot_inpaint_dispatch_receives_final_mask_and_preserves_repair_region`、`flux2_inpaint_dispatch_receives_final_mask`。测试调用生产 `Engine::run` 所委托的私有 dispatch function，并注入 inference closure 捕获 mask。非空时 closure 收到最终 mask；空最终 mask 时 closure 计数为 0、dispatch 返回输入图。AOT 的 Repair Brush case 在测试模块内定义一个最小 `PaintForward` 实现已有公开 `InpaintForward`，让 closure 调用真实 `run_inpaint()`，同时断言 region/mask 外输出保持输入，从而把准备入口与共享 Original composite 串成一条无模型生产路径。不要重复测试 expansion 算法，不增加 production trait、泛型 Model 或全局 hook。

Run: `bun cargo test -p koharu-app final_inpaint_mask_`

Run: `bun cargo test -p koharu-app inpaint_dispatch_receives_final_mask`

Expected: FAIL，共享入口和 backend dispatch 不存在。

**Step 4: 实现唯一共享 mask 准备入口和空 mask 短路**

```rust
pub fn prepare_inpaint_mask<Expand>(
    mask: &DynamicImage,
    bubble_mask: &DynamicImage,
    all_blocks: &[TextRegion],
    eligible_lines: &[EligibleTextLine],
    policy: SourceTextPolicy,
    region: Option<Region>,
    expand: Expand,
) -> Option<(DynamicImage, Vec<TextRegion>)>
where
    Expand: FnOnce(&DynamicImage, &DynamicImage, &[TextRegion]) -> GrayImage,
{
    let inference_blocks = if region.is_none() && policy == SourceTextPolicy::HanOnly {
        eligible_lines
            .iter()
            .map(|line| line.region.clone())
            .collect::<Vec<_>>()
    } else {
        all_blocks.to_vec()
    };

    let final_mask = if let Some(region) = region {
        let clipped_mask = clip_mask_to_region(mask, &region);
        let clipped_bubble = clip_mask_to_region(bubble_mask, &region);
        let expanded = expand(&clipped_mask, &clipped_bubble, &inference_blocks);
        DynamicImage::ImageLuma8(clip_gray_mask_to_region(&expanded, &region))
    } else if policy == SourceTextPolicy::HanOnly {
        let allowed = line_support_mask(mask.width(), mask.height(), eligible_lines);
        let filtered = DynamicImage::ImageLuma8(intersect_gray_masks(&mask.to_luma8(), &allowed));
        let expanded = expand(&filtered, bubble_mask, &inference_blocks);
        DynamicImage::ImageLuma8(intersect_gray_masks(&expanded, &allowed))
    } else {
        DynamicImage::ImageLuma8(expand(mask, bubble_mask, &inference_blocks))
    };

    final_mask
        .to_luma8()
        .pixels()
        .any(|pixel| pixel.0[0] != 0)
        .then_some((final_mask, inference_blocks))
}
```

三个 backend-local dispatch function 只负责调用该入口并处理 `None => Ok(image.clone())`；`Some((mask, blocks))` 才调用各自 inference closure。`Engine::run` 直接委托一次，不在入口外重新赋值 mask。Lama 使用入口给出的 blocks；AOT/Flux2 忽略 blocks。

Run: `bun cargo test -p koharu-app final_inpaint_mask_`

Run: `bun cargo test -p koharu-app inpaint_dispatch_receives_final_mask`

Expected: PASS，不加载模型。

**Step 5: 写 Segment 最终化与空目标短路失败测试**

测试零 Text、缺 OCR、纯英文、有效混合 polygons、unsupported 混合无 polygons。`finalize_segment_mask()` 测试使用内存 probability mask，直接检查准备写入 blob 的最终 GrayImage；生产 dispatch 使用计数 closure，零 Text、纯英文和 unsupported-only 页面断言 inference 次数为 0。

Run: `bun cargo test -p koharu-app pipeline::engines::ctd_segment::tests`

Expected: FAIL，最终化和模型前 dispatch 尚不存在。

**Step 6: 实现 Segment 生产边界**

```rust
fn finalize_segment_mask(
    image: &DynamicImage,
    probability: &GrayImage,
    regions: &[TextRegion],
    eligible_lines: &[EligibleTextLine],
    policy: SourceTextPolicy,
) -> GrayImage {
    let refined = refine_segmentation_mask(image, probability, regions);
    if policy == SourceTextPolicy::HanOnly {
        let allowed = line_support_mask(refined.width(), refined.height(), eligible_lines);
        intersect_gray_masks(&refined, &allowed)
    } else {
        refined
    }
}
```

`segment_regions(scene, page, policy)` 在模型前执行：零 Text 返回两个空 Vec；存在任一 Text 但 OCR 缺失/空白时返回 `OCR text required before segmentation`；HanOnly 使用 eligible lines；AllText 使用当前所有节点级 regions。一个最小私有 dispatch 先调用 `segment_regions()`：regions 为空时直接返回同尺寸全零 `GrayImage`，只有非空时才调用 inference closure，再调用 `finalize_segment_mask()`。`Engine::run` 只委托该 dispatch。将 segmenter `needs` 从 `TextBoxes` 改为 `OcrText`。

Run: `bun cargo test -p koharu-app pipeline::engines::ctd_segment::tests`

Expected: PASS；最终 GrayImage 的英文 ROI 为 0，unsupported 不进入 regions。

**Step 7: 写共享 HD strategy 最终像素失败测试**

在 `crates/koharu-ml/src/inpainting/strategy.rs` 复用现有 `PaintForward`，增加 `original_strategy_restores_unmasked_pixels`：小图、`HdStrategy::Original`、中心 mask，fake forward 返回全色图，断言 mask 外逐像素等于输入且 mask 内采用生成结果。

Run: `bun cargo test -p koharu-ml original_strategy_restores_unmasked_pixels`

Expected: FAIL，当前 `pad_forward()` 直接返回整图模型输出。AOT 默认 Resize 在小图上同样走该路径。

最小实现：`pad_forward()` 得到模型输出后创建 `image.clone()`，复用现有 `composite_masked(&mut output, &generated, mask, 0, 0)`，只写回 mask 内像素。Lama 已在模型内合成，重复的同 mask composite 行为等价；AOT、Lama、Repair Brush 共用这一根因修复。

Run: `bun cargo test -p koharu-ml original_strategy_restores_unmasked_pixels`

Run: `bun cargo test -p koharu-ml inpainting::strategy::tests`

Expected: PASS；Original、Resize、Crop 的 mask 外像素均保持输入。

**Step 8: 修复 Flux2 内部 mask 与 full-frame 输出边界**

先增加无模型测试。提取一个生产实际调用的私有 `dispatch_inpaint_with_reference()`；它接收 image、mask、options 和一个最小 `FnMut(&DynamicImage, &DynamicImage) -> Result<DynamicImage>` 生成 closure，负责 crop/full-frame 选择及最终 composite。`Flux2Klein::inpaint_with_reference()` 只做参数校验后委托该函数，closure 内调用现有 `inpaint_full_frame()`；不新增 trait、测试 hook 或第二套路径。

```rust
fn dispatch_inpaint_with_reference<Generate>(
    image: &DynamicImage,
    mask: &DynamicImage,
    options: &Flux2InpaintOptions,
    mut generate: Generate,
) -> Result<DynamicImage>
where
    Generate: FnMut(&DynamicImage, &DynamicImage) -> Result<DynamicImage>,
{
    if let Some(bounds) = inpaint_crop_bounds(image, mask, options.mask_padding) {
        let image_crop = image.crop_imm(bounds.x, bounds.y, bounds.width, bounds.height);
        let mask_crop = mask.crop_imm(bounds.x, bounds.y, bounds.width, bounds.height);
        let generated = generate(&image_crop, &mask_crop)?;
        return composite_inpaint_crop(image, &generated, &mask_crop, bounds);
    }

    let generated = generate(image, mask)?;
    composite_inpaint_crop(
        image,
        &generated,
        mask,
        CropBounds {
            x: 0,
            y: 0,
            width: image.width(),
            height: image.height(),
        },
    )
}
```

公开方法的唯一生成调用为：

```rust
dispatch_inpaint_with_reference(image, mask, options, |frame, frame_mask| {
    self.inpaint_full_frame(frame, frame_mask, reference_image, options)
})
```

- `flux2_mask_resize_does_not_expand_support`：`prepare_mask()` 和 packed-mask resize 使用 `FilterType::Nearest`，不产生 Triangle interpolation halo。
- `flux2_mask_full_frame_composite_preserves_unmasked_pixels`：调用生产 `dispatch_inpaint_with_reference()`，使用会令 crop bounds 覆盖全图的非空 mask 和返回全白图的计数 closure；断言 closure 恰好一次收到全图、mask 外逐像素等于原图。
- `flux2_mask_crop_composite_preserves_unmasked_pixels`：同样调用生产 dispatch，使用中心小 mask；断言 closure 收到小于原图的 crop，最终 mask 外逐像素等于原图。

Run: `bun cargo test -p koharu-ml flux2_mask_`

Expected: FAIL，当前使用 Triangle 且 full-frame 直接返回 generated。

最小实现：

1. `latents.rs` 两处 mask resize 改为 `FilterType::Nearest`；图像 resize 不变。
2. app 的 Flux2 dispatch 使用：

```rust
let options = Flux2InpaintOptions {
    mask_padding: 0,
    ..Default::default()
};
```

3. `dispatch_inpaint_with_reference()` 的 crop 分支调用 closure 后继续使用现有 `composite_inpaint_crop()` 和 `mask_crop`；full-frame 分支调用 closure 后把全图 `CropBounds` 与调用方原 mask 传给同一 helper。公开 `inpaint_with_reference()` 不再保留独立 crop/full-frame 分支，因此测试绕过 dispatch 或生产绕过 composite 都会直接暴露。latent 分辨率无法表达的边缘误差也不会改变最终 mask 外像素。

Run: `bun cargo test -p koharu-ml flux2_mask_`

Run: `bun cargo test -p koharu-app flux2_inpaint_dispatch_receives_final_mask`

Expected: PASS；默认 CLI 工具仍可显式设置自己的 `mask_padding`，Koharu app 固定为 0。

**Step 9: 写 DAG 相对顺序失败测试**

在 `engine.rs` 用真实 registry ids 测试：OCR 在 segmenter 前；translator 在 Lama/AOT/Flux2 前；未选择 translator 时三个 inpainter 仍保留在 order；Repair Brush 单 engine 仍可运行。不要断言互不依赖 engine 的顺序。

Run: `bun cargo test -p koharu-app pipeline::engine::tests::orders_`

Expected: FAIL，segmenter 和 Inpainter 尚未声明新 artifact needs。

**Step 10: 建立 DAG 边**

给三个 Inpainter 的 `needs` 增加 `Artifact::Translations`。`build_order()` 只在已选 producer 存在时添加边，因此 standalone Inpaint/Repair Brush 不受阻。

Run: `bun cargo test -p koharu-app pipeline::engine::tests::orders_`

Expected: PASS。

**Step 11: 先写 UI 请求失败测试，再修正两个调用点**

不创建 UI helper 或独立 helper 测试文件：

- `CanvasToolbar`：Detect 为 detector + bubble segmenter + font detector；OCR 为 OCR + segmenter。
- `MenuBar`：Full Pipeline 仍列出所有 engines，但注释说明 DAG 决定顺序；custom Detect 不含 segmenter，custom OCR 同时加入 OCR + segmenter。
- 删除“backend 会跳过已满足 artifact”的失实注释。

先在现有 `MenuBar.test.tsx` 增加 custom Detect/OCR toggle 请求断言。

Run: `bun run --filter ui test -- ui/tests/components/MenuBar.test.tsx`

Expected: FAIL，custom Detect 仍错误包含 segmenter，custom OCR 尚未包含 segmenter。

随后直接修改两个调用点；Toolbar 的短数组改动由 TypeScript、现有 UI suite 和代码审查覆盖，不新增第二套抽象测试。

Run: `bun run --filter ui test -- ui/tests/components/MenuBar.test.tsx`

Expected: PASS。

**Step 12: 提交 Task 3**

```bash
git add crates/koharu-app/src/pipeline/engines/support.rs \
  crates/koharu-app/src/pipeline/engines/ctd_segment.rs \
  crates/koharu-app/src/pipeline/engines/lama.rs \
  crates/koharu-app/src/pipeline/engines/aot.rs \
  crates/koharu-app/src/pipeline/engines/flux2_klein.rs \
  crates/koharu-app/src/pipeline/engine.rs \
  crates/koharu-ml/src/inpainting/strategy.rs \
  crates/koharu-ml/src/flux2_klein/mod.rs \
  crates/koharu-ml/src/flux2_klein/latents.rs \
  ui/components/canvas/CanvasToolbar.tsx \
  ui/components/MenuBar.tsx ui/tests/components/MenuBar.test.tsx
git commit -m "fix(pipeline): protect Han-only inpaint regions"
```

### Task 4: 严格逐行翻译与 CLI 两阶段 fallback

**Files:**

- Modify: `crates/koharu-app/src/llm.rs:200-248,435-538`
- Modify: `crates/koharu-app/src/pipeline/engines/support.rs`
- Modify: `crates/koharu-app/src/pipeline/engines/llm_translate.rs:1-166`
- Modify: `crates/koharu-app/bin/pipeline.rs:198-281,354-425`
- Test: 上述四个文件的现有测试模块

**Step 1: 写 strict tagged parser 失败测试**

测试统一以 `strict_tagged_blocks_` 开头：完整标签、乱序恢复成功；无标签、首标签前正文、非法/重复/缺失/越界 tag、空 block、一个 block 多逻辑行全部失败。

Run: `bun cargo test -p koharu-app strict_tagged_blocks_`

Expected: FAIL，strict parser 不存在。

**Step 2: 实现 strict parser，保留 lenient 路径**

复用现有 `parse_block_tag()` 和 `find_next_tag()`，使用 `Vec<Option<String>>` slots；返回值按 tag index，而不是响应出现顺序。现有 `parse_tagged_blocks()` 和 `split_legacy_lines()` 不改，继续服务 AllText。

```rust
fn parse_tagged_blocks_strict(
    translation: &str,
    expected: usize,
) -> anyhow::Result<Vec<String>> {
    for line in translation.lines().map(str::trim_start).filter(|line| line.starts_with('[')) {
        anyhow::ensure!(parse_block_tag(line).is_some(), "invalid translation tag");
    }
    let mut slots = vec![None; expected];
    let mut cursor = translation;
    let mut found = false;
    while let Some((offset, len, id)) = find_next_tag(cursor) {
        if !found {
            anyhow::ensure!(cursor[..offset].trim().is_empty(), "content before first tag");
        }
        found = true;
        anyhow::ensure!(id < expected, "translation tag out of range");
        anyhow::ensure!(slots[id].is_none(), "duplicate translation tag");
        cursor = &cursor[offset + len..];
        let end = find_next_tag(cursor).map(|(offset, _, _)| offset).unwrap_or(cursor.len());
        let content = cursor[..end].trim();
        anyhow::ensure!(!content.is_empty(), "empty translation block");
        anyhow::ensure!(content.lines().count() == 1, "translation block must contain one line");
        slots[id] = Some(content.to_string());
        cursor = &cursor[end..];
    }
    anyhow::ensure!(found, "tagged translation response required");
    anyhow::ensure!(slots.iter().all(Option::is_some), "missing translation tag");
    Ok(slots.into_iter().map(Option::unwrap).collect())
}
```

Run: `bun cargo test -p koharu-app strict_tagged_blocks_`

Expected: PASS。

**Step 3: 写 strict Provider 后处理与空 sources 失败测试**

测试统一以 `strict_translation_` 开头：thinking 清理后解析、wrapping quotes 最后清理、未闭合 `<think>` 失败；`strict_translation_empty_sources_short_circuits` 直接把 `RwLock::new(State::Empty)` 传给生产实际调用的私有 strict orchestration，空 sources 返回空 Vec 而不是 `no LLM loaded`，证明在 state lock/Provider generate 前短路，不构造 RuntimeManager/LlamaBackend，不增加 provider fake 或全局 hook。

Run: `bun cargo test -p koharu-app strict_translation_`

Expected: FAIL，strict orchestration 尚不存在。

**Step 4: 固定 Provider 后处理顺序**

提取私有 `generate_translation(state: &mut State, ...)`，只负责现有 local/provider match，供 AllText 和 strict 路径复用。新增 `parse_strict_translation_response()`，以及接收 `&RwLock<State>` 的私有 strict orchestration；它的第一条分支必须是 `if sources.is_empty() { return Ok(Vec::new()); }`，之后才 lock state，并固定为 generate → strip thinking → strict parse → 每项 strip wrapping quotes。公开给 pipeline 使用的 `translate_text_lines_strict()` 只委托该函数。未闭合 `<think>` 由 strict parser 明确失败。

Run: `bun cargo test -p koharu-app strict_translation_`

Expected: PASS；AllText 旧测试同时通过。

**Step 5: 写 HanOnly 原子 op builder 失败测试**

在 `support.rs` 增加共享生产函数测试，统一以 `han_translation_ops_` 开头：

- translations 数量不等于 targets 时返回 Err，不返回部分 ops。
- 拒绝重复 `(node_id, line_index)`；先把已经按 tag 恢复的 translation 与对应 target zip，再按 `(node_id, line_index)` 对 pair 排序和回组，禁止分别排序导致错配。
- eligible 节点写 translation 并清 sprite/sprite_transform。
- 纯英文和 unsupported in-scope 节点清 translation/sprite/sprite_transform。
- `text_node_ids` 外节点完全无 ops。
- targets 为空仍返回必要 cleanup。

函数签名固定为 `build_han_only_translation_ops(scene: &Scene, page: PageId, allowed_ids: Option<&[NodeId]>, targets: &[(NodeId, EligibleTextLine)], translations: &[String]) -> anyhow::Result<Vec<Op>>`。

Run: `bun cargo test -p koharu-app han_translation_ops_`

Expected: FAIL，builder 不存在。

**Step 6: 接入 HanOnly/AllText 翻译分支**

- AllText 保留当前 `collect_translation_targets_from()`、`translate_texts()`、节点级写回和 legacy fallback。
- HanOnly 先从 page collector 取得 targets，再按 `text_node_ids` 过滤；targets 为空时直接以空 translations 调用 builder 做 cleanup，不调用 LLM；非空时只把 eligible line source 发送给 `translate_text_lines_strict()`。
- `let translations = ...await?;` 必须在 `build_han_only_translation_ops()` 之前；Provider 或 parser Err 时尚未构造任何 ops。
- unsupported 不进入请求；builder 负责其 cleanup。

Run: `bun cargo test -p koharu-app pipeline::engines::llm_translate::tests`

Expected: PASS。

**Step 7: 写 CLI 阶段拆分、translator 识别与失败门禁测试**

测试统一以 `cli_fallback_` 开头：

- selected engines 没有 `Artifact::Translations` producer 且含 `Artifact::FinalRender` producer 时，renderer 被移到第二阶段。
- renderer-only 输入允许第一阶段为空。
- selected engines 含 translation producer 时不拆分；显式 `--steps llm,...` 即使没有 `--with-translate` 也不得 fallback。
- `RunOutcome { warning_count: 1 }` 由 `require_clean_phase()` 返回 Err；零 warning 返回 Ok。
- HanOnly fallback 复用 `build_han_only_translation_ops()`，只写 Han lines 并清英文旧字段。
- AllText 继续把完整节点 OCR 复制到 translation。

```rust
fn split_render_phase(steps: Vec<String>) -> anyhow::Result<(Vec<String>, Vec<String>)> {
    let infos = steps
        .iter()
        .map(|id| Registry::find(id))
        .collect::<anyhow::Result<Vec<_>>>()?;
    if infos.iter().any(|info| info.produces.contains(&Artifact::Translations)) {
        return Ok((steps, Vec::new()));
    }
    let (render, first): (Vec<_>, Vec<_>) = steps
        .into_iter()
        .zip(infos)
        .partition(|(_, info)| info.produces.contains(&Artifact::FinalRender));
    Ok((
        first.into_iter().map(|(id, _)| id).collect(),
        render.into_iter().map(|(id, _)| id).collect(),
    ))
}
```

`require_clean_phase(outcome: &RunOutcome) -> anyhow::Result<()>` 只执行 `ensure!(outcome.warning_count == 0, "pipeline phase failed")`；不增加状态 enum。

Run: `bun cargo test -p koharu-app --bin pipeline cli_fallback_`

Expected: FAIL，阶段拆分和策略 fallback 不存在。

**Step 8: 实现 CLI 两阶段流程**

先验证原始 steps 非空，再拆分：

1. 用 `split_render_phase()` 从真实 selected engine artifacts 决定是否需要 fallback；不再用 `cli.with_translate` 作为事实来源。
2. 第一阶段非空才运行 detector/OCR/segment/inpaint 等 steps；返回后立即调用 `require_clean_phase()`。
3. 第一阶段 clean 后，无 translation producer 且存在第二阶段 renderer 时执行策略 fallback。
4. HanOnly 使用 page collector 的 sources 调用共享 op builder；AllText 使用当前节点级复制逻辑。
5. fallback apply 成功后才运行第二阶段 renderer；第二阶段返回后同样调用 `require_clean_phase()`。
6. 任一 phase Err 或非零 warning 都立即返回，不继续后续动作；两次 run 分别使用新的 cancel flag。成功返回时 warning_count 必为 0，无需维护额外合并状态。

Run: `bun cargo test -p koharu-app --bin pipeline cli_fallback_`

Run: `bun cargo check -p koharu-app --all-targets`

Expected: PASS。

**Step 9: 提交 Task 4**

```bash
git add crates/koharu-app/src/llm.rs \
  crates/koharu-app/src/pipeline/engines/support.rs \
  crates/koharu-app/src/pipeline/engines/llm_translate.rs \
  crates/koharu-app/bin/pipeline.rs
git commit -m "fix(translation): enforce atomic Han line mapping"
```

### Task 5: 限制 Renderer 几何并复用 hard-line layout

**Files:**

- Modify: `crates/koharu-app/src/pipeline/engines/renderer.rs:27-140`
- Modify: `crates/koharu-app/src/renderer.rs:40-55,214-340,491-690,731-793`
- Test: 上述两个文件的现有测试模块

**Step 1: 写 Renderer 输入失败测试**

测试统一以 `han_only_renderer_` 开头：

- 纯英文和 unsupported 不创建 input，并清旧 translation/sprite/sprite_transform。
- eligible 节点创建 input，渲染前清旧 sprite/sprite_transform。
- `text_node_ids` 外节点无 input、无 cleanup。
- 有效轴对齐混合 polygons 使用中文行 bbox 并设置 `lock_layout_box = true`；非零 rotation/旋转/斜切混合节点已由 Task 2 标记 unsupported，不创建 input。
- translation 非空行数不等于 eligible lines 时，在 `render_page()` 前返回 Err 且没有 ops。
- AllText 保持原 transform、lock 值、full-page scope 和通用布局。

Run: `bun cargo test -p koharu-app han_only_renderer_`

Expected: FAIL，当前输入仍只检查非空 translation。

**Step 2: 实现最小输入分支**

给 `RenderBlockInput` 增加 `preserve_explicit_lines: bool`：AllText 为 false；HanOnly 为 true。HanOnly 按 node 分组 eligible lines，transform 使用中文行安全 bbox 并集并固定 `rotation_deg = 0.0`；eligible 行数少于 OCR 非空行数时锁定 layout box。输入构造返回 `(Vec<RenderBlockInput>, Vec<Op>)`，不新增状态 enum。`render_page()` 成功后先放 cleanup ops，再追加新 sprite ops；单个 block 渲染失败时 cleanup 仍清掉旧 sprite，整体 Err 时 driver 不 apply，保持事务原子性。

Run: `bun cargo test -p koharu-app han_only_renderer_`

Expected: PASS。

**Step 3: 写水平、垂直与 Bubble hard-line 失败测试**

复用现有 `any_system_font()`，测试统一以 `preserves_explicit_lines_` 开头：

- Horizontal 普通路径：长单行仍一行，两行输入仍两行，无 discretionary hyphen。
- VerticalRl 普通路径：长单行仍一列，两行输入仍两列。
- Bubble collision：Horizontal 和 VerticalRl 各覆盖单行/双行。
- Bubble mask 存在且 mixed block 被 lock 时，layout box 仍是中文 bbox，显式行数不变。

synthetic hyphen 判定继续使用 `glyph.cluster as usize == line.range.end`，不引用不存在的字段。

Run: `bun cargo test -p koharu-app preserves_explicit_lines_`

Expected: FAIL，当前两个方向都会使用一个有限 max extent。

**Step 4: 复用唯一 layout-at-size helper**

```rust
fn run_layout_at<'a>(
    builder: &TextLayout<'a>,
    text: &str,
    layout_box: LayoutBox,
    font_size: f32,
    preserve_explicit_lines: bool,
) -> Result<LayoutRun<'a>> {
    let layout = builder.clone().with_font_size(font_size.max(1.0));
    if preserve_explicit_lines {
        layout.without_hyphenation().run(text)
    } else {
        layout
            .with_max_width(layout_box.width.max(1.0))
            .with_max_height(layout_box.height.max(1.0))
            .run(text)
    }
}
```

`fit_font_size()`、`run_collision_layout_at()` 都调用该函数；`preserve_explicit_lines` 穿过 collision 的全部内部调用。hard-line 不设置 max width/height，外层现有 `layout.width/height` 比较继续驱动二分。显式字号仍直接运行，最小字号和 collision fallback 保持原行为。不要新增 `fit_explicit_lines` 或手工合并 `LayoutRun`。

Run: `bun cargo test -p koharu-app preserves_explicit_lines_`

Run: `bun cargo test -p koharu-renderer layout::tests`

Expected: 全部 PASS；AllText 的现有断词行为不变。

**Step 5: 提交 Task 5**

```bash
git add crates/koharu-app/src/pipeline/engines/renderer.rs \
  crates/koharu-app/src/renderer.rs
git commit -m "fix(renderer): preserve explicit Han line layout"
```

### Task 6: 文档、全量门禁与真实图片验收

**Files:**

- Modify: `docs/zh-CN/project-functional-analysis.md`
- Verify only: 当前本地 `image-test` 项目及问题图片

Task 6 不新增前面任务的纯函数测试。

**Step 1: 更新中文限制说明**

写明：默认 `source_text_policy = "han_only"`；服务端 `config.toml` 可切换 `all_text`；混合节点缺少安全轴对齐逐行 polygons、存在非零 rotation、旋转或斜切 geometry 时安全跳过，不做等高或旋转猜测；旧项目必须从 Source 完整重跑；仅重新 Render 无法恢复已被旧 Inpainted 擦除的英文。

**Step 2: 运行 Rust 全量门禁**

```bash
bun cargo fmt --all -- --check
bun cargo check --workspace --all-targets
bun cargo clippy --workspace --all-targets -- -D warnings
bun cargo test --workspace --tests
```

Expected: 全部退出码 0；默认测试不下载模型。

**Step 3: 运行 UI、生成物与桌面门禁**

Task 1 生成物已经提交后再运行：

```bash
bun run format:check
bun run lint:ui
bun run test:ui
bun run check:generated
bun run build
```

Expected: 全部退出码 0；`bun run build` 只记录为普通 Tauri build。

Apple Silicon macOS 额外运行：

```bash
bun cargo check -p koharu --all-targets --features=metal
bun cargo build --release -p koharu --features=metal
```

Expected: 退出码 0；其他平台明确跳过，不用 CUDA 代替。

**Step 4: 从 Source 完整重跑问题图片**

完整流程包含 detector、bubble/font、OCR、segmenter、translator、inpainter、renderer；DAG 顺序由 Task 3 无模型测试证明。检查：

- Segment Mask：英文 ROI 全黑，只有可靠中文目标非零。
- Inpainted：Lama、AOT 或 Flux2 当前所选 backend 的 mask 外与输入基面逐像素相同；中文已移除。Repair Brush 另检查 region 外逐像素不变。
- Rendered：英文只保留 Source 中的一份，译文只出现在中文行区域。
- `Full-Body Sculpting`、`Enjoy a Confident Body` 不新增软换行或 `Confi-\ndent`。
- 缺 polygons 的混合节点被安全跳过并仅记录一次不含正文的 warning。

保存 Source、Segment Mask、Inpainted、Rendered 截图作为本地证据，不提交图片，不新增调试图层。

**Step 5: 提交文档**

```bash
git add docs/zh-CN/project-functional-analysis.md
git commit -m "docs: document Han-only pipeline limits"
```

## 最终验收清单

- [ ] HTTP、MCP、CLI、Repair Brush 均继承服务端策略。
- [ ] `StartPipelineRequest` 和 HTTP 路径不变；`/config` 只增加响应字段/schema。
- [ ] 混合节点缺少有效逐行 polygons 时返回 unsupported，不做等高切分。
- [ ] 混合节点的非有限、退化、完全越界、非零 rotation、旋转或斜切 geometry 在共享行解析边界返回 unsupported；合法轴对齐 quad 被规范化到节点与图像交集，原始越界 quad 不再进入后续阶段。
- [ ] unsupported 不进入 Segment、Translate、Inpaint 或 Renderer，且 warning 不含 OCR 正文。
- [ ] HanOnly 的旧 Segment Mask、refined Segment Mask 和 expansion 后 mask 都被中文 support 限制。
- [ ] 完全越界、非有限、退化 quad 不产生 support；部分越界只在节点与图像交集内产生像素；纯中文无 polygons 时使用合法节点 bbox fallback 产生非零 support。
- [ ] Lama、AOT、Flux2 的生产 dispatch 都经过共享最终 mask 入口。
- [ ] 纯英文、unsupported-only、空 strict sources 和空最终 mask 在 inference/Provider closure 前短路，closure 调用次数为 0。
- [ ] Lama、AOT 的 Original/Resize/Crop 及 Flux2 crop/full-frame 输出在最终 mask 外逐像素保持输入；Repair Brush region 外保持输入。
- [ ] Flux2 app 使用 `mask_padding = 0`，mask resize 无 interpolation halo；crop/full-frame 无模型测试均经过生产 `dispatch_inpaint_with_reference()`，输出的 mask 外像素保持原图。
- [ ] Repair Brush 不做语言过滤，expansion 后仍限制到用户 region。
- [ ] HanOnly 每个 eligible line 对应唯一 `(node_id, line_index)` 和 tagged block。
- [ ] 缺失、重复、非法、越界、空、多行 Provider block 在任何 op 和 Inpaint 前失败。
- [ ] `text_node_ids` 限制 HanOnly 请求、回组、cleanup 和 Renderer；scope 外节点不变。
- [ ] 旧 translation、sprite、sprite_transform 按策略清理；Renderer-only 不保留旧 sprite。
- [ ] CLI 无 translator 时先完成策略 fallback，再运行 Renderer；HanOnly 不复制英文。
- [ ] CLI 根据 selected engine artifacts 判断 translator；第一或第二阶段 warning_count 非零时立即失败且不执行后续动作。
- [ ] AllText 保持节点级 Provider、legacy fallback、节点 transform 和通用布局。
- [ ] Detect 为 detector + bubble + font；OCR 为 OCR + segmenter；custom segmenter 只跟随 OCR toggle。
- [ ] DAG 保证 OCR → segmenter，以及选择 translator 时 translator → inpainter；standalone Inpaint/Repair Brush 仍可运行。
- [ ] 零 Text 合法；存在 Text 但 OCR 缺失时 Segment 在模型前明确失败。
- [ ] Horizontal、VerticalRl、Bubble 和非 Bubble 的 hard-line 都只保留显式行数且无 discretionary hyphen。
- [ ] 全部 Rust、UI、生成物、普通 Tauri 与 macOS Metal 门禁通过或按平台明确跳过。
- [ ] 当前问题图已从 Source 完整重跑并通过像素与视觉验收。

只有以上项目全部满足，本计划才算执行完成。
