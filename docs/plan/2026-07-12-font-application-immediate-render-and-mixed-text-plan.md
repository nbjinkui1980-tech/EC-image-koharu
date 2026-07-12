# 字体应用、即时渲染与混合文本提示 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 选择字体后立即在当前桌面画布看到真实渲染结果；未选择文本框时把字体应用到当前页全部文本框，已选择时只修改选中框；对缺少可靠逐行几何的中英混合文本给出明确、可操作的安全提示。

**Architecture:** 保留后端 Scene 为唯一事实来源。离散字体选择先提交 `Op`，再复用现有 renderer pipeline 立即重渲染；输入、滑杆等连续编辑继续使用 500ms debounce。字体批量操作只修改 `fontFamilies`，保留每个文本框的字号、颜色、描边、效果和对齐。当前 OCR 只返回节点级文本，不产生逐行几何，因此不伪造 `linePolygons`；HanOnly 下缺少可靠逐行几何的混合文本继续由后端安全跳过，并在文本块面板显示原因与处理建议。

**Tech Stack:** React 19、TypeScript、Zustand、TanStack Query、Vitest、Testing Library、MSW、Bun、Rust/Tauri（只做回归验证）。

---

## 已核实的当前行为

- `RenderControlsPanel.tsx` 在有选择时写入节点 `style.fontFamilies`，没有选择时只写 `preferences.defaultFont`，因此“全局”不会改变当前页已有文本节点。
- 所有样式修改都调用 `queueAutoRender()`，其固定等待 500ms；调用失败只写 `console.error`，用户无法立即确认字体是否真正渲染。
- Renderer 仅在节点 `style.font_families` 为空时使用 `defaultFont`。已有显式字体的节点不会被新的全局默认字体覆盖。
- 节点颜色、描边、字号和字体预测独立解析。统一字体不应顺便覆盖这些视觉属性。
- PaddleOCR-VL 和 MIT48px OCR 都只回写节点级 `text`，没有输出逐行坐标。后端 `eligible_text_lines()` 对中英混合节点要求数量匹配且安全的 `line_polygons`，缺失时返回 unsupported。这是防止英文被擦除的安全边界，不能用等高切分或猜测框绕过。

## 范围与不变量

### In Scope

- 字体 family/variant 的当前页全局作用域与选中作用域。
- 字体变更后立即启动当前配置的 Renderer。
- 自动渲染失败的可见错误提示。
- HanOnly 下缺少逐行几何的中英混合节点状态提示。
- UI 单元/组件测试及现有后端几何安全回归。

### Out of Scope

- 不修改 OCR 模型、Detector、Segment、Translate、Inpaint 或 Renderer 排版算法。
- 不生成虚假的 `linePolygons`，不把节点矩形等分为行框。
- 不强制统一字号、颜色、描边、效果或对齐；这些仍由现有独立控件控制。
- 不修改 HTTP 请求结构、OpenAPI schema、Rust Scene 类型或 `source_text_policy`。
- 不新增依赖、Zustand store、Renderer trait、后端 DTO 或新 helper 文件。

### 核心不变量

- 字体 Op 成功提交后才允许启动立即渲染。
- 没有选中节点时：更新 `defaultFont`，并给当前页全部文本节点写入同一 `fontFamilies`。
- 有选中节点时：只更新选中节点，不修改全局 `defaultFont`。
- 字体修改只改变 `fontFamilies`；每个节点原有 `fontSize`、`color`、`stroke`、`effect`、`textAlign` 必须保持。
- 字体 family 和 variant 的行为一致；未缓存 Google Font 必须先下载成功，再提交 Op 和渲染。
- 立即渲染会取消尚未执行的 debounce timer，避免同一操作重复启动 Renderer。
- 文本输入、字号、颜色等现有连续编辑仍走 debounce，不扩大本次立即渲染范围。
- HanOnly 混合节点没有可靠逐行几何时不得猜测、不得翻译/擦除英文；UI 只显示安全状态和解决建议。
- AllText 不受混合几何提示限制，继续使用现有节点级行为。

## Task 1：为现有自动渲染增加可等待的立即执行入口

**Files:**

- Modify: `ui/lib/io/scene.ts:91-129`
- Test: `ui/tests/lib/io/scene.test.ts`

### Step 1：先写失败测试

在 `ui/tests/lib/io/scene.test.ts` 增加 `describe('auto render')`，通过 MSW 记录 `/api/v1/pipelines` 请求，并使用 fake timers 验证：

1. `runAutoRenderNow('p1')` 立即读取 `/config` 并启动配置中的 renderer。
2. 请求包含 `steps: ['koharu-renderer']`、`pages: ['p1']` 和当前 `defaultFont`。
3. 先调用 `queueAutoRender('p1')` 再调用 `runAutoRenderNow('p1')`，推进 500ms 后仍只有一次 pipeline 请求。
4. renderer 为空时不发 pipeline 请求。
5. `/config` 或 `/pipelines` 失败时调用 `useEditorUiStore.getState().showError(...)`，且 Promise 正常结束，不产生未处理 rejection。

测试名称固定为：

```text
runAutoRenderNow starts the configured renderer immediately
runAutoRenderNow cancels the pending debounced render
runAutoRenderNow skips when no renderer is configured
runAutoRenderNow surfaces pipeline failures through the editor error state
```

### Step 2：运行并确认预期失败

```bash
bun run --filter ui test -- tests/lib/io/scene.test.ts -t runAutoRenderNow
```

预期：FAIL，原因是 `runAutoRenderNow` 尚未导出。

### Step 3：实现最小共享执行路径

在 `ui/lib/io/scene.ts`：

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
bun run --filter ui test -- tests/lib/io/scene.test.ts -t runAutoRenderNow
bun run --filter ui test -- tests/lib/io/scene.test.ts
```

预期：新增测试和原有 scene 测试全部 PASS。

### Step 5：提交边界

仅在用户另行授权提交时执行：

```bash
git add ui/lib/io/scene.ts ui/tests/lib/io/scene.test.ts
git commit -m "fix(ui): add immediate auto render execution"
```

## Task 2：修正字体作用域并在字体选择后立即渲染

**Files:**

- Modify: `ui/components/panels/RenderControlsPanel.tsx:286-328,429-483`
- Test: `ui/tests/components/RenderControlsPanel.test.tsx`

### Step 1：先改测试为目标语义并确认失败

扩展 `vi.mock('@/lib/io/scene')`，同时 mock `runAutoRenderNow`。新增或替换测试：

```text
changing global font updates every text node and the default font
changing a selected font updates only selected nodes and leaves the default font unchanged
changing a font preserves each node non-font style fields
changing a font awaits applyOp before immediate render
changing a font variant uses the same scope and immediate render behavior
downloading a Google font completes before applying and rendering it
```

关键断言：

- 无选择时 `applyOp` 收到包含当前页全部 Text 节点的 batch。
- `defaultFont` 更新为选择的 PostScript name。
- 有选择时 batch 只包含 `selectedNodes`，`defaultFont` 保持原值。
- `runAutoRenderNow('p1')` 被调用一次，`queueAutoRender` 没有因字体操作被调用。
- 用受控 Promise 暂停 `applyOp`，在 resolve 前断言 `runAutoRenderNow` 未调用；resolve 后才调用。
- 输入节点使用不同的 `fontSize/color/stroke/effect/textAlign`，生成的每个 patch 只替换 `fontFamilies`，其他字段保持各自原值。

### Step 2：运行并确认预期失败

```bash
bun run --filter ui test -- tests/components/RenderControlsPanel.test.tsx -t "global font|selected font|font variant|Google font"
```

预期：FAIL。当前无选择路径只更新 `defaultFont`，并且字体操作只进入 debounce 队列。

### Step 3：让现有样式写入函数支持立即渲染

把 `applyStyleToNodes` 改为返回 `Promise<void>`，增加一个默认 `false` 的 `immediate` 参数；不新增 DTO 或 helper 文件：

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

### Step 4：统一 family 与 variant 的目标节点规则

两个 handler 都按同一规则执行：

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
bun run --filter ui test -- tests/lib/io/scene.test.ts tests/components/RenderControlsPanel.test.tsx
```

预期：全部 PASS；字体变化走立即渲染，其他样式编辑仍走 debounce。

### Step 6：提交边界

仅在用户另行授权提交时执行：

```bash
git add ui/components/panels/RenderControlsPanel.tsx ui/tests/components/RenderControlsPanel.test.tsx
git commit -m "fix(ui): apply font scope and render immediately"
```

## Task 3：显示 HanOnly 混合文本的安全跳过状态

**Files:**

- Modify: `ui/lib/api/index.ts`
- Modify: `ui/components/panels/TextBlocksPanel.tsx:25-35,184-196,206-362`
- Test: `ui/tests/components/TextBlocksPanel.test.tsx`
- Modify: `ui/public/locales/en-US/translation.json`
- Modify: `ui/public/locales/zh-CN/translation.json`
- Modify: `ui/public/locales/zh-TW/translation.json`
- Modify: `ui/public/locales/ja-JP/translation.json`
- Modify: `ui/public/locales/ko-KR/translation.json`
- Modify: `ui/public/locales/pt-BR/translation.json`
- Modify: `ui/public/locales/ru-RU/translation.json`
- Modify: `ui/public/locales/es-ES/translation.json`
- Modify: `ui/public/locales/tr-TR/translation.json`

### Step 1：先写失败组件测试

在 `TextBlocksPanel.test.tsx` 扩展 fixture，使 Text 节点可传入 `text` 和 `linePolygons`。通过 `/api/v1/config` 返回 `source_text_policy`，测试：

```text
shows a safe-skip warning for HanOnly mixed text without line polygons
shows a safe-skip warning for HanOnly mixed text with mismatched line polygons
does not warn for pure Han or pure non-Han text
does not warn for mixed text with matching line polygons
does not block mixed text in AllText mode
```

断言警告使用稳定 test id，例如 `textblock-geometry-warning-0`。HanOnly unsupported 卡片的 Generate 按钮 disabled；AllText 同一节点保持原有 LLM/processing 禁用规则。

### Step 2：运行并确认预期失败

```bash
bun run --filter ui test -- tests/components/TextBlocksPanel.test.tsx -t "safe-skip|mixed text|AllText"
```

预期：FAIL，当前 UI 没有策略查询或几何状态。

### Step 3：复用 API facade 的既有 query 模式

在 `ui/lib/api/index.ts`：

- 从 generated imports 引入 `getConfig`。
- 从 schemas 引入 `AppConfig`。
- 增加与现有 `useGetMeta/useGetCurrentLlm` 同形的 `useGetConfig`。

```ts
export const useGetConfig = (options?: ApiQueryOptions<AppConfig>) =>
  useApiQuery(getGetConfigQueryKey(), getConfig, options)
```

不要新建 hook 文件，不改生成代码。

### Step 4：在现有组件内做最小显示判定

在 `TextBlocksPanel.tsx` 内保留一个局部纯函数，仅识别当前 UI 能可靠知道的“缺失/数量不匹配”：

```ts
const HAN = /\p{Script=Han}/u

function lacksMixedLineGeometry(node: TextNodeEntry): boolean {
  const lines = (node.data.text ?? '')
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean)
  const hanLines = lines.filter((line) => HAN.test(line)).length
  if (hanLines === 0 || hanLines === lines.length) return false
  return node.data.linePolygons?.length !== lines.length
}
```

- `source_text_policy` 缺失时按后端默认值 `han_only` 处理。
- 仅当 HanOnly 且 `lacksMixedLineGeometry(node)` 为 true 时显示 warning，并禁用该卡片 Generate。
- 提示不得回显 OCR 正文；中文建议为：“仅中文模式无法安全定位此混合文本的中文行。请用框选工具拆成独立文字框，或切换 AllText。”
- 有数量匹配的 polygons 时，UI 不重复实现后端有限性、相交和轴对齐校验；后端仍是最终安全边界。
- 不修改 `eligible_text_lines()`，不新增等高 fallback，不根据换行数量猜测几何。

### Step 5：补齐现有九种语言的同名 key

在每个 `translation.json` 的 `textBlocks` 下新增：

```json
"mixedGeometryWarning": "..."
```

所有 locale 必须包含相同 key；不能只依赖英文 fallback。

### Step 6：运行测试与后端安全回归

```bash
bun run --filter ui test -- tests/components/TextBlocksPanel.test.tsx
bun cargo test -p koharu-app eligible_mixed_node_
```

预期：

- UI 测试 PASS。
- 后端测试继续证明缺失/错配/不安全 polygon 返回 unsupported，不会用节点矩形猜测混合行。

### Step 7：提交边界

仅在用户另行授权提交时执行：

```bash
git add ui/lib/api/index.ts ui/components/panels/TextBlocksPanel.tsx ui/tests/components/TextBlocksPanel.test.tsx ui/public/locales
git commit -m "fix(ui): explain unsupported mixed text geometry"
```

## Task 4：全量回归与桌面人工验收

**Files:**

- No production file changes.
- Manual fixture: `/Users/jinkui/Desktop/截屏2026-07-12 13.40.11.png` 对应的原项目页面。

### Step 1：运行 UI 定向回归

```bash
bun run --filter ui test -- tests/lib/io/scene.test.ts tests/components/RenderControlsPanel.test.tsx tests/components/TextBlocksPanel.test.tsx
```

预期：全部 PASS，且测试不加载或下载 ML 模型。

### Step 2：运行项目质量门禁

```bash
bun run format:check
bun run lint:ui
bun run test:ui
bun cargo fmt --all -- --check
bun cargo check --workspace --all-targets
bun cargo test -p koharu-app eligible_mixed_node_
bun run check:generated
bun run build
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
6. `S-CURVE / S型曲线` 这类缺少 `linePolygons` 的混合节点在 HanOnly 下显示安全提示，Generate 禁用；英文不会因猜测行框被擦除。
7. 将该节点拆成独立英文框和中文框后，中文框可单独生成；或切换 AllText 后按原节点级行为处理。
8. 自动渲染失败时出现现有错误提示，不只写开发者控制台。

### Step 4：停止条件

只有同时满足以下条件才算计划实施完成：

- 字体 family/variant 的全局与选中作用域测试通过。
- 字体 Op 与立即 Renderer 的顺序测试通过，且没有重复 debounce 请求。
- 非字体样式字段保持测试通过。
- 混合文本提示与 AllText 兼容测试通过。
- 后端 mixed geometry 安全测试通过。
- 当前问题页面完成上述人工验收。
- 无非预期生成物或无关文件修改。

## 预期修改规模

- 生产实现：约 45–70 行。
- 测试：约 100–150 行。
- 多语言文本：9 个同名 key。
- 新依赖：0。
- 新 production 文件：0。
- Rust/OpenAPI/schema 修改：0。

## 已知限制

- 统一字体只统一 font family/variant，不会自动统一颜色、描边和字号；这是避免破坏节点已有设计的明确选择。
- 现有 OCR 没有逐行几何输出。HanOnly 无法安全自动处理缺少 `linePolygons` 的中英混合节点；本次提供安全提示与拆框/AllText 处理路径，不伪造几何。
- 若未来需要自动处理此类混合节点，应另立计划，让 Detector/OCR 产出真实逐行 polygon，并以实际模型输出和图像坐标回归测试证明；不应在本计划中增加启发式切分。
