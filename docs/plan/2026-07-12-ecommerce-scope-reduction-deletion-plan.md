# 电商图片翻译安全删减 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 删除已确认无生产消费者的重复 ML 实现和诊断程序，并按电商产品范围收缩低价值编辑入口，同时保持全部生产 Engine、Codex、Inpainter、Provider、自动化入口和旧项目行为不变。

**Architecture:** 本计划把“真实重复/零生产引用”与“产品范围删减”分开处理，不包含模型优选、Engine 配置修复或异步任务架构替换。生产 Detector、OCR、Font Detector、Google Fonts、MCP、Codex、Lama/AOT/Flux2、Speech Bubble 和 BubbleMask 全部保留；旧 Scene wire enum 与 HTTP Pipeline schema 不修改。

**Tech Stack:** Rust 2024、Cargo workspace、Axum/Utoipa、Postcard Scene/Op、React 19、TypeScript、Zustand、Vitest、Bun、Tauri。

---

## 1. 封闭范围

### 1.1 真实重复或零生产引用

- `koharu-ml` 的 14 个独立模型诊断 bin 和 `koharu-llm` 的 3 个独立诊断 bin。
- `koharu-ml` 中未被 App 使用的第二套 PaddleOCR-VL，以及未接入 Pipeline 的 Manga Text Segmentation 2025。

### 1.2 产品范围删减

- MenuBar 的 Custom Pipeline toggle、Run Custom Current/All 和对应前端 preference。
- 普通彩色 Brush 的新建、绘制、显示和快捷键 UI。
- `/api/v1/pages/{id}/image-layers` Custom image 新建 API，但仅在调用清单证明无外部消费者后执行。

上述三项不是完全重复实现：Custom Pipeline 提供持久化任意阶段子集，彩色 Brush 写入像素 overlay，Custom image API 创建 Scene 图层。本计划删除它们的依据是电商产品范围和消费者证据，不得把它们描述为重复核心能力。

### 1.3 明确保留

- Codex 整页生成、登录设置、AI 面板、`koharu-ai` crate、Codex CLI 和 `/api/v1/ai/codex/*`。
- Lama、AOT、Flux2 Klein 三种 Inpainter 及其 App Engine、ML 模块、模型 inventory、配置和测试。
- Speech Bubble segmentation、BubbleMask、Bubble Renderer 布局和当前 DAG 依赖。
- PP-DocLayout、Comic Text Detector、Comic Text Bubble、Anime Text；PaddleOCR、Manga OCR、MIT48px OCR；YuzuMarker Font Detection。
- Google Fonts catalog、下载、缓存和字体选择 UI。
- MCP、`rmcp`、HTTP、SSE、Jobs、产品 `pipeline` CLI 和 OpenAPI generator。
- 自定义 system prompt UI、`customSystemPrompt` preference，以及 HTTP/MCP/CLI 的 `system_prompt`。
- Engine/Provider 高级设置和 `StartPipelineRequest.steps`；删除 Custom Pipeline UI 不删除后端自定义步骤能力。
- TextBlocks 手工编辑、Segment Eraser、Repair Brush、Scene/Op/history/undo/KHR、PNG/JPEG/WebP/PSD/KHR 导出。
- `ImageRole::Custom`、`MaskRole::BrushInpaint` 和旧项目中的对应节点。

### 1.4 明确排除

- 不在本计划修复 `comic-text-detector` 同时进入 Detector/Segmenter 时的重复 Engine ID 或多 SegmentMask producer 问题；保留 `comic-text-detector` 与 `comic-text-detector-seg`，另行建立行为修复任务。
- 不合并 HTTP Pipeline、Codex 和 MCP 的异步任务生命周期；Jobs、cancel、SSE 和 MCP 行为保持原样。
- 不为 MenuBar、CanvasToolbar、TextBlocksPanel 的请求拼装新增共享 helper；删除 Custom Pipeline 后仍有实际重复且产生维护问题时再处理。
- 不抽取 Detector/OCR 的 Scene Op helper，不删除任何可插拔 Detector、OCR、Inpainter 或 Provider。

### 1.5 停止条件

任何 Task 出现以下情况时停止该 Task，不继续连带删除：

- 旧 `.khr`/history 无法打开或旧 BrushInpaint 不再进入 Renderer composite；
- Codex UI/API、任一生产 Engine、Google Fonts 或 MCP 发生 diff；
- `StartPipelineRequest`、`/pipelines`、SSE、Jobs、Provider 或 Engine config schema 改变；
- HanOnly 纯英文进入 Segment/Provider/Inpaint/Renderer；
- Segment Eraser 不再编辑 Segment Mask，或 Repair Brush region、mask 外像素、AllText 兼容行为回归；
- 默认测试开始下载模型。
- 仓库脚本、CI、发布流程或已登记的外部人工流程仍调用计划删除的 17 个诊断 bin。

## 2. 删除后目标结构

不创建新抽象或替代实现，只收缩已有目录：

```text
crates/
  koharu-ai/                         # 完整保留
  koharu-app/
    bin/pipeline.rs                  # 保留产品 CLI
    src/pipeline/engines/            # 所有生产 Engine 保留
  koharu-llm/
    src/                             # 保留 LLM/Provider/PaddleOCR 生产库
    # 删除 bin/
  koharu-ml/
    src/                             # 删除重复 PaddleOCR 与 Manga Seg 2025
    tests/                           # 删除上述两个未使用模块的模型测试
    # 删除 bin/
  koharu-rpc/
    src/mcp/                         # 完整保留
    bin/openapi.rs                   # 保留生成器
ui/
  components/                        # 保留 AI、阶段按钮、prompt、Engine/Provider 设置
  hooks/                             # 删除普通 Brush 专用 hooks
```

## 3. 执行顺序

- **Wave A，可立即执行：** Task 0、Task 2、Task 3。
- **Wave A，有消费者证据后执行：** Task 1；仓库与已登记外部流程均无诊断 bin 消费者时才能删除。
- **Wave B，兼容证据后执行：** Task 4；没有调用证据时标记 NOT APPLICABLE。
- **最终验收：** Task 5。

每个 Task 独立提交。当前工作树已有 `Cargo.toml`、Renderer、CUDA 和其他 `docs/plan/` 修改，执行前必须先隔离或完成这些修改，删除提交不得混入无关 diff。

### Task 0：冻结保留行为与工作树基线

**Files:**

- Read: `docs/plan/2026-07-12-ecommerce-image-translation-productization-roadmap.md`
- Read: `docs/plan/2026-07-11-ecommerce-chinese-only-translation-quality-plan.md`
- Test: `crates/koharu-app/src/session.rs`
- Test: `crates/koharu-app/src/pipeline/mod.rs`
- Test: `crates/koharu-app/src/pipeline/engines/renderer.rs`

**Step 1: 记录工作树、空间和 binary 基线**

```bash
git status --short
du -sh target ui/node_modules 2>/dev/null || true
cargo metadata --no-deps --format-version 1 > /tmp/koharu-metadata-before.json
```

Expected: 明确记录已有 dirty files；本 Task 不清理用户文件。

**Step 2: 运行核心保留回归**

```bash
bun cargo test -p koharu-app eligible_mixed_node_
bun cargo test -p koharu-app han_only_renderer_
bun cargo test -p koharu-app strict_translation_
bun cargo test -p koharu-app orders_translator_before_inpainters
bun cargo test -p koharu-app orders_inpainters_without_translator
bun cargo test -p koharu-app lama_inpaint_dispatch_receives_final_mask
bun cargo test -p koharu-app aot_inpaint_dispatch_receives_final_mask_and_preserves_repair_region
bun cargo test -p koharu-app flux2_inpaint_dispatch_receives_final_mask
bun cargo test -p koharu-rpc --test openapi
bun run test:ui
```

Expected: PASS；上述命令均不加载模型。

**Step 3: 增加三个 characterization tests**

在 `session.rs` 增加 `legacy_optional_layers_round_trip_before_scope_reduction`：创建并重开含 Source、Rendered、Custom image、Segment/Bubble/BrushInpaint mask 和带 sprite 的 Text 节点的项目，断言节点角色与 blob ref 不变。

在 Pipeline Renderer 现有 tests 中增加 `legacy_brush_layer_still_composites_without_editor_ui`：使用内存 RGBA base/brush 调用生产 `dispatch_render_page()`，断言 brush 像素出现在 `final_render`，且空 Han target 不调用 renderer closure。

在 `pipeline/mod.rs` 现有 tests 中增加 `registry_retains_all_production_engine_ids`：表驱动调用 `Registry::find()`，断言以下 15 个生产 ID 均已注册：`anime-text`、`aot-inpainting`、`speech-bubble-segmentation`、`comic-text-bubble-detector`、`comic-text-detector`、`comic-text-detector-seg`、`flux2-klein`、`lama-manga`、`llm`、`manga-ocr`、`mit48px-ocr`、`paddle-ocr-vl-1.6`、`pp-doclayout-v3`、`koharu-renderer`、`yuzumarker-font-detection`。只锁定 ID 存在，不断言 CTD 的跨类别配置行为。

Run:

```bash
bun cargo test -p koharu-app legacy_optional_layers_round_trip_before_scope_reduction
bun cargo test -p koharu-app legacy_brush_layer_still_composites_without_editor_ui
bun cargo test -p koharu-app registry_retains_all_production_engine_ids
```

Expected: PASS；在后续删除前先建立行为锁，不要求制造失败。

**Step 4: Commit**

```bash
git add crates/koharu-app/src/session.rs \
  crates/koharu-app/src/pipeline/mod.rs \
  crates/koharu-app/src/pipeline/engines/renderer.rs
git commit -m "test(project): lock retained production behavior"
```

若当前 Renderer dirty diff 尚未完成，不得暂存该文件；先在独立 worktree 执行本计划。

### Task 1：删除诊断 bin 和零生产引用 ML 模块

**Files:**

- Modify: `crates/koharu-ml/Cargo.toml`
- Modify: `crates/koharu-llm/Cargo.toml`
- Modify: `crates/koharu-ml/src/lib.rs`
- Delete: `crates/koharu-ml/bin/`
- Delete: `crates/koharu-llm/bin/`
- Delete: `crates/koharu-ml/src/paddleocr_vl/`
- Delete: `crates/koharu-ml/src/manga_text_segmentation_2025/`
- Delete: `crates/koharu-ml/tests/ocr.rs`
- Delete: `crates/koharu-ml/tests/manga_text_segmentation_2025.rs`

**Step 1: 证明生产调用边界**

```bash
rg -n "koharu_ml::paddleocr_vl|manga_text_segmentation_2025" crates \
  --glob '!crates/koharu-ml/bin/**' --glob '!crates/koharu-ml/tests/**'
rg -n "koharu_llm::paddleocr_vl" crates/koharu-app/src/pipeline/engines/paddle_ocr.rs
if rg -n --hidden --glob '!.git/**' --glob '.github/**' --glob 'scripts/**' --glob 'docs/**' \
  --glob '!docs/plan/**' \
  --glob 'package.json' --glob 'ui/package.json' -- \
  '--bin (comic-text-detector|comic-text-bubble-detector|lama|manga-ocr|paddleocr-vl|mit48px-ocr|font-detect|pp-doclayout-v3|manga-text-segmentation-2025|speech-bubble-segmentation|anime-text|aot-inpainting|flux2-klein|flux2-klein-prompt|llama|llm|paddleocr-vl-llm)\b' .; then
  echo "diagnostic bin still has a repository consumer" >&2
  exit 1
fi
```

Expected: 第一条只命中 `crates/koharu-ml/src/lib.rs`；第二条命中生产 PaddleOCR Engine；第三段无命中。执行者还必须记录发布/CI/人工模型诊断流程的外部消费者确认；无法确认时停止 Task 1，不以仓库零引用代替外部证据。

**Step 2: 运行保留库检查**

```bash
bun cargo check -p koharu-app --lib
bun cargo check -p koharu-ml --lib --tests
bun cargo check -p koharu-llm --lib --tests
```

Expected: PASS。

**Step 3: 执行最小删除**

- 从两个 Cargo manifest 删除对应 17 个 `[[bin]]` 声明，再删除两个 `bin/` 目录。
- 删除 `koharu-ml::paddleocr_vl` 导出、实现和 `crates/koharu-ml/tests/ocr.rs`；App 继续使用 `koharu-llm::paddleocr_vl`。
- 删除 `manga_text_segmentation_2025` 导出、实现和测试。
- 仅当 `rg` 证明非 bin 源码不再使用时才删 dependency；`koharu-ml` 的 `clap` 和 `koharu-llm` 的 `tracing-subscriber` 明确保留。
- 不修改 `koharu-ai/bin/codex.rs`；Lama/AOT/Flux2 的独立诊断 bin 会随 `koharu-ml/bin/` 删除，但其 library module、生产 Engine、模型 inventory 和测试全部保留。

**Step 4: 验证 binary 列表和 workspace**

```bash
cargo metadata --no-deps --format-version 1 > /tmp/koharu-metadata-after.json
python3 - <<'PY'
import json
from pathlib import Path

metadata = json.loads(Path('/tmp/koharu-metadata-after.json').read_text())
targets = {
    (package['name'], target['name'])
    for package in metadata['packages']
    for target in package['targets']
    if 'bin' in target['kind']
}
removed_packages = {'koharu-ml', 'koharu-llm'}
assert not any(package in removed_packages for package, _ in targets), targets
assert ('koharu-app', 'pipeline') in targets
assert ('koharu-rpc', 'openapi') in targets
assert ('koharu-ai', 'codex') in targets
print(sorted(targets))
PY
if rg -n "PaddleOCR-VL-1\.5|model:paddleocr-vl-candle" crates; then
  echo "stale PaddleOCR-VL 1.5 package remains" >&2
  exit 1
fi
rg -n "PaddleOCR-VL-1\.6-GGUF|model:paddleocr-vl-1\.6" \
  crates/koharu-llm/src/paddleocr_vl.rs
if rg -n "manga_text_segmentation_2025|MangaTextSegmentation" crates; then
  echo "stale Manga Text Segmentation implementation remains" >&2
  exit 1
fi
bun cargo fmt --all -- --check
bun cargo check --workspace --all-targets
bun cargo test -p koharu-app orders_inpainters_without_translator
bun cargo test -p koharu-app registry_retains_all_production_engine_ids
git diff --check
```

Expected: `koharu-ml`/`koharu-llm` 无 bin；PaddleOCR-VL 1.5 与 Manga Segmentation 实现/模型声明消失；PaddleOCR-VL 1.6、15 个生产 Engine、Pipeline、OpenAPI、Codex bin 保留；workspace PASS。

**Step 5: Commit**

```bash
git add crates/koharu-ml/Cargo.toml crates/koharu-llm/Cargo.toml \
  crates/koharu-ml/src/lib.rs crates/koharu-ml/bin crates/koharu-llm/bin \
  crates/koharu-ml/src/paddleocr_vl \
  crates/koharu-ml/src/manga_text_segmentation_2025 \
  crates/koharu-ml/tests/ocr.rs \
  crates/koharu-ml/tests/manga_text_segmentation_2025.rs
git commit -m "refactor(ml): remove unused model diagnostics"
```

### Task 2：删除 Custom Pipeline UI，保留 prompt 和后端步骤能力

**Files:**

- Modify: `ui/components/MenuBar.tsx`
- Modify: `ui/lib/stores/preferencesStore.ts`
- Modify: `ui/tests/components/MenuBar.test.tsx`
- Modify: `ui/public/locales/en-US/translation.json`
- Modify: `ui/public/locales/es-ES/translation.json`
- Modify: `ui/public/locales/ja-JP/translation.json`
- Modify: `ui/public/locales/ko-KR/translation.json`
- Modify: `ui/public/locales/pt-BR/translation.json`
- Modify: `ui/public/locales/ru-RU/translation.json`
- Modify: `ui/public/locales/tr-TR/translation.json`
- Modify: `ui/public/locales/zh-CN/translation.json`
- Modify: `ui/public/locales/zh-TW/translation.json`

**Step 1: 写失败测试**

在现有 `MenuBar.test.tsx` 增加：

- `menu_hides_custom_pipeline_controls`
- `full_pipeline_still_uses_all_configured_engines`

第二个测试必须断言 Full Pipeline request 仍包含 detector、segmenter、bubble segmenter、font detector、OCR、translator、Inpainter 和 Renderer，并保留 `systemPrompt`、default font 与 reading order。

本 Task 明确接受删除“持久化任意阶段子集 + Current/All”这一高级 UI；HTTP/MCP/CLI 的 `steps` 仍提供同等后端能力。测试使用默认的不同 Engine ID，不在本 Task 为 CTD 相同 ID 或多 producer 增加去重逻辑。

Run:

```bash
bun run --filter ui test -- tests/components/MenuBar.test.tsx \
  -t "menu_hides_custom_pipeline_controls|full_pipeline_still_uses_all_configured_engines"
```

Expected: FAIL，当前仍显示 Custom Pipeline submenu 和 Run Custom actions。

**Step 2: 最小删除**

- 删除 MenuBar 的 `runCustomPipeline`、两个 Run Custom item、五个 toggle 和相关 import/state。
- 从 preferences 类型、initial state、setter 和 `partialize` 删除 `customPipeline`。
- 把 persistence version 从 7 升到 8；migration 只执行 `delete persisted.customPipeline`。
- 删除只服务 Custom Pipeline 的 locale key。
- 不修改 `CanvasToolbar.tsx`、`TextBlocksPanel.tsx`、`customSystemPrompt`、system prompt textarea 或任何 Codex preference。
- 不修改 Rust/HTTP/MCP/CLI 的 `steps`、`system_prompt` 或 Engine config。
- 不新增 Pipeline request helper，不顺带修改 Full Pipeline 的 Engine 选择或 DAG 行为。

**Step 3: 验证**

```bash
bun run --filter ui test -- tests/components/MenuBar.test.tsx
bun run lint:ui
bun run --filter ui build
rg -n "customPipeline|setCustomPipeline|runCustomPipeline|runCustomCurrent|runCustomAll" ui \
  --glob '!ui/openapi.json' --glob '!ui/lib/api/generated.ts' \
  --glob '!ui/lib/api/schemas/**'
rg -n "customSystemPrompt|systemPromptPlaceholder" \
  ui/components/canvas/CanvasToolbar.tsx \
  ui/components/panels/TextBlocksPanel.tsx \
  ui/lib/stores/preferencesStore.ts
```

Expected: MenuBar tests、lint 和 Next build PASS；第一个 `rg` 无 Custom Pipeline UI 命中，第二个 `rg` 仍命中保留的 prompt UI/调用方。

**Step 4: Commit**

```bash
git add ui/components/MenuBar.tsx ui/lib/stores/preferencesStore.ts \
  ui/tests/components/MenuBar.test.tsx ui/public/locales/*/translation.json
git commit -m "refactor(ui): remove custom pipeline controls"
```

### Task 3：按产品范围删除普通彩色 Brush UI，保留旧 BrushInpaint composite

**Files:**

- Delete: `ui/hooks/useRenderBrushDrawing.ts`
- Delete: `ui/hooks/useBrushLayerDisplay.ts`
- Modify: `ui/lib/types.ts`
- Modify: `ui/lib/stores/editorUiStore.ts`
- Modify: `ui/lib/stores/preferencesStore.ts`
- Modify: `ui/hooks/useBrushCursor.ts`
- Modify: `ui/hooks/useKeyboardShortcuts.ts`
- Modify: `ui/components/canvas/ToolRail.tsx`
- Modify: `ui/components/canvas/SubToolRail.tsx`
- Modify: `ui/components/canvas/Workspace.tsx`
- Modify: `ui/components/panels/LayersPanel.tsx`
- Modify: `ui/components/SettingsDialog.tsx`
- Create: `ui/tests/components/ToolRail.test.tsx`
- Modify: `ui/tests/components/SubToolRail.test.tsx`
- Modify: `ui/tests/hooks/useKeyboardShortcuts.test.tsx`
- Create: `ui/tests/hooks/useMaskDrawing.test.tsx`
- Modify: `ui/public/locales/en-US/translation.json`
- Modify: `ui/public/locales/es-ES/translation.json`
- Modify: `ui/public/locales/ja-JP/translation.json`
- Modify: `ui/public/locales/ko-KR/translation.json`
- Modify: `ui/public/locales/pt-BR/translation.json`
- Modify: `ui/public/locales/ru-RU/translation.json`
- Modify: `ui/public/locales/tr-TR/translation.json`
- Modify: `ui/public/locales/zh-CN/translation.json`
- Modify: `ui/public/locales/zh-TW/translation.json`
- Test: `crates/koharu-app/src/pipeline/engines/renderer.rs`

**Step 1: 写失败测试**

本 Task 是产品范围删减，不把彩色 Brush 与 Repair Brush 视为同一功能；前者写入 BrushInpaint 像素 overlay，后者编辑 Segment Mask 并触发 Inpaint。

- `ToolRail.test.tsx`: `does_not_expose_color_brush_but_keeps_repair_and_eraser`。
- `SubToolRail.test.tsx`: 删除普通 Brush/color picker 断言，保留 Eraser/Repair Brush 的 size control 断言。
- `useKeyboardShortcuts.test.tsx`: `removed_brush_shortcut_does_not_switch_tools`。
- `useMaskDrawing.test.tsx`: `eraser_mode_keeps_segment_mask_visible_and_updates_only_segment_endpoint`。从默认 `select` 状态调用生产 `setMode('eraser')`，断言 `showSegmentationMask == true`；mock 现有 `useCanvasDrawing()` 只捕获其 `onFinalizeFullCanvas`，调用生产 `useMaskDrawing()` 的 finalize 路径，断言请求为 `PUT /api/v1/pages/{id}/masks/segment?...`，且从不请求 `/masks/brushInpaint`。不创建新 production helper。
- 重新运行 Task 0 的 `legacy_brush_layer_still_composites_without_editor_ui`。

Run:

```bash
bun run --filter ui test -- tests/components/ToolRail.test.tsx \
  tests/components/SubToolRail.test.tsx \
  tests/hooks/useKeyboardShortcuts.test.tsx \
  tests/hooks/useMaskDrawing.test.tsx -t "brush|repair|eraser|segment"
bun cargo test -p koharu-app legacy_brush_layer_still_composites_without_editor_ui
```

Expected: UI tests FAIL；Rust characterization test PASS。

**Step 2: 最小删除**

- 从 `ToolMode`、ToolRail、Workspace 和 cursor/shortcut 分支删除普通 `brush`。
- 删除两个普通 Brush 专用 hooks，以及 Workspace 的 brush blob/display/drawing wiring。
- 在 `editorUiStore.setMode()` 中让 `eraser` 与 `repairBrush` 一样强制 `showSegmentationMask: true`；删除 `showBrushLayer` 后不得沿用 Eraser 的旧可见层状态。
- 将 Workspace 的 Segment 绘制入口收敛为 `maskPointerEnabled = mode === 'repairBrush' || mode === 'eraser'`，删除 `brushPointerEnabled` 和 brush bindings；Eraser 与 Repair Brush 都只复用现有 `useMaskDrawing()`，不得新增路由 helper 或绘图抽象。
- 从 LayersPanel 删除可编辑 Brush layer toggle；旧 BrushInpaint 数据仍由后端 Renderer 自动 composite。
- 从 preferences shortcuts 删除 `brush`，从 `brushConfig` 删除只服务彩色 Brush 的 `color`。
- persistence version 从 8 升到 9；migration 删除 `persisted.shortcuts.brush` 和 `persisted.brushConfig.color`。
- SubToolRail 只保留 Eraser/Repair Brush 共用 size control，删除 ColorPicker。
- 删除普通 Brush、brush color 和 show brush layer 的 locale keys。
- 保留 `MaskRole::BrushInpaint`、`PUT /pages/{id}/masks/brushInpaint`、Renderer overlay、PSD/KHR 和 `findMaskBlob(..., 'brushInpaint')` 兼容 helper/test。

**Step 3: 验证**

```bash
bun run --filter ui test -- tests/components/ToolRail.test.tsx \
  tests/components/SubToolRail.test.tsx \
  tests/hooks/useKeyboardShortcuts.test.tsx \
  tests/hooks/useMaskDrawing.test.tsx
bun run lint:ui
bun run --filter ui build
bun cargo test -p koharu-app legacy_brush_layer_still_composites_without_editor_ui
bun cargo test -p koharu-app final_inpaint_mask_preserves_repair_region_semantics
rg -n "mode === 'brush'|value: 'brush'|shortcuts\.brush|showBrushLayer|useBrushLayerDisplay|useRenderBrushDrawing|brushConfig\.color" ui
rg -n "BrushInpaint|brushInpaint" crates/koharu-core crates/koharu-app crates/koharu-rpc
```

Expected: UI tests、lint、Next build 和 Rust tests PASS；第一个 `rg` 无 UI 命中，第二个 `rg` 仍命中后端兼容和 Renderer 路径；Eraser 只写 Segment Mask。

**Step 4: Commit**

```bash
git add ui/hooks/useRenderBrushDrawing.ts ui/hooks/useBrushLayerDisplay.ts \
  ui/lib/types.ts ui/lib/stores/editorUiStore.ts ui/lib/stores/preferencesStore.ts \
  ui/hooks/useBrushCursor.ts ui/hooks/useKeyboardShortcuts.ts \
  ui/components/canvas/ToolRail.tsx ui/components/canvas/SubToolRail.tsx \
  ui/components/canvas/Workspace.tsx ui/components/panels/LayersPanel.tsx \
  ui/components/SettingsDialog.tsx ui/tests/components/ToolRail.test.tsx \
  ui/tests/components/SubToolRail.test.tsx \
  ui/tests/hooks/useKeyboardShortcuts.test.tsx \
  ui/tests/hooks/useMaskDrawing.test.tsx \
  ui/public/locales/*/translation.json
git commit -m "refactor(ui): remove color brush editing"
```

### Task 4：在无调用证据后删除 Custom image-layer 新建 API

**Prerequisite:** 发布记录、内部调用清单和仓库搜索均证明 `/api/v1/pages/{id}/image-layers` 没有外部消费者。无法证明时记录 NOT APPLICABLE，跳过本 Task，不阻止 Task 5。

**Files:**

- Modify: `crates/koharu-rpc/src/routes/pages.rs`
- Modify: `crates/koharu-rpc/tests/openapi.rs`
- Modify: `crates/koharu-rpc/tests/snapshots/openapi__openapi_paths_snapshot.snap`
- Modify: `ui/orval.config.ts`
- Generated: `ui/openapi.json`
- Generated: `ui/lib/api/generated.ts`
- Generated: `ui/lib/api/schemas/addImageLayerResponse.ts`
- Generated: `ui/lib/api/schemas/index.ts`
- Test: `crates/koharu-app/src/session.rs`

`ui/lib/api/index.ts` 当前没有导出 `addImageLayer`，不修改它。

**Step 1: 写失败测试**

在 `openapi.rs` 增加 `custom_image_layer_route_is_not_exposed`，从真实 OpenAPI spec 断言 `paths` 不包含 `/pages/{id}/image-layers`。

Run:

```bash
bun cargo test -p koharu-rpc custom_image_layer_route_is_not_exposed
```

Expected: FAIL，当前 route 仍注册。

**Step 2: 删除 route**

- 从 pages router 删除模块顶部的 image-layer 路由说明、`add_image_layer` registration、multipart handler、`AddImageLayerResponse` 和仅被该 handler 使用的 `center_on_page()`。
- 删除 `pages.rs` 中随上述代码失去引用的 imports；不得用 `#[allow(dead_code)]` 或 `#[allow(unused_imports)]` 掩盖残留。
- 从 `ui/orval.config.ts` 删除仅服务已移除 operation 的 `addImageLayer.formData` override。
- 不删除 `ImageRole::Custom`、项目 decode、KHR/PSD export 或 BlobStore。
- 不修改其他 `/pages`、`/pipelines` 或 MCP route。

**Step 3: 生成预期文件并验证实现**

```bash
bun run generate:api
INSTA_UPDATE=always bun cargo test -p koharu-rpc --test openapi
bun cargo test -p koharu-rpc --test openapi
bun cargo clippy -p koharu-rpc --all-targets -- -D warnings
bun cargo test -p koharu-app legacy_optional_layers_round_trip_before_scope_reduction
bun run format:check
bun run lint:ui
bun run test:ui
git diff --check
```

Expected: 只有 image-layer route、Orval operation override、path/schema/client 和 OpenAPI snapshot 变化；`cargo clippy` 不会留下 `center_on_page` 或 route-only import 警告。此时不要运行 `check:generated`，因为预期生成物尚未提交。

**Step 4: Commit**

```bash
git add crates/koharu-rpc/src/routes/pages.rs \
  crates/koharu-rpc/tests/openapi.rs \
  crates/koharu-rpc/tests/snapshots/openapi__openapi_paths_snapshot.snap \
  ui/orval.config.ts ui/openapi.json ui/lib/api/generated.ts ui/lib/api/schemas
git commit -m "refactor(api): remove unused custom image upload"
```

**Step 5: 提交后检查生成物漂移**

```bash
bun run check:generated
```

Expected: PASS，工作树不再包含生成物 diff。

### Task 5：文档同步、全量门禁和空间复测

**Files:**

- Modify: `docs/zh-CN/project-functional-analysis.md`
- Modify only if Task 4 executed: `docs/en-US/reference/http-api.md`
- Modify only if Task 4 executed: `docs/ja-JP/reference/http-api.md`
- Modify only if Task 4 executed: `docs/pt-BR/reference/http-api.md`
- Modify only if Task 4 executed: `docs/zh-CN/reference/http-api.md`

普通 Brush 的 PSD/KHR 兼容仍存在，因此 `export-and-manage-projects.md` 中“旧项目可包含 brush layer”的导出说明保留。

**Step 1: 更新中文功能分析**

- 删除普通彩色 Brush 的创建/显示操作说明，改为“旧项目 BrushInpaint 仍参与 Render/PSD/KHR”。
- 删除 Custom Pipeline preference/UI 说明；保留 system prompt、Engine/Provider、HTTP/MCP/CLI 自定义步骤说明。
- 若 Task 4 执行，从四种语言的 HTTP API reference 删除 `/pages/{id}/image-layers`；否则不改这些文档。
- 不修改 MCP、Google Fonts、Codex、Inpainter、Speech Bubble 或生产 Engine 文档。

**Step 2: 全量质量门禁**

```bash
bun cargo fmt --all -- --check
bun cargo check --workspace --all-targets
bun cargo clippy --workspace --all-targets -- -D warnings
bun cargo test --workspace --tests
bun run format:check
bun run lint:ui
bun run test:ui
bun run check:generated
bun run build
bun cargo check -p koharu --all-targets --features=metal
bun cargo test -p koharu-app registry_retains_all_production_engine_ids
git diff --check
```

Expected: PASS；默认测试中的模型集成测试仍为 ignored，不下载模型。

**Step 3: 验证保留范围没有被修改**

```bash
rg -n "koharu-ai|koharu_ai" Cargo.toml crates/koharu-app crates/koharu-ai
rg -n "AiPanel|useGetCodexAuthStatus" ui/components
rg -n 'id: "(anime-text|aot-inpainting|speech-bubble-segmentation|comic-text-bubble-detector|comic-text-detector|comic-text-detector-seg|flux2-klein|lama-manga|llm|manga-ocr|mit48px-ocr|paddle-ocr-vl-1.6|pp-doclayout-v3|koharu-renderer|yuzumarker-font-detection)"' \
  crates/koharu-app/src/pipeline/engines
rg -n "rmcp|/mcp|pub mod mcp" Cargo.toml crates/koharu-rpc
rg -n "google-fonts|GoogleFontService" crates/koharu-app crates/koharu-rpc ui
```

Expected: 所有命令仍有生产命中。若任一结果为空，停止并检查误删。

**Step 4: 复测 binary 和磁盘结果**

```bash
cargo metadata --no-deps --format-version 1 > /tmp/koharu-metadata-final.json
du -sh target ui/node_modules 2>/dev/null || true
git diff --stat
```

Expected:

- `koharu-ml`/`koharu-llm` 的 17 个诊断 bin 不再编译。
- `koharu-ai` Codex bin、产品 `pipeline` 和 `openapi` 保留。
- 重复 PaddleOCR、Manga Text Segmentation 2025 和冗余 UI 已删除。
- 不设置源码删减配额，不为追求行数继续删除生产功能。

**Step 5: 人工电商验收**

- AI tab 可登录并进入 Codex 整页生成。
- Full Pipeline request 仍包含全部配置阶段，包括 Bubble、Font Detector 和 Inpainter。
- CanvasToolbar 仍可单独运行 Translate、Inpaint 和 Render，并保留 custom prompt。
- Repair Brush、Segment Eraser 和旧 BrushInpaint composite 正常。
- 纯英文/品牌/SKU/数字不翻译、不擦除、不重绘。
- 旧项目仍可打开、保存和导出。

**Step 6: Commit docs**

```bash
git add docs/zh-CN/project-functional-analysis.md \
  docs/en-US/reference/http-api.md docs/ja-JP/reference/http-api.md \
  docs/pt-BR/reference/http-api.md docs/zh-CN/reference/http-api.md
git commit -m "docs: align guides with simplified ecommerce UI"
```

只暂存实际修改的文档；Task 4 未执行时不得暂存四个 HTTP reference。

## 4. 完成定义

- [ ] Codex 整页生成、AI UI/API/CLI 完整保留。
- [ ] Lama、AOT、Flux2、Speech Bubble、BubbleMask 和 Renderer Bubble 布局完整保留。
- [ ] 所有生产 Detector/OCR/Font Detector、Google Fonts、MCP 和 Provider 配置完整保留。
- [ ] Custom Pipeline UI 已删除，但 custom prompt 和后端 `steps` 能力保留。
- [ ] 普通彩色 Brush UI 已删除，Repair Brush/Segment Eraser 和旧 BrushInpaint composite 保留。
- [ ] `koharu-ml`/`koharu-llm` 的 17 个诊断 bin、重复 PaddleOCR 和 Manga Text Segmentation 2025 已删除。
- [ ] 仓库与已登记外部流程均无诊断 bin 消费者；PaddleOCR-VL 1.5 模型声明消失，1.6 生产声明保留。
- [ ] Custom image upload 只有在无调用证据后才删除；否则明确记录 NOT APPLICABLE。
- [ ] 旧项目、HanOnly、AllText、Jobs/SSE、HTTP/MCP/CLI 和导出回归通过。
- [ ] CTD 重复配置、异步任务生命周期和 Pipeline request helper 明确留给独立任务，本计划不声称已修复。
- [ ] 生成物检查顺序正确，全部门禁通过，未混入计划外工作树修改。
