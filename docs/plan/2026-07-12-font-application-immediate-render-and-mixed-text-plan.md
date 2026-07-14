# 字体应用、即时渲染与中英混合文本保护 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 选择字体后立即在当前桌面画布看到真实渲染结果；未选择文本框时把字体应用到当前页全部文本框，已选择时只修改选中框；纯英文行不进入后续流程，`S型曲线` 这类单字母标记随中文整行处理，而包含完整英文词的中英同排内容只擦除、翻译和重绘中文区域并保留英文原像素。

**Architecture:** 保留后端 Scene 为唯一事实来源。离散字体选择先提交 `Op`，再复用现有 renderer pipeline 立即重渲染；输入、滑杆等连续编辑继续使用 500ms debounce。字体批量操作只修改 `fontFamilies`，保留每个文本框的字号、颜色、描边、效果和对齐。电商流程固定遵守 HanOnly：纯英文节点在 Segment 前短路；已有逐行 polygon 继续按行过滤；同一视觉行同时含完整英文词和中文时，PaddleOCR-VL 提供权威正文，跨平台 PP-OCRv5 提供 word boxes，经严格校验后写回现有 `text + linePolygons` 下游链路。词框无法形成完整、无歧义且安全的英文/中文分区时，清除该候选的旧逐行几何并让整个节点停止，不按字符宽度猜坐标。

**Tech Stack:** React 19、TypeScript、Zustand、TanStack Query、Vitest、Testing Library、MSW、Bun、Rust/Tauri、PaddleOCR-VL 1.6、PP-OCRv5 server ONNX、`ocrs-cjk`、纯 Rust RTen。

---

## 已核实的当前行为与剩余缺口

- `d852b7c3` 已实现 `runAutoRenderNow()`、字体全局/选中作用域、Google Font 失败短路、HanOnly 混合几何警告和九种语言 key。
- 自动渲染测试实际位于 `ui/tests/lib/io/autoRender.test.ts`；本修订统一所有路径和命令，并要求核对实际执行数量，防止全部 skipped 但退出码为 0 的假绿。
- 字体实现已按 `applyOp → runAutoRenderNow` 执行，但仍缺少 Google Font 下载成功顺序以及 `stroke/effect` 保持的明确回归断言。
- UI 已覆盖缺少 polygon、匹配 polygon、纯中文和 AllText 兼容；现有纯英文测试锁定的是旧“可生成”行为，必须改为 HanOnly 下显示跳过状态并禁用 Generate，同时补充“polygon 数量不匹配”的独立测试。
- 后端已有纯英文/unsupported 的 Segment 短路、共享空 mask、Lama dispatch 和空 Han Renderer 测试；Translator 有效几何 fixture、AOT 与 Flux2 生产 dispatch 短路仍需补齐，本计划不重复已有 Lama 覆盖。
- 当前 `TextData.line_polygons` 只能描述整行，不能可靠区分同一视觉行内的英文词和中文。最新产品决定使用 PP-OCRv5 word boxes 补充几何，并由 PaddleOCR-VL 首轮正文做严格内容校验。
- 当前 `eligible_text_lines()` 会把任何含 Han 的单行整体视为目标。必须只对“含完整英文词且仍未被可靠词框拆分”的行返回 unsupported；`S型曲线` 仍是合法整行目标。

## 范围与不变量

### In Scope

- 字体 family/variant 的当前页全局作用域与选中作用域。
- 字体变更后立即启动当前配置的 Renderer。
- 自动渲染失败的可见错误提示。
- HanOnly 下缺少逐行几何的中英混合节点状态提示。
- 纯英文内容在 Segment、Translate、Inpaint、Renderer 前短路的现有后端回归。
- PaddleOCR-VL 首轮 OCR 识别“完整英文词 + 中文”的同排内容后，仅对候选 crop 按需执行 PP-OCRv5 word-box 推理，并把严格校验后的可靠结果写回现有 `text + linePolygons`。
- 中文逻辑单元进入 Segment、Translate、Inpaint、Renderer；纯英文逻辑单元保留 Source 原像素且不进入这些阶段。
- UI 单元/组件测试及现有后端几何安全回归。

### Out of Scope

- 不修改 OCR 模型权重、现有 Detector、Inpaint 模型或 Renderer 排版算法；只增加 PP-OCRv5 几何适配和现有 HanOnly 下游入口。
- 不生成虚假的 `linePolygons`，不把节点矩形等分为行框。
- 不强制统一字号、颜色、描边、效果或对齐；这些仍由现有独立控件控制。
- 不修改 HTTP 请求结构、OpenAPI schema、Rust Scene 类型或 `source_text_policy`；词框结果继续写入已有 `TextData.text` 和 `line_polygons`。
- 仅新增 PP-OCRv5 所需的 `ocrs-cjk`/`rten` 依赖和一个 `koharu-ml/src/pp_ocr_v5.rs` 适配模块；不新增 Zustand store、Renderer trait、持久化 Scene/API DTO、Provider fake 或通用 OCR 架构层。
- 不删除 AllText 后端兼容模式；但电商 HanOnly UI 不把“切换 AllText”作为处理建议。

### 核心不变量

- 字体 Op 成功提交后才允许启动立即渲染。
- 没有选中节点时：更新 `defaultFont`，并给当前页全部文本节点写入同一 `fontFamilies`。
- 有选中节点时：只更新选中节点，不修改全局 `defaultFont`。
- 字体修改只改变 `fontFamilies`；每个节点原有 `fontSize`、`color`、`stroke`、`effect`、`textAlign` 必须保持。
- 字体 family 和 variant 的行为一致；未缓存 Google Font 必须先下载成功，再提交 Op 和渲染。
- 立即渲染会取消尚未执行的 debounce timer，避免同一操作重复启动 Renderer。
- 文本输入、字号、颜色等现有连续编辑仍走 debounce，不扩大本次立即渲染范围。
- HanOnly 混合节点没有可靠逐行几何时不得猜测、不得翻译/擦除英文；UI 只显示安全状态和解决建议。
- 纯英文节点不得调用 Segment inference、Provider、Inpaint backend 或 Renderer，最终可见像素保持 Source/Inpainted 基面。
- “完整英文词”定义为同一连续 Latin 单元内至少两个字母，允许内部连字符或撇号；`AI智能` 中的 `AI`、`Peach蜜桃臀` 中的 `Peach` 必须保留，`S型曲线` 中的单字母 `S` 不属于受保护完整英文词，该整行必须擦除、翻译和重绘。
- 有可靠逐行/word-box 几何的混合节点只处理含 Han 的逻辑单元；纯英文逻辑单元不得进入 mask、Provider 输入、Inpaint 或 Renderer。
- 含完整英文词和中文的同一视觉行，Provider 只能收到中文逻辑单元；最终 Mask 在英文 polygon 内必须全零，最终 Rendered 在英文 polygon 内必须逐像素等于 Source/Inpainted 基面。
- PP-OCRv5 只在 HanOnly 候选节点按需加载；英文必须与 PaddleOCR-VL 首轮正文精确一致，Han 必须保持相同字符数量和 Han 脚本形状；坐标必须有限、非退化、位于原 crop 内且英文/中文目标不重叠，并至少形成一个受保护英文单元及恰好一个含 Han 目标。任何条件失败都清除该候选旧 `line_polygons`，让共享 eligibility 将其判为 unsupported，后续模型调用次数为零。
- 缺少、错配或不安全逐行几何的混合节点整体停止后续处理；允许清理旧 translation/sprite 等陈旧派生字段，但不得产生新的英文处理结果。
- AllText 仅保留现有后端兼容行为，不作为电商流程入口、提示或绕过方案。

## Task 1：为现有自动渲染增加可等待的立即执行入口

**Files:**

- Modify: `ui/lib/io/scene.ts:92-144`
- Test: `ui/tests/lib/io/autoRender.test.ts`

### Step 1：补齐并校正现有回归测试

在现有 `ui/tests/lib/io/autoRender.test.ts` 中通过 MSW 记录 `/api/v1/pipelines` 请求，并使用 fake timers 验证：

1. `runAutoRenderNow('p1')` 立即读取 `/config` 并启动配置中的 renderer。
2. 请求包含 `steps: ['koharu-renderer']`、`pages: ['p1']` 和当前 `defaultFont`。
3. 先调用 `queueAutoRender('p1')` 再调用 `runAutoRenderNow('p1')`，推进 500ms 后仍只有一次 pipeline 请求。
4. `runAutoRenderNow()` 遇到 renderer 为空时不发 pipeline 请求；不能只测试 debounce 入口。
5. `/config` 和 `/pipelines` 两种失败分别调用 `useEditorUiStore.getState().showError(...)`，且 Promise 正常结束，不产生未处理 rejection。

测试名称固定为：

```text
runAutoRenderNow starts the configured renderer immediately
runAutoRenderNow cancels the pending debounced render
runAutoRenderNow skips when no renderer is configured
runAutoRenderNow surfaces config failures through the editor error state
runAutoRenderNow surfaces pipeline failures through the editor error state
```

### Step 2：运行完整测试文件并拒绝 skipped 假绿

```bash
bun run --filter ui test -- tests/lib/io/autoRender.test.ts
```

预期：`Test Files 1 passed`，上述五个 `runAutoRenderNow` 测试全部实际执行；如果输出 `skipped` 或测试数量没有增加，则视为 FAIL，即使退出码为 0。

### Step 3：仅在测试暴露回归时修正共享执行路径

当前生产结构已存在。只有测试失败时才在 `ui/lib/io/scene.ts` 做以下最小修正：

- 保留 `queueAutoRender(pageId)` 和 500ms debounce。
- 把“读取配置并调用 renderer”保留为唯一私有执行函数。
- 增加一个私有 timer 清理函数，仅管理现有两个模块变量。
- 导出 `runAutoRenderNow(pageId): Promise<void>`：先清理 pending timer/page，再执行同一渲染函数。
- debounce callback 与立即入口共用同一个错误报告函数；使用现有 `useEditorUiStore.getState().showError(String(err))`，同时保留 `console.error` 供开发诊断。

目标结构：

```ts
function cancelQueuedAutoRender(): void {
  if (autoRenderTimer) clearTimeout(autoRenderTimer)
  autoRenderTimer = null
  autoRenderPendingPageId = null
}

async function runAutoRender(pageId: string): Promise<void> {
  const cfg = await getConfig()
  const renderer = cfg.pipeline?.renderer
  if (!renderer) return
  await startPipeline({
    steps: [renderer],
    pages: [pageId],
    defaultFont: usePreferencesStore.getState().defaultFont,
  })
}

async function runAutoRenderWithFeedback(pageId: string): Promise<void> {
  try {
    await runAutoRender(pageId)
  } catch (error) {
    console.error('Auto-render failed:', error)
    useEditorUiStore.getState().showError(String(error))
  }
}

export async function runAutoRenderNow(pageId: string): Promise<void> {
  cancelQueuedAutoRender()
  await runAutoRenderWithFeedback(pageId)
}
```

不要新增并发队列、事件总线或新的 store。Jobs 状态继续由现有 pipeline/SSE 路径维护。

### Step 4：运行测试并确认通过

```bash
bun run --filter ui test -- tests/lib/io/autoRender.test.ts
```

预期：新增测试和原有 auto-render 测试全部 PASS。

### Step 5：提交边界

仅在用户另行授权提交时执行：

```bash
git add ui/lib/io/scene.ts ui/tests/lib/io/autoRender.test.ts
git commit -m "fix(ui): add immediate auto render execution"
```

## Task 2：修正字体作用域并在字体选择后立即渲染

**Files:**

- Modify: `ui/components/panels/RenderControlsPanel.tsx:286-352,453-481`
- Test: `ui/tests/components/RenderControlsPanel.test.tsx`

### Step 1：补齐当前实现尚未覆盖的回归断言

扩展 `vi.mock('@/lib/io/scene')`，同时 mock `runAutoRenderNow`。新增或替换测试：

```text
changing global font updates every text node and the default font
changing a selected font updates only selected nodes and leaves the default font unchanged
changing a font preserves each node non-font style fields
changing a font awaits applyOp before immediate render
changing a font variant uses the same scope and immediate render behavior
downloading a Google font completes before applying and rendering it
failing to download a Google font stops before applying and rendering
```

关键断言：

- 无选择时 `applyOp` 收到包含当前页全部 Text 节点的 batch。
- `defaultFont` 更新为选择的 PostScript name。
- 有选择时 batch 只包含 `selectedNodes`，`defaultFont` 保持原值。
- `runAutoRenderNow('p1')` 被调用一次，`queueAutoRender` 没有因字体操作被调用。
- 用受控 Promise 暂停 `applyOp`，在 resolve 前断言 `runAutoRenderNow` 未调用；resolve 后才调用。
- 输入节点使用不同的 `fontSize/color/stroke/effect/textAlign`，生成的每个 patch 只替换 `fontFamilies`，其他字段保持各自原值。
- 用受控 Google Font fetch Promise 暂停下载，在 resolve 前断言 `applyOp`、`runAutoRenderNow` 均未调用；下载和 `invalidateScene()` 完成后才允许继续。
- family/variant 操作后显式断言 `queueAutoRender` 未调用，避免立即渲染和 debounce 重复启动。

### Step 2：运行完整组件回归

```bash
bun run --filter ui test -- tests/components/RenderControlsPanel.test.tsx
```

预期：完整测试文件执行且无 skipped。当前生产实现应满足目标；新测试若失败，只修正对应根因，不重写字体控件或新增抽象。

### Step 3：核对现有样式写入顺序，仅在测试失败时最小修正

当前 `applyStyleToNodes` 已返回 `Promise<void>` 并支持默认 `false` 的 `immediate` 参数。核对结构保持如下；只有测试失败时才修改，不新增 DTO 或 helper 文件：

```ts
const applyStyleToNodes = async (
  nodes: TextNodeEntry[],
  updates: Partial<TextStyle>,
  label: string,
  immediate = false,
): Promise<void> => {
  if (!page || nodes.length === 0) return
  const op =
    nodes.length === 1
      ? buildStyleOp(nodes[0], updates)
      : ops.batch(label, nodes.map((node) => buildStyleOp(node, updates)))
  await applyOp(op)
  if (immediate) await runAutoRenderNow(page.id)
  else queueAutoRender(page.id)
}
```

- 原有同步调用点使用 `void applyStyleToNodes(...)`，保持 debounce 行为。
- 字体 family/variant handler 使用 `await applyStyleToNodes(..., true)`。
- 不复制 `buildStyleOp`，不新增专用 font-op 类型。

### Step 4：核对 family 与 variant 共用同一目标节点规则

两个 handler 已按同一规则执行；测试必须锁定以下行为：

```ts
const targets = selectedNodes.length > 0 ? selectedNodes : textNodes
if (selectedNodes.length === 0) {
  usePreferencesStore.getState().setDefaultFont(postScriptName)
}
await applyStyleToNodes(targets, { fontFamilies: [postScriptName] }, 'Font update', true)
```

未缓存 Google Font：

- 下载成功后 `await invalidateScene()`，再提交 style Op。
- 下载失败时调用现有 `showError` 并 `return`，不得写入不可用字体或启动 renderer。

不要把“统一字体”扩大为覆盖字号、颜色或描边。截图中 `Tiny Waist`、`S型曲线`、`Peach Booty` 的颜色/描边差异来自各节点现有 style/font prediction；这些属性继续由对应控件显式统一。

### Step 5：运行测试并确认通过

```bash
bun run --filter ui test -- tests/components/RenderControlsPanel.test.tsx
bun run --filter ui test -- tests/lib/io/autoRender.test.ts tests/components/RenderControlsPanel.test.tsx
```

预期：全部 PASS；字体变化走立即渲染，其他样式编辑仍走 debounce。

### Step 6：提交边界

仅在用户另行授权提交时执行：

```bash
git add ui/components/panels/RenderControlsPanel.tsx ui/tests/components/RenderControlsPanel.test.tsx
git commit -m "fix(ui): apply font scope and render immediately"
```

## Task 3：使用 PP-OCRv5 词框保护完整英文词，只处理同排中文

**Files:**

- Modify: Cargo.toml
- Modify: Cargo.lock
- Modify: crates/koharu-ml/Cargo.toml
- Modify: crates/koharu-ml/src/lib.rs
- Create: crates/koharu-ml/src/pp_ocr_v5.rs
- Modify/Test: crates/koharu-app/src/pipeline/engines/paddle_ocr.rs
- Modify/Test: crates/koharu-app/src/pipeline/engines/support.rs
- Test: crates/koharu-app/src/pipeline/engines/ctd_segment.rs
- Test: crates/koharu-app/src/pipeline/engines/llm_translate.rs
- Verify: crates/koharu-app/src/pipeline/engines/lama.rs
- Test: crates/koharu-app/src/pipeline/engines/aot.rs
- Test: crates/koharu-app/src/pipeline/engines/flux2_klein.rs
- Modify/Test: crates/koharu-app/src/pipeline/engines/renderer.rs
- Modify: crates/koharu-app/src/renderer.rs（仅暴露现有 placement_origin）
- Verify: crates/koharu-llm/src/paddleocr_vl.rs（普通 OCR 正文仍是权威来源，不新增位置解析器）
- Verify: ui/lib/api/index.ts
- Modify: ui/components/panels/TextBlocksPanel.tsx
- Test: ui/tests/components/TextBlocksPanel.test.tsx
- Modify: ui/public/locales 下现有九种 translation.json

### Step 1：先锁定完整英文词和单字母标记分类

在 support.rs 先写并运行：

~~~text
protected_latin_word_distinguishes_words_from_single_letter_labels
eligible_single_latin_label_with_han_targets_the_whole_line
eligible_inline_english_word_without_word_boxes_is_unsupported
eligible_word_box_inline_mixed_targets_only_han_units
~~~

真值表：

| PaddleOCR-VL 正文 | PP-OCRv5 安全词框 | HanOnly 结果 |
| --- | --- | --- |
| English only | 不调用 | 空目标，后续短路 |
| S型曲线 | 不调用 | 整行是目标 |
| AI智能塑形 | 无/失败 | unsupported |
| Peach蜜桃臀 | 无/失败 | unsupported |
| Peach 换行 蜜桃臀 | 英文/中文独立 polygon | 只返回蜜桃臀 |
| S-CURVE 换行 S型曲线 | 英文/中文独立 polygon | 只返回 S型曲线 |

contains_protected_latin_word() 继续作为唯一共享分类入口：连续 Latin 单元至少两个字母；内部连字符、ASCII 撇号和弯撇号不终止已开始的单元。禁止在各后端复制判断。

先在缺少实现的基线运行，预期新增测试 FAIL；当前工作树若已有同名实现，则必须确认测试实际执行而不是零匹配。

### Step 2：增加最小跨平台 PP-OCRv5 适配

在 koharu-ml/src/pp_ocr_v5.rs 实现：

- 使用 ocrs-cjk + 纯 Rust RTen 加载 PP-OCRv5 server detector、recognizer 和字典。
- 三个 runtime package 均为 bootstrap=false；普通启动、普通 OCR、AllText 和纯英文不得下载模型。
- 只有 PaddleOCR-VL 首轮正文包含同一行完整英文词 + Han 时，paddle_ocr.rs 才懒加载模型。
- 返回最小 PpOcrWordBox：line_index、text、crop-local axis-aligned bbox、confidence。
- 使用现有 OcrEngine 的 TextLine::segments() 取得脚本/词段边界，不新增 OCR trait、provider 或全局 hook。
- 字典解析必须保持模型 label 索引；多 scalar grapheme 用单个占位字符维持索引，不把它当作电商中文/Latin 目标。

模型加载在 spawn_blocking 内完成；默认测试只验证字典和纯函数，不调用 RuntimeManager 下载。

先写并运行：

~~~bash
bun cargo test -p koharu-ml ppocr_dictionary
~~~

预期：实现前编译 FAIL；实现后 1 个以上匹配测试 PASS，且无模型下载。

### Step 3：用 PaddleOCR-VL 正文严格校验 PP-OCRv5 词框

paddle_ocr.rs 保持首轮 PaddleOcrVlTask::Ocr 批处理。增加唯一生产入口 dispatch_inline_word_boxes()，它只负责候选选择、PP 推理、校验和按原 index 回组。

校验契约：

1. crop bounds 必须非空并位于原图内。
2. bbox/confidence 必须有限；confidence 至少 0.5；bbox 非退化并完全位于 crop。
3. 去除 Unicode 空白后，PP 所有 segment 必须完整覆盖 PaddleOCR-VL 正文，不能缺字、增字或改变顺序。
4. 英文字符必须与 PaddleOCR-VL 精确一致；Han 只允许同字符数量、Han 脚本形状一致的识别差异，最终写入的正文始终取 PaddleOCR-VL。
5. 结果必须包含至少一个同排受保护英文单元及恰好一个含 Han 目标；多个 Han 目标、英文与 Han 仍在同一 item、英文/中文 bbox 重叠、越界或低置信度全部失败。
6. S型曲线中的单字母 S 与 Han 单元合并为整个中文目标；不保护 S。
7. AllText、纯英文、纯中文、已有安全换行几何和普通 OCR 节点不调用 PP-OCRv5。

生产结果必须区分三个状态，不得用一个 Option 混淆：

- 未尝试：只写首轮 OCR text，line_polygons 外层为 None，保留已有 detector 几何。
- 校验成功：写 PaddleOCR-VL 权威逻辑正文，并写 Some(Some(polygons))。
- 候选已尝试但失败：写首轮 OCR text，并写 Some(None) 清除旧 line_polygons，防止任何旧逐行几何继续处理新正文。

使用最小 tuple 或现有私有函数表达“attempted + update”；不新增单实现 trait 或通用 DTO。构造 TextDataPatch 的最小纯函数必须由 Model::run() 实际调用，并用测试断言候选失败会产生 line_polygons == Some(None)。

测试名至少包括：

~~~text
requests_word_boxes_only_for_inline_han_with_a_complete_latin_word
all_text_never_requests_word_boxes_or_rewrites_ocr_text
production_word_box_dispatch_calls_inference_only_for_candidates
production_word_box_dispatch_marks_failed_candidates_for_geometry_clear
production_word_box_dispatch_keeps_original_index_order
pp_ocr_word_boxes_use_vl_text_and_preserve_the_english_span
maps_valid_word_box_units_back_to_absolute_page_polygons
merges_a_single_latin_label_into_the_han_target
rejects_word_boxes_when_the_english_word_disagrees_with_vl
rejects_word_boxes_that_does_not_cover_the_first_ocr_text
rejects_word_boxes_that_leaves_english_and_han_in_one_unit
rejects_word_boxes_with_multiple_han_units_for_one_node_sprite
rejects_word_boxes_outside_the_original_crop
rejects_overlapping_or_low_confidence_word_boxes
failed_word_box_candidate_clears_stale_line_polygons
~~~

运行：

~~~bash
bun cargo test -p koharu-app pipeline::engines::paddle_ocr::tests
~~~

预期：全部匹配测试 PASS，无模型下载；失败候选的 Scene patch 明确清除旧 polygon。

### Step 4：让共享 HanOnly 边界短路所有后续阶段

eligible_text_lines() 遇到任何仍同时含 Han 和完整 Latin 词的逻辑行时返回 None。词框成功后的 Peach 换行 蜜桃臀继续复用现有 polygon 数量校验，只返回 Han 行。

所有后续阶段继续消费 eligible_lines_for_page()：

- Segment：纯英文和 unsupported 在 inference closure 前短路。
- Translate：Provider targets 只包含 Han 的 node_id + line_index。
- Lama/AOT/Flux2：最终 mask 在 expansion 前后均限制到 Han support；空最终 mask 不调用 backend。
- Renderer：每个 stored sprite 按自身 NodeId 的 Han support 独立裁剪，禁止跨节点借用允许区。
- AllText：保持节点级 OCR/Provider/transform/通用布局，不经过 PP 词框或 HanOnly 裁剪。

运行：

~~~bash
bun cargo test -p koharu-app protected_latin_word_
bun cargo test -p koharu-app eligible_single_latin_label_with_han_targets_the_whole_line
bun cargo test -p koharu-app eligible_inline_english_word_without_word_boxes_is_unsupported
bun cargo test -p koharu-app eligible_word_box_inline_mixed_targets_only_han_units
bun cargo test -p koharu-app segment_dispatch_word_box_inline_mixed_keeps_english_roi_zero
bun cargo test -p koharu-app segment_dispatch_skips_english_and_unsupported_before_inference
bun cargo test -p koharu-app han_only_translation_targets_skip_english_and_unsupported
bun cargo test -p koharu-app final_inpaint_mask_keeps_word_box_english_word_zero
bun cargo test -p koharu-app lama_inpaint_dispatch_receives_final_mask
bun cargo test -p koharu-app aot_inpaint_dispatch_skips_empty_han_targets
bun cargo test -p koharu-app flux2_inpaint_dispatch_skips_empty_han_targets
~~~

预期：每条命令至少执行一个测试并 PASS；默认测试不下载模型。

### Step 5：在最终整页合成中恢复英文 Source 像素

仅 HanOnly 使用 protected_source_lines_for_page() 从已验证的逻辑正文和安全 polygon 收集完整英文词框。该 helper 必须复用 safe_mixed_line_bbox() 和 line_support_mask()，不信任原始越界 polygon。

dispatch_render_page() 是唯一生产合成入口：

1. 每个 sprite 仅按自己的 Han line support 清除允许区外 alpha。
2. 以 Inpainted 优先、Source fallback 构造 base。
3. 在受保护英文词框内从 Source 逐像素恢复。
4. 显式 Repair Brush 在 Source 恢复之后叠加，保留用户主动 region/mask 语义。
5. 再叠加已经裁剪的 Han sprites。
6. AllText 完全跳过 Source 恢复和 Han 裁剪。

生产测试必须调用 dispatch_render_page()，不得只测试 mask/helper：

~~~text
protected_source_lines_keep_only_validated_english_word_boxes
han_only_renderer_clips_word_box_inline_mixed_to_han_mask
han_only_renderer_restores_validated_english_pixels_from_source
han_only_renderer_does_not_allow_one_node_sprite_into_another_node_mask
han_only_renderer_uses_the_same_fractional_origin_for_clip_and_composite
han_only_renderer_empty_text_targets_skip_backend_but_textless_renders
~~~

运行：

~~~bash
bun cargo test -p koharu-app protected_source_lines_keep_only_validated_english_word_boxes
bun cargo test -p koharu-app han_only_renderer_
~~~

预期：英文 ROI 等于 Source；中文 ROI 保留新 sprite；Repair Brush 仍可显式覆盖；stored sprite 在 Han support 外透明。

### Step 6：保持 UI 操作闭环和安全提示

TextBlocksPanel.tsx 只镜像用户可见阻断条件，不复制后端几何验证：

- source_text_policy 缺失时按 han_only。
- 纯英文或 unresolved 完整英文词 + Han：显示安全提示并禁用 Generate。
- S型曲线：允许 Generate。
- PP-OCRv5 成功写入匹配 text + linePolygons 后：允许 Generate。
- 手工修改 OCR 正文时同一 Op 清除 linePolygons，禁止复用旧词框。
- 提示不得回显 OCR 正文、不得建议切换 AllText；九种 locale 使用现有 mixedGeometryWarning key，并说明重新运行 PaddleOCR-VL 让 PP-OCRv5 生成词框，或手工拆框。

运行：

~~~bash
bun run --filter ui test -- tests/components/TextBlocksPanel.test.tsx
~~~

预期：完整测试文件 PASS，无 skipped。

### Step 7：Task 3 汇总回归

~~~bash
bun cargo test -p koharu-ml ppocr_dictionary
bun cargo test -p koharu-app pipeline::engines::paddle_ocr::tests
bun cargo test -p koharu-app eligible_mixed_node_
bun cargo test -p koharu-app eligible_pure_english_without_polygons_is_empty
bun cargo test -p koharu-app segment_dispatch_skips_english_and_unsupported_before_inference
bun cargo test -p koharu-app han_only_translation_targets_skip_english_and_unsupported
bun cargo test -p koharu-app final_inpaint_mask_short_circuits_empty_results
bun cargo test -p koharu-app han_translation_ops_empty_targets_still_cleanup
bun cargo test -p koharu-app han_only_renderer_
bun run --filter ui test -- tests/components/TextBlocksPanel.test.tsx
~~~

预期：全部 PASS、每个过滤器实际命中、无模型下载。HTTP、OpenAPI、Scene schema 和 StartPipelineRequest 不变。

### Step 8：提交边界

仅在用户另行授权提交时执行：

~~~bash
git add Cargo.toml Cargo.lock \
  crates/koharu-ml/Cargo.toml crates/koharu-ml/src/lib.rs crates/koharu-ml/src/pp_ocr_v5.rs \
  crates/koharu-app/src/pipeline/engines/paddle_ocr.rs \
  crates/koharu-app/src/pipeline/engines/support.rs \
  crates/koharu-app/src/pipeline/engines/ctd_segment.rs \
  crates/koharu-app/src/pipeline/engines/llm_translate.rs \
  crates/koharu-app/src/pipeline/engines/aot.rs \
  crates/koharu-app/src/pipeline/engines/flux2_klein.rs \
  crates/koharu-app/src/pipeline/engines/renderer.rs crates/koharu-app/src/renderer.rs \
  ui/components/panels/TextBlocksPanel.tsx ui/tests/components/TextBlocksPanel.test.tsx \
  ui/public/locales
git commit -m "fix(pipeline): preserve English words with PP-OCRv5 boxes"
~~~

不得暂存计划文档或其他既有无关修改。
## Task 4：全量回归与桌面人工验收

**Files:**

- No production file changes.
- Manual fixture: `/Users/jinkui/Desktop/截屏2026-07-12 13.40.11.png` 对应的原项目页面。

### Step 1：运行 UI 定向回归

```bash
bun run --filter ui test -- tests/lib/io/autoRender.test.ts tests/components/RenderControlsPanel.test.tsx tests/components/TextBlocksPanel.test.tsx
```

预期：全部 PASS，且测试不加载或下载 ML 模型。

### Step 2：运行项目质量门禁

```bash
bun run format:check
bun run lint:ui
bun run test:ui
bun cargo fmt --all -- --check
bun cargo check --workspace --all-targets
bun cargo clippy --workspace --all-targets -- -D warnings
bun cargo test -p koharu-ml ppocr_dictionary
bun cargo test -p koharu-app pipeline::engines::paddle_ocr::tests
bun cargo test -p koharu-app protected_latin_word_
bun cargo test -p koharu-app eligible_single_latin_label_with_han_targets_the_whole_line
bun cargo test -p koharu-app eligible_inline_english_word_without_word_boxes_is_unsupported
bun cargo test -p koharu-app eligible_word_box_inline_mixed_targets_only_han_units
bun cargo test -p koharu-app final_inpaint_mask_keeps_word_box_english_word_zero
bun cargo test -p koharu-app eligible_mixed_node_
bun cargo test -p koharu-app segment_dispatch_skips_english_and_unsupported_before_inference
bun cargo test -p koharu-app segment_dispatch_word_box_inline_mixed_keeps_english_roi_zero
bun cargo test -p koharu-app han_only_translation_targets_skip_english_and_unsupported
bun cargo test -p koharu-app protected_source_lines_keep_only_validated_english_word_boxes
bun cargo test -p koharu-app han_only_renderer_
bun cargo test -p koharu-app final_inpaint_mask_short_circuits_empty_results
bun cargo test -p koharu-app lama_inpaint_dispatch_receives_final_mask
bun cargo test -p koharu-app aot_inpaint_dispatch_skips_empty_han_targets
bun cargo test -p koharu-app flux2_inpaint_dispatch_skips_empty_han_targets
bun cargo test -p koharu-app han_only_renderer_empty_text_targets_skip_backend_but_textless_renders
NO_PROXY=127.0.0.1,localhost no_proxy=127.0.0.1,localhost bun cargo test --workspace --tests
bun run check:generated
bun run build
bun cargo check -p koharu --all-targets --features=metal
bun cargo build --release -p koharu --features=metal
git diff --check
git status --short
```

预期：

- 所有命令退出码为 0。
- `check:generated` 不留下非预期 diff；本计划不改变 OpenAPI。
- 保留实施前已存在的无关工作树修改，不暂存、不回退。

### Step 3：桌面 UI 人工验收

运行：

```bash
bun run dev
```

在问题页面逐项验证：

1. 清空文本框选择，选择一个新字体：当前页三个文本框都立即显示该字体，scope 显示“全局”。
2. 只选择中间文本框，再选择另一字体：只有中间框改变，其他框和默认字体不变。
3. 切换同一 family 的 variant：作用域与 family 完全一致，画布无需额外点击即可更新。
4. 连续拖动字号或输入翻译：仍经过 debounce，不产生每次击键一个 pipeline job。
5. 修改字体前后，三个节点各自的字号、颜色、描边、效果和对齐不被字体操作改变。
6. 纯英文文本框在 HanOnly 下显示安全跳过提示且 Generate 禁用；点击不会创建新的局部 Generate 任务。
7. `S型曲线` 作为单字母标记 + 中文整行擦除、翻译并重绘，不保留原始 `S`。
8. `Peach蜜桃臀` 或 `AI智能塑形` 经 PaddleOCR-VL 正文与 PP-OCRv5 词框严格校验后，只擦除和重绘中文 polygon；完整英文词保持 Source 原像素。
9. 人为构造 PP 英文与 VL 正文不一致、低置信度、位置越界、polygon 重叠或仍未拆开的同排内容：清除旧逐行几何，显示安全提示，Generate 禁用，整节点停止后续处理。
10. `S-CURVE\nS型曲线` 具有两组真实 polygon 时，英文逻辑单元不进入后续阶段，中文逻辑单元整行处理。
11. 将混合节点手工拆成独立英文框和中文框后，英文框保持原像素且不进入后续阶段，中文框可单独生成。
12. UI 不把 AllText 作为电商处理建议；显式旧 AllText 配置仍保持既有兼容行为。
13. 自动渲染失败时出现现有错误提示，不只写开发者控制台。

### Step 4：停止条件

只有同时满足以下条件才算计划实施完成：

- 字体 family/variant 的全局与选中作用域测试通过。
- 字体 Op 与立即 Renderer 的顺序测试通过，且没有重复 debounce 请求。
- 非字体样式字段保持测试通过。
- 纯英文、unsupported 混合节点的 UI 禁用和全阶段短路测试通过。
- `S型曲线` 作为整行目标；含完整英文词的同排内容只有在 PaddleOCR-VL + PP-OCRv5 严格校验成功后才处理中文逻辑单元。
- 有安全 word-box/逐行几何的混合节点只处理中文逻辑单元，英文 polygon 不进入最终 mask、Provider、Inpaint 或 Renderer，最终英文 ROI 与 Source 逐像素相等；显式 Repair Brush 仍保留用户 region 语义。
- AllText 兼容没有被删除，但未作为电商 UI 的建议或绕过入口。
- 后端 mixed geometry 安全测试通过。
- 当前问题页面完成上述人工验收。
- 无非预期生成物或无关文件修改。

## 剩余预期修改规模

- 生产实现：集中在 PP-OCRv5 RTen 适配、Paddle OCR 严格回组、共享 HanOnly 判定、Renderer Source 恢复和 UI 阻断；字体和自动渲染生产代码预期不再修改。
- 测试：覆盖词框字典/坐标回组、失败清理旧 polygon、`S型曲线`、完整英文词保护、假绿、Google Font 成功顺序、样式保持、Translator 目标以及三种 Inpainter 生产 dispatch 短路。
- 多语言文本：更新现有 9 个 `mixedGeometryWarning`，不得新增第二套 key。
- 新依赖：`ocrs-cjk` 与 `rten`，仅用于 PP-OCRv5 纯 Rust ONNX 推理。
- 新 production 文件：`crates/koharu-ml/src/pp_ocr_v5.rs`。
- OpenAPI/Scene schema 修改：0；PaddleOCR-VL 普通 OCR 仍是正文权威来源。

## 已知限制

- 统一字体只统一 font family/variant，不会自动统一颜色、描边和字号；这是避免破坏节点已有设计的明确选择。
- PP-OCRv5 模型在第一次命中 HanOnly 同排候选时按需下载；离线或下载失败时节点安全跳过，不会退回字符宽度估算。
- PP 英文与 PaddleOCR-VL 正文不一致、词框不安全或仍把完整英文词和 Han 放在同一 item 时，节点必须清除旧逐行几何并跳过。用户可以重跑 PaddleOCR-VL，或手工拆分文字框。
- 单 Latin 字母标记与 Han 相连时按用户规则整行处理，例如 `S型曲线`；如未来需要保护单字母品牌/尺码代码，应新增显式配置，不在本计划中继续增加启发式例外。
