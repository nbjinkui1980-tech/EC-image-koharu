# 电商图片翻译产品化与模块化 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** 在保持 Koharu 现有漫画能力、HTTP/OpenAPI 兼容和 HanOnly 英文保护不变量的前提下，将项目升级为可批量、可复核、可配置本地/云端阶段 Provider 的电商图片翻译工作台。

**Architecture:** 继续使用现有 Scene/Op、Artifact DAG、Engine inventory、LLM Provider、Jobs/SSE 和 React Query facade，不建立第二套工作流框架。先完成既有字体与即时渲染计划并拆分已过大的共享模块，再增加最小的电商域策略、翻译约束、质量检查和输出；真实逐行几何与第一个远程 Engine 都是停止式决策门，未选定真实实现前不创建通用 Provider 抽象。

**Tech Stack:** Rust 2024、Serde/Postcard、Utoipa/OpenAPI、Axum、Candle/llama.cpp、`image`/`imageproc`、React 19、TypeScript、TanStack Query、Zustand、Vitest、MSW、Bun、Tauri。

---

## 1. 计划定位与当前基线

### 1.1 已完成基础层

`docs/plan/2026-07-11-ecommerce-chinese-only-translation-quality-plan.md` 已在当前分支形成以下提交：

- `3440b10a`：`SourceTextPolicy` 与入口继承。
- `a228dfe0`：安全 Han 行目标与 unsupported geometry。
- `a9c118b4`：Segment/Inpaint 最终 mask、Flux2 与 DAG。
- `235a7949`：严格逐行翻译和原子写回。
- `aca3dd86`：HanOnly 显式行布局。
- `d15776c5`：中文使用限制文档。

本规划不重复实现这些任务，只把它们作为回归不变量。当前工作树仍有未提交的 Renderer/CUDA 后续修复，开始任何新阶段前必须先按实际 diff 审查和验证，不能覆盖或混入无关提交。

### 1.2 必须纳入的待完成编辑层

完整纳入：

`docs/plan/2026-07-12-font-application-immediate-render-and-mixed-text-plan.md`

其四项结果作为本规划 Phase 1：

1. 可等待的立即自动渲染入口。
2. 字体 family/variant 的全局/选中作用域。
3. HanOnly 混合文本缺几何的安全提示。
4. UI、Rust 安全回归与问题图片人工验收。

本规划额外补充一个根因修复：字体下拉列表不得为每个可见 Google Font 持久下载字体。当前 `FontRow` 把所有虚拟行传为 `isVisible={true}`，本机已形成 1,384 个字体目录；列表只显示名称，只有明确选择的 family/variant 才下载。

## 2. 产品范围

### 2.1 In Scope

- 中文电商文案检测、OCR、翻译、修补和重绘。
- 英文、品牌、SKU、型号、价格、数字、单位和用户保护文本不被误译或擦除。
- 项目级保护词、术语表和目标市场语言。
- 真实逐行 polygon、方向和置信度的几何门禁。
- 字体即时预览、作用域、基础电商文字样式。
- 自动质量检查、问题页重试和批量导出。
- 每个 Engine 可选本地实现，远程 Engine 可独立绑定 Provider/Model。
- PNG/JPEG/WebP/PSD/KHR 输出及可复现命名。

### 2.2 Out of Scope

- 不建立插件市场、通用工作流 DSL、多租户 SaaS 或分布式任务系统。
- 不新增第四种 Inpaint 模型，除非电商基准证明现有三种均无法达标。
- 不用节点等高切分、OCR 换行数或旋转猜测生成 `linePolygons`。
- 不在第一阶段实现渐变文字、曲线文字、任意透视或完整富文本编辑器。
- 不复制现有 HTTP 客户端、Scene reducer、Provider registry 或 Jobs store。
- 不提交真实商家图片；真实基准由本地受控目录提供，自动化只用合成小图。

## 3. 核心不变量与验收标准

- [ ] HanOnly 纯英文、protected 和 unsupported 节点不进入 Segment、Provider、Inpaint 或 Renderer。
- [ ] 所有 Inpainter 最终 mask 外逐像素等于输入基面；Repair Brush region 外不变。
- [ ] 品牌、SKU、数字、百分比、货币和尺寸单位验证失败时不产生任何部分 translation ops。
- [ ] 新版本可打开修改前生成的 Scene/Postcard 和 history log；旧 TOML、AllText 节点级 Provider、legacy fallback 和通用布局保持可读可用。
- [ ] 字体 family/variant 在 Op 成功后立即渲染；无选择时应用当前页全部 Text 节点，有选择时只应用选中节点。
- [ ] 打开或滚动字体列表不会下载 Google Font；明确选择后最多下载所选 family/variant。
- [ ] 中英混合节点缺少真实逐行几何时显示可操作提示，不猜测、不静默生成错误图。
- [ ] MenuBar、CanvasToolbar、TextBlocks Generate 使用同一目标语言优先级和即时错误反馈。
- [ ] 已实施的每个远程 Engine 独立选择 `provider_id + model_id`；本地 Engine 不依赖远程配置。
- [ ] 同一项目不会同时运行两个修改 Scene 的 pipeline；失败页面可按 PageId 重试。
- [ ] HTTP 与 MCP 通过同一个原子单写入 guard；成功、失败和取消后都释放 guard。
- [ ] JPEG/WebP 导出保留页面尺寸；JPEG 质量值边界校验，WebP 使用现有 lossless encoder；默认 PNG/PSD/KHR 行为不变。
- [ ] completed_with_errors/failed 在 UI 保留到 dismiss；质量 warning 可定位 PageId/NodeId，重试只覆盖失败页。
- [ ] Rendered/Inpainted 导出可在 UI 选择 PNG/JPEG/WebP 和 JPEG quality，失败可见且不会关闭输入。
- [ ] 默认自动化测试不下载模型；真实模型与真实图片验证明确标记为本地验收。
- [ ] 每阶段完成后可独立通过其定向测试和 workspace check，不依赖后续阶段才能编译。

## 4. 模块与目录规划

只在对应 Phase 开始时创建真实使用的文件，不预建空目录或占位 trait。

```text
crates/koharu-core/src/
  ecommerce/
    mod.rs                 # 仅重导出
    policy.rs              # TextRole、EcommerceProjectConfig、GlossaryEntry
  scene.rs                 # 只增加字段，不承载业务判断
  op.rs                    # 追加独立 Op 变体，不改变旧 Patch 的 Postcard 布局

crates/koharu-app/src/
  legacy_scene.rs          # Task 3 才创建：只负责旧 Postcard 快照迁移
  ecommerce/
    mod.rs                 # 仅重导出
    policy.rs              # 保护词、文本角色、数字/单位提取
    translation.rs         # prompt 约束与严格后验验证
    quality.rs             # Phase 4 才创建：页面检查
  pipeline/engines/support/
    mod.rs                 # 保持原公开 re-export
    scene.rs               # Source/Blob/Node/Upsert 辅助
    han.rs                 # Han 行 eligibility、geometry、cleanup/translation ops
    mask.rs                # support mask、intersection、prepare_inpaint_mask
    # reading-order 单函数保留在 han.rs，不为它单建文件
  pipeline/engines/remote/
    mod.rs                 # 选定第一个远程 Engine 后才创建
    ocr.rs                 # 由独立 Provider 子计划确定，不预建

ui/components/
  export/
    ExportDialog.tsx               # Task 9 才创建：图片格式和 JPEG quality
  settings/
    EcommerceSettingsPane.tsx
  panels/render/
    AdvancedTextStyleControls.tsx   # Phase 5 才创建
  operations/
    FailedPagesActions.tsx          # 批量重试任务才创建

ui/lib/
  io/pipeline.ts           # Task 9：三个 pipeline 入口共享目标语言、请求记录和即时错误
  api/index.ts             # 保持稳定 facade；不手改 generated.ts/schemas

tests/integration-tests/tests/
  ecommerce_pipeline.rs
  ecommerce_export.rs
```

### 4.1 所有权规则

- `koharu-core`：只放跨 App/RPC/UI schema 的纯数据类型，不放 I/O、模型或规则执行。
- `koharu-app::ecommerce`：只放纯策略与质量逻辑；Pipeline Engine 负责调用，不复制规则。
- `pipeline/engines/support`：只放多个 Engine 已经复用的基础函数。
- `koharu-renderer`：只负责布局和栅格化，不判断品牌、语言和 Provider。
- `koharu-rpc`：只做请求校验、启动、导出和 schema，不放电商业务规则。
- UI：后端为事实来源；generated API 不手改，Zustand 只保存本地偏好，不镜像 Scene。

### 4.2 防止文件继续膨胀

- 当前 `support.rs` 约 1,452 行，必须在增加电商规则前拆分。
- 新功能不得继续写入约 1,378 行的 `SettingsDialog.tsx`；只增加 pane import 和 tab 接线。
- `renderer.rs` 和 `RenderControlsPanel.tsx` 只做委托接线；新增高级样式放入所属模块。
- 一个 Phase 若让现有文件净增超过 100 行，必须证明这些代码无法放入已有责任模块；不能为“以后可能复用”提前抽象。
- `mod.rs` 只声明模块和重导出；禁止成为新的工具杂物箱。
- 测试与实现同模块或放入现有测试文件；不为单个纯函数建立测试框架。

## 5. 分阶段执行计划

Task 0–5、7–9 是本文件的可执行生产范围。Task 6 和 Task 10 是明确停止门：没有基准证据或具体 Provider 时只记录 NOT APPLICABLE/BLOCKED，不创建 production skeleton。Task 11 只验收已经实施的能力；远程 Provider 子计划未实施前不得宣称“每阶段远程 Provider”已经完成。

### Task 0：冻结基线与电商验收矩阵

**Files:**

- Read: `docs/plan/2026-07-11-ecommerce-chinese-only-translation-quality-plan.md`
- Read: `docs/plan/2026-07-12-font-application-immediate-render-and-mixed-text-plan.md`
- Read: `docs/zh-CN/project-functional-analysis.md`
- Verify: `crates/koharu-app/src/pipeline/engines/*`
- Verify: `ui/components/panels/*`

**Step 1: 记录基线**

```bash
git status --short
git log -8 --oneline
wc -l crates/koharu-app/src/pipeline/engines/support.rs \
  crates/koharu-app/src/renderer.rs \
  ui/components/SettingsDialog.tsx \
  ui/components/panels/RenderControlsPanel.tsx
```

Expected: 明确记录既有 dirty files，后续提交不得包含无关文件。

**Step 2: 运行基础回归**

```bash
bun cargo test -p koharu-app eligible_mixed_node_
bun cargo test -p koharu-app han_only_renderer_
bun cargo test -p koharu-app strict_translation_
bun run --filter ui test -- tests/lib/io/autoRender.test.ts
```

Expected: 现有 HanOnly/AllText 不变量通过；失败必须先作为基线问题处理，不能混入新功能。

**Step 3: 建立本地真实图片 manifest**

在未跟踪目录 `$KOHARU_ECOMMERCE_BENCHMARK_DIR` 保存至少 50 张图片和 `manifest.json`。每个样本只记录可复现标注，不保存模型输出：

```json
{
  "images": [
    {
      "file": "fashion/001.png",
      "targets": [
        {
          "kind": "translatable_han",
          "polygon": [[10, 10], [90, 10], [90, 30], [10, 30]],
          "direction": "horizontal"
        }
      ]
    }
  ]
}
```

样本覆盖服装、美妆、食品、电子产品、尺寸表、促销海报、包装文字、中英混排、旋转文字和复杂纹理。真实图片、manifest 和逐图结果都不提交；仓库只提交聚合指标和不含 OCR 正文的结论。

**Commit:** 无代码提交。

### Task 1：完成字体即时渲染计划并阻止字体缓存失控

**Files:**

- Execute: `docs/plan/2026-07-12-font-application-immediate-render-and-mixed-text-plan.md`
- Modify: `ui/lib/io/scene.ts:91-129`
- Modify: `ui/components/panels/RenderControlsPanel.tsx:286-483`
- Modify: `ui/components/panels/TextBlocksPanel.tsx:25-362`
- Modify: `ui/lib/api/index.ts`
- Modify: `ui/components/ui/font-select.tsx:40-115,296-323`
- Test: `ui/tests/lib/io/autoRender.test.ts`
- Test: `ui/tests/components/RenderControlsPanel.test.tsx`
- Test: `ui/tests/components/TextBlocksPanel.test.tsx`
- Locale: `ui/public/locales/*/translation.json`

**Step 1: 先执行既有计划的失败测试**

按原计划完成：`runAutoRenderNow`、字体作用域、Op → immediate render 顺序、非字体 style 保持、混合几何提示和 AllText 兼容。原计划中所有 `ui/tests/lib/io/scene.test.ts` 引用由本路线图明确替换为现有 `ui/tests/lib/io/autoRender.test.ts`，不得新建重复测试文件。

**Step 2: 增加字体缓存根因失败测试**

在现有 `RenderControlsPanel.test.tsx` 通过 MSW 记录 `/api/v1/google-fonts/{family}/fetch`：

- 打开并滚动字体列表不产生 fetch。
- 选择一个未缓存 Google Font 只产生一次 fetch。
- 下载失败不提交 style Op、不启动 Renderer，并显示错误。

Run:

```bash
bun run --filter ui test -- tests/components/RenderControlsPanel.test.tsx -t "Google font"
```

Expected: FAIL，当前每个可见 `FontRow` 都调用 `useGoogleFontPreview()`。

**Step 3: 最小实现**

- `FontRow` 不调用持久下载 preview hook，Google Font 行先使用普通 UI 字体显示名称。
- 保留 `useGoogleFontPreview()` 给当前已选字体和已选 family 的 variant。
- family/variant 明确选择后才调用 `fetchGoogleFont()`。
- 不增加预览 CDN、LRU、缓存数据库或清理守护程序。

**Step 4: 验证**

```bash
bun run --filter ui test -- tests/lib/io/autoRender.test.ts \
  tests/components/RenderControlsPanel.test.tsx \
  tests/components/TextBlocksPanel.test.tsx
bun run lint:ui
```

Expected: PASS；字体列表浏览不再增长磁盘缓存。

**Commit boundary:**

```bash
git add ui/lib/io/scene.ts ui/lib/api/index.ts \
  ui/components/ui/font-select.tsx \
  ui/components/panels/RenderControlsPanel.tsx \
  ui/components/panels/TextBlocksPanel.tsx \
  ui/tests/lib/io/autoRender.test.ts \
  ui/tests/components/RenderControlsPanel.test.tsx \
  ui/tests/components/TextBlocksPanel.test.tsx \
  ui/public/locales/en-US/translation.json \
  ui/public/locales/es-ES/translation.json \
  ui/public/locales/ja-JP/translation.json \
  ui/public/locales/ko-KR/translation.json \
  ui/public/locales/pt-BR/translation.json \
  ui/public/locales/ru-RU/translation.json \
  ui/public/locales/tr-TR/translation.json \
  ui/public/locales/zh-CN/translation.json \
  ui/public/locales/zh-TW/translation.json
git commit -m "fix(ui): apply fonts immediately without preview cache growth"
```

### Task 2：拆分现有 Pipeline support 热点，不改变行为

**Files:**

- Delete: `crates/koharu-app/src/pipeline/engines/support.rs`
- Create: `crates/koharu-app/src/pipeline/engines/support/mod.rs`
- Create: `crates/koharu-app/src/pipeline/engines/support/scene.rs`
- Create: `crates/koharu-app/src/pipeline/engines/support/han.rs`
- Create: `crates/koharu-app/src/pipeline/engines/support/mask.rs`
- Verify callers: `crates/koharu-app/src/pipeline/engines/*.rs`

**Step 1: 锁定现有回归**

```bash
bun cargo test -p koharu-app pipeline::engines::support::tests
bun cargo test -p koharu-app eligible_mixed_node_
bun cargo test -p koharu-app final_inpaint_mask_
bun cargo test -p koharu-app han_translation_ops_
```

Expected: PASS。

**Step 2: 机械移动**

- `scene.rs`：source/load/find/text/upsert/new/clear helpers。
- `han.rs`：`EligibleTextLine`、unsupported、Han 检测、安全 bbox、translation cleanup/ops 和现有单个 reading-order sort。
- `mask.rs`：support rasterization、mask intersection、region clip、final mask preparation。
- `mod.rs`：保持现有公开名称和 re-export，调用方不改 import path。
- 测试随其实现移动；不改断言、不顺带重命名 API。

**Step 3: 验证零行为差异**

```bash
bun cargo fmt --all -- --check
bun cargo check -p koharu-app --all-targets
bun cargo test -p koharu-app pipeline::engines::support
git diff --check
```

Expected: PASS；除模块移动和必要 import 外无逻辑 diff。

**Commit boundary:**

```bash
git add crates/koharu-app/src/pipeline/engines/support.rs \
  crates/koharu-app/src/pipeline/engines/support
git commit -m "refactor(pipeline): split shared engine support modules"
```

### Task 3：增加最小电商文本策略、保护词和术语表

**Files:**

- Create: `crates/koharu-core/src/ecommerce/mod.rs`
- Create: `crates/koharu-core/src/ecommerce/policy.rs`
- Modify: `crates/koharu-core/src/lib.rs`
- Modify: `crates/koharu-core/src/scene.rs:116-143,277-312`
- Modify: `crates/koharu-core/src/op.rs`
- Create: `crates/koharu-app/src/legacy_scene.rs`
- Modify: `crates/koharu-app/src/session.rs:38-43,156-220`
- Modify: `crates/koharu-app/src/history.rs`
- Create: `crates/koharu-app/src/ecommerce/mod.rs`
- Create: `crates/koharu-app/src/ecommerce/policy.rs`
- Create: `crates/koharu-app/src/ecommerce/translation.rs`
- Modify: `crates/koharu-app/src/lib.rs`
- Modify: `crates/koharu-app/src/pipeline/engines/support/han.rs`
- Modify: `crates/koharu-app/src/pipeline/engines/llm_translate.rs`
- Modify: `ui/components/SettingsDialog.tsx`
- Create: `ui/components/settings/EcommerceSettingsPane.tsx`
- Modify: `ui/components/panels/TextBlocksPanel.tsx`
- Test: existing Rust/UI test modules
- Generated: `ui/openapi.json`, `ui/lib/api/schemas/*`

**Step 1: 先写旧 Postcard 和 history 兼容失败测试**

测试名称：

```text
legacy_v1_snapshot_migrates_with_ecommerce_defaults
legacy_history_replays_after_ecommerce_ops_are_appended
new_snapshot_round_trips_ecommerce_config_and_text_roles
```

测试必须使用修改前真实字段顺序编码的 V1 snapshot/log，不得用新类型先序列化再反序列化冒充兼容测试。

Run:

```bash
bun cargo test -p koharu-app legacy_v1_snapshot_migrates_with_ecommerce_defaults
bun cargo test -p koharu-app legacy_history_replays_after_ecommerce_ops_are_appended
```

Expected: FAIL，当前没有 V2 envelope、V1 decoder 或新增电商状态。

**Step 2: 写策略失败测试**

测试名称：

```text
protected_text_never_becomes_han_target
review_required_text_is_not_auto_processed
translation_invariants_preserve_terms_numbers_and_units
translation_invariant_failure_builds_no_ops
all_text_still_honors_explicit_protected_role
```

最小数据模型：

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum TextRole {
    Translatable,
    Protected,
    ReviewRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GlossaryEntry {
    pub source: String,
    pub target: String,
    pub target_language: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToSchema)]
#[serde(default, rename_all = "camelCase")]
pub struct EcommerceProjectConfig {
    pub enabled: bool,
    pub target_language: Option<String>,
    pub protected_terms: Vec<String>,
    pub glossary: Vec<GlossaryEntry>,
}
```

`ProjectMeta.ecommerce` 使用 `#[serde(default)]`，默认 `enabled = false`；`TextData.text_role: Option<TextRole>` 使用 `#[serde(default)]`，`None` 表示沿用现有 HanOnly/AllText 自动判断。

不能只依赖 `#[serde(default)]` 读取旧 Postcard：Postcard struct 按目标字段数解码，追加字段会改变布局。

Run:

```bash
bun cargo test -p koharu-core ecommerce
bun cargo test -p koharu-app translation_invariant_
```

Expected: FAIL，类型和策略函数不存在。

**Step 3: 实现最小 V2 snapshot 迁移和独立 Op**

- 新 snapshot 以固定 `KHR2` magic、版本号、epoch 和当前 Scene 编码；加载时先检查原始 magic，V2 解析失败必须报错，不能静默降级。
- 非 `KHR2` 文件只按 `legacy_scene::SnapshotV1` 解码一次；先用同模块的 `LogFrameV1/OpV1` 重放旧 history，再把最终旧 Scene 转换为当前类型并填入默认电商状态。测试中的旧 log 必须包含带 TextData 的 AddNode 或 AddPage，不能只测空操作。
- `legacy_scene.rs` 只复制 V1 snapshot/history 解码确实需要的 Scene、ProjectMeta、Page、Node、NodeKind、TextData、Op 和相关 Patch wire structs；布局未变化且不会嵌套变更类型的字段直接复用。
- V1 snapshot 和全部有效 log frame 成功重放后，才原子写入 V2 snapshot 并截断旧 log；任一步失败都保留原文件并返回错误，不能丢弃无法解码的历史。
- 不给现有 `ProjectMetaPatch` 或 `TextDataPatch` 追加字段，避免旧 history 的 struct 布局变化。
- 在 `Op` 枚举末尾追加 `UpdateEcommerceConfig { config, prev }` 和 `UpdateTextRole { page, id, role, prev }`；旧变体索引不变。
- 新 snapshot 写入和 history replay 测试全部 PASS 后，才继续业务策略。

Run:

```bash
bun cargo test -p koharu-app legacy_v1_snapshot_migrates_with_ecommerce_defaults
bun cargo test -p koharu-app legacy_history_replays_after_ecommerce_ops_are_appended
bun cargo test -p koharu-app new_snapshot_round_trips_ecommerce_config_and_text_roles
```

Expected: PASS。

**Step 4: 实现唯一策略边界**

- `ecommerce::policy` 负责 `TextRole`、项目保护词、品牌/SKU/数字/单位 token 提取。
- `support::han` 只调用策略，不复制保护词判断。
- `ecommerce::translation` 在 strict Provider 结果完整映射后、构造 ops 前验证术语和 token。
- 验证失败返回 Err；不得构造部分 translation/sprite cleanup ops。
- 第一版只做精确 source token/term 保持；不做货币、数字或单位本地化换算。
- 对保护词先按 Unicode trim、去空项并做精确子串匹配；不增加 NLP 依赖。

**Step 5: UI 接线**

- `SettingsDialog.tsx` 只接入 `EcommerceSettingsPane`，不继续增加 pane 实现。
- 文本块卡片提供 Auto/Translate/Protect/Review 状态；分别通过新增的两个 Op 持久化，不修改旧 Patch wire layout。
- 项目 `target_language` 只作为 UI 启动 pipeline 时现有 `targetLanguage` 请求字段的默认值，不改变 `StartPipelineRequest` JSON。
- 保护词与术语表是项目数据，不存入全局 Zustand 偏好。

**Step 6: OpenAPI 与回归**

```bash
bun cargo test -p koharu-core ecommerce
bun cargo test -p koharu-app legacy_v1_snapshot_
bun cargo test -p koharu-app legacy_history_
bun cargo test -p koharu-app translation_invariant_
bun cargo check -p koharu-core -p koharu-app --all-targets
bun run generate:api
bun run --filter ui test -- tests/components/TextBlocksPanel.test.tsx
```

Expected: PASS；`StartPipelineRequest` 不新增字段，旧 snapshot/history 可读取，Scene/Project schema 只做向后兼容增量。检查并仅暂存预期生成物；此提交完成后才允许在最终门禁运行 `check:generated`。

**Commit boundary:** `feat(ecommerce): add protected text and glossary policy`

### Task 4：用电商基准选择真实逐行几何路径

**Files:**

- Verify: `crates/koharu-app/src/pipeline/engines/pp_doclayout.rs`
- Verify: `crates/koharu-app/src/pipeline/engines/ctd_full.rs`
- Verify: `crates/koharu-ml/src/comic_text_detector/*`
- Verify: `crates/koharu-app/src/pipeline/engines/paddle_ocr.rs`
- Create: `crates/koharu-ml/tests/ecommerce_geometry.rs`

**Step 1: 写最小 benchmark 计算失败测试**

在单个测试文件内定义 manifest DTO、IoU 匹配和四个聚合指标；先用两个内存 polygon 样本验证召回率、polygon 覆盖率、英文误检率和方向正确率。

Run:

```bash
bun cargo test -p koharu-ml --test ecommerce_geometry benchmark_metrics_
```

Expected: FAIL，metric helper 尚不存在。

**Step 2: 实现单文件 ignored harness**

- 复用现有 PP-DocLayout 与 Comic Text Detector API；不创建 benchmark framework、trait 或 production DTO。
- 普通测试只计算内存 DTO，不加载模型。
- `run_ecommerce_geometry_benchmark` 标记 `#[ignore]`，只在显式提供 `KOHARU_ECOMMERCE_BENCHMARK_DIR` 时读取 Task 0 manifest 和图片。
- 输出 `$KOHARU_ECOMMERCE_BENCHMARK_DIR/results/geometry.json`，只含文件相对路径、计数和聚合指标，不写 OCR 正文。

Run:

```bash
bun cargo test -p koharu-ml --test ecommerce_geometry benchmark_metrics_
```

Expected: PASS，且没有模型下载。

**Step 3: 显式运行真实模型门禁**

在本地 50 张基准图分别运行 PP-DocLayout 和 Comic Text Detector，记录：

- 可翻译中文区域召回率 ≥ 90%。
- 逐行 polygon 覆盖率 ≥ 95%。
- 品牌/英文误检率 ≤ 1%。
- rotation/direction 正确率 ≥ 95%。

Run:

```bash
KOHARU_ECOMMERCE_BENCHMARK_DIR=/absolute/private/benchmark \
  bun cargo test -p koharu-ml --test ecommerce_geometry \
  run_ecommerce_geometry_benchmark -- --ignored --nocapture
```

Expected: 测试明确打印两个现有模型的四项指标，并生成聚合 JSON；命令不会由默认 workspace tests 运行。

**Step 4: 停止式决策门禁**

- CTD 达标：复用现有 `comic-text-detector`，不新增 Engine。
- CTD 未达标：停止后续几何 production 改动，先选择一个能返回真实 line/word polygon 的本地或远程 OCR，再为该具体 Provider 写独立 child plan；未选定前不得进入远程 Engine 实施任务。
- 禁止把 `polygon_points` 的节点外框当逐行框；禁止等高、投影或换行猜测。

**Step 5: 只为已选现有 Engine 增加无模型契约测试**

若 CTD 达标，在现有 CTD engine 测试中增加内存 DTO → Scene Op 测试，证明 text、confidence、direction、rotation、line polygons 使用绝对图像坐标，且非法/越界几何在共享 Han 边界安全跳过。若未达标，本步骤属于后续具体 Provider 子计划，本 Task 不提交 production skeleton。

**Commit boundary:** benchmark harness 独立提交 `test(ecommerce): add reproducible geometry benchmark`；只有 CTD 达标并补齐现有 engine 契约测试时，再提交 `test(ecommerce): lock reliable CTD line geometry`。不提交新 Engine skeleton。

### Task 5：增加翻译与页面质量门禁

**Files:**

- Create: `crates/koharu-app/src/ecommerce/quality.rs`
- Modify: `crates/koharu-app/src/pipeline/mod.rs:162-275`
- Modify: `crates/koharu-core/src/events.rs`
- Modify: `crates/koharu-rpc/src/routes/pipelines.rs:130-195`
- Modify: `ui/components/ActivityBubble.tsx`
- Test: `crates/koharu-app/src/ecommerce/quality.rs`
- Test: `tests/integration-tests/tests/pipelines.rs`
- Test: `ui/tests/components/ActivityBubble.test.tsx`
- Generated: `ui/openapi.json`, `ui/lib/api/schemas/*`

**Step 1: 写纯质量规则失败测试**

覆盖：

- eligible Han 节点缺译文。
- protected 节点出现 translation/sprite。
- 品牌、SKU、数字、百分比、货币和尺寸单位漂移。
- sprite transform 越出页面或为非有限值。
- unsupported geometry 只产生安全元数据，不包含 OCR 正文。

Run: `bun cargo test -p koharu-app ecommerce::quality`

Expected: FAIL，quality 模块不存在。

页面结束时没有保存每次 Inpaint 的输入基面和最终有效 mask，因此 `inspect_page()` 不检查 mask 外像素。该不变量继续由 Lama/AOT/Flux2/Repair Brush 的生产 dispatch 测试证明；真实验收 harness 在运行前保存输入和有效 mask 后比较。

**Step 2: 写 warning 状态闭环失败测试**

测试名称：

```text
quality_issues_increment_run_outcome_warning_count
quality_warning_contains_page_and_optional_node_id_without_ocr_text
quality_warning_marks_http_job_completed_with_errors
```

Run:

```bash
bun cargo test -p koharu-app quality_issues_increment_run_outcome_warning_count
bun cargo test -p koharu-integration-tests --test pipelines quality_warning_
```

Expected: FAIL；当前 `WarningSink` 只是回调，只有 `report_step_failure()` 会增加 `warning_count`，`JobWarningEvent` 也没有稳定 PageId。

**Step 3: 复用现有 warning/status 路径**

- `QualityIssue` 保持在 `koharu-app::ecommerce::quality` 内，只含 code、PageId、可选 NodeId 和安全建议；不新建 core quality 模块。
- `inspect_page()` 返回 `Vec<QualityIssue>`，纯函数不加载模型。
- Pipeline 每页结束后仅在 `project.ecommerce.enabled` 时调用。
- 在 `pipeline/mod.rs` 增加一个小型 `report_quality_issues()`：对每个 issue 同时递增当前 `warning_count` 并调用现有 `WarningSink`，`step_id` 固定为 `ecommerce-quality`。
- `WarningTick` 携带 PageId、安全 code 和可选 NodeId；`JobWarningEvent` 增加可选 `page_id`、`code`、`node_id` 响应字段，现有 warning 字段和 HTTP 路径不变。
- HTTP 仍只依据 `RunOutcome.warning_count` 选择 `CompletedWithErrors`，不建立第二套状态来源。
- 不新增 Quality Artifact、第二个 Jobs store 或独立事件总线。
- OCR round-trip 真实图片检查保持本地验收，不放入默认测试。

**Step 4: UI 最小展示**

ActivityBubble 显示问题类型、PageId 和可操作建议；不回显敏感 OCR 正文。暂不建立完整审核数据库。

**Step 5: 验证**

```bash
bun cargo test -p koharu-app ecommerce::quality
bun cargo test -p koharu-app quality_issues_increment_run_outcome_warning_count
bun cargo test -p koharu-integration-tests --test pipelines quality_warning_
bun run --filter ui test -- tests/components/ActivityBubble.test.tsx
bun cargo test -p koharu-app pipeline::tests
bun cargo check -p koharu-app -p koharu-rpc --all-targets
bun run generate:api
```

Expected: PASS；质量 issue 会产生 `CompletedWithErrors`，PageId 可用于后续重试，消息不含 OCR 正文；非电商项目和 AllText 旧流程状态不变。仅暂存预期生成物，最终门禁再运行 `check:generated`。

**Commit boundary:** `feat(ecommerce): add post-pipeline quality gates`

### Task 6：按基准决定是否需要新增文字样式

本 Task 是 YAGNI 停止门，不直接修改 production 文件。

**Step 1: 统计真实返工原因**

从 Task 0 的 50 图结果分别统计 letter spacing、line height 和 shadow。某项只有在至少 3 张不同图片中是主要返工原因时才成立；否则标记 NOT APPLICABLE。

**Step 2: 需要时单独写一个字段的 child plan**

一次只规划一个达到门禁的字段。子计划必须覆盖 Rust/UI 输入验证、Horizontal/VerticalRl、hard-line、AllText、布局边界，以及旧 snapshot 和 history 中 `TextStyle` 的 Postcard 迁移；不能仅追加 serde 字段后声称兼容。

只有至少两个高级控件真实存在时才抽出 `AdvancedTextStyleControls`。字体选择仍只改 `fontFamilies`；商品保护继续由最终 mask 双重限制和 Inpainter 生产测试证明。

**Commit boundary:** 本停止门无代码提交；具体样式由独立计划实施。

### Task 7：单项目写入 guard 与失败页重试

**Files:**

- Modify: `crates/koharu-app/src/session.rs:46-53,88-122`
- Modify: `crates/koharu-rpc/src/routes/pipelines.rs`
- Modify: `crates/koharu-rpc/src/routes/operations.rs`
- Modify: `crates/koharu-rpc/src/mcp/mod.rs:182-217`
- Test: `tests/integration-tests/tests/pipelines.rs`

**Step 1: 写 guard 和入口失败测试**

测试名称：

```text
project_session_pipeline_guard_is_atomic
pipeline_guard_releases_after_drop
pipeline_guard_http_conflicts_while_held
pipeline_guard_mcp_and_http_use_same_launcher
```

Run:

```bash
bun cargo test -p koharu-app pipeline_guard_
bun cargo test -p koharu-rpc pipeline_guard_
```

Expected: FAIL，当前 ProjectSession 没有 pipeline guard，HTTP 与 MCP 各自启动任务。

**Step 2: 实现一个共享单写入边界**

- `ProjectSession` 增加一个 `AtomicBool`；`try_acquire_pipeline_write(self: &Arc<Self>)` 使用 `compare_exchange` 返回持有 session Arc 的小型 RAII guard，Drop 时释放。
- 不增加 lock registry、第二个 Jobs store 或持久队列。
- 把 `routes/pipelines.rs` 现有创建 Job、cancel、sink 和 spawn 的逻辑收敛为一个 `pub(crate) start_pipeline_job()`；HTTP 与 MCP 都调用它。
- shared launcher 在 spawn 前获取 session guard；冲突映射为 HTTP 409，MCP 返回明确 invalid request。
- guard 移入 spawned task，成功、失败、panic unwind 和取消退出时都由 Drop 释放；测试不加载模型。
- CLI 每次只持有一个 `ProjectSession` 且跨进程已由 `.lock` 拒绝第二个 opener，不新增 CLI guard 接线。

**Step 3: 复用现有 warning 做失败页重试**

第一版只实现：

- 同一 ProjectSession 同时最多一个写入 pipeline；第二个请求返回 409。
- 复用 Task 5 已加入 `JobWarningEvent.pageId`，不再次修改事件 schema。
- Warning/SSE 提供 Task 9 重试所需的稳定失败 PageIds；本 Task 不实现 UI。
- 继续使用现有 whole-project/page scope、Jobs registry、SSE 和 cancel。

只有进程重启丢任务成为实际生产瓶颈时，再设计持久队列。

**Step 4: 验证**

```bash
bun cargo test -p koharu-app pipeline_guard_
bun cargo test -p koharu-rpc pipeline_guard_
bun cargo test -p koharu-integration-tests --test pipelines
bun cargo check -p koharu-app -p koharu-rpc --all-targets
```

Expected: PASS；HTTP/MCP 共享 guard，完成和取消后可再次启动，warning 保留稳定 PageId，scope 外页面不变。

**Commit boundary:** `fix(pipeline): serialize project writes and expose failed page ids`

### Task 8：向后兼容的电商图片导出

**Files:**

- Modify: `crates/koharu-rpc/src/routes/projects.rs:225-430`
- Modify: `crates/koharu-rpc/src/psd_export.rs`
- Test: `tests/integration-tests/tests/ecommerce_export.rs`
- Generated: `ui/openapi.json`, `ui/lib/api/schemas/*`

**Step 1: 写格式和边界失败测试**

覆盖：旧请求仍导出 PNG；JPEG 固定白色 alpha 背景；JPEG quality 只允许 1..=100；WebP 使用现有 lossless encoder；输出尺寸不变；PSD/KHR 带 image options 时明确拒绝。

Run:

```bash
bun cargo test -p koharu-integration-tests --test ecommerce_export
```

Expected: FAIL，测试文件和新选项尚不存在。

**Step 2: 增加最小导出选项**

在现有 `ExportProjectRequest` 增加可选：

```text
imageFormat: png | jpeg | webp
quality: 1..100
```

- 不增加 `fileNameTemplate`；继续复用当前稳定的 `page-NNN-id` 命名。
- `quality` 只允许 JPEG 使用；WebP 复用 `image` crate 的 lossless encoder，不增加依赖。
- JPEG alpha 固定合成到白色背景；WebP/PNG 保持页面尺寸。
- PSD/KHR 未携带 image options 时完全保持当前行为；携带时返回 400，避免静默忽略无效输入。

**Step 3: 验证与生成物**

```bash
bun cargo test -p koharu-integration-tests --test ecommerce_export
bun cargo test -p koharu-rpc --test openapi
bun cargo check -p koharu-rpc --all-targets
bun run generate:api
```

Expected: PASS；旧导出请求仍产生 PNG，检查并仅暂存预期 schema/client diff，最终门禁再运行 `check:generated`。

**Commit boundary:** `feat(export): add jpeg and webp image options`

### Task 9：补全电商 UI 操作闭环

**Files:**

- Create: `ui/lib/io/pipeline.ts`
- Create: `ui/components/export/ExportDialog.tsx`
- Create: `ui/components/operations/FailedPagesActions.tsx`
- Modify: `ui/components/MenuBar.tsx`
- Modify: `ui/components/canvas/CanvasToolbar.tsx`
- Modify: `ui/components/panels/TextBlocksPanel.tsx`
- Modify: `ui/components/ActivityBubble.tsx`
- Modify: `ui/lib/stores/jobsStore.ts`
- Modify: `ui/lib/io/pagesIo.ts`
- Create: `ui/tests/lib/io/pipeline.test.ts`
- Create: `ui/tests/components/CanvasToolbar.test.tsx`
- Create: `ui/tests/components/ExportDialog.test.tsx`
- Modify: `ui/tests/components/MenuBar.test.tsx`
- Modify: `ui/tests/components/TextBlocksPanel.test.tsx`
- Modify: `ui/tests/components/ActivityBubble.test.tsx`
- Modify: `ui/tests/lib/io/pagesIo.test.ts`
- Locale: `ui/public/locales/*/translation.json`

**Step 1: 写统一 pipeline 启动失败测试**

测试名称：

```text
project_target_language_is_used_when_toolbar_has_no_explicit_selection
explicit_toolbar_language_overrides_project_default
pipeline_start_error_uses_existing_activity_error
accepted_pipeline_request_is_remembered_by_operation_id
remembered_request_survives_started_and_snapshot_merge
dismissed_job_is_not_restored_by_snapshot
```

Run:

```bash
bun run --filter ui test -- tests/lib/io/pipeline.test.ts
```

Expected: FAIL，当前三个入口分别直接调用 `startPipeline()`，项目默认语言、即时 409/网络错误和原始请求没有统一处理。

**Step 2: 实现一个共享 UI 启动函数**

在 `ui/lib/io/pipeline.ts` 只实现两个导出：

```typescript
export function currentPipelineTargetLanguage(): string | undefined

export async function startPipelineFromUi(
  request: StartPipelineRequest,
): Promise<StartPipelineResponse | undefined>
```

- 目标语言优先级固定为：请求已显式携带的 `targetLanguage`（用于精确 retry）→ `editorUiStore.selectedLanguage` → 当前 Scene 的 `project.ecommerce.targetLanguage` → `undefined`。三个普通入口不再自己填该字段。
- 通过现有 React Query scene cache 读取项目配置，复用 `pagesIo.ts` 已有的 cache 读取模式；不把 Scene 镜像到 Zustand。
- `startPipelineFromUi()` 只包装现有 API：补入最终目标语言，成功后按 operationId 把完整请求记录到现有 `jobsStore`，失败时调用现有 `editorUiStore.showError()` 并返回 `undefined`。
- MenuBar、CanvasToolbar、TextBlocksPanel 三个入口全部改用该函数；不再各自拼目标语言和错误处理。
- MenuBar pipeline items、CanvasToolbar workflow buttons 和 TextBlocks Generate 在任一 pipeline running 时禁用；后端 409 仍作为竞态兜底显示在 ActivityBubble。
- 不新增 pipeline store、toast 系统或第二个 API facade。

Run:

```bash
bun run --filter ui test -- tests/lib/io/pipeline.test.ts \
  tests/components/MenuBar.test.tsx \
  tests/components/CanvasToolbar.test.tsx \
  tests/components/TextBlocksPanel.test.tsx
```

Expected: PASS；三个入口发送相同的目标语言优先级和错误语义。

**Step 3: 写质量定位、保留和重试失败测试**

测试名称：

```text
completed_with_errors_remains_visible_until_dismissed
quality_warning_navigates_to_page_and_optional_node
retry_reuses_original_request_with_failed_page_ids_only
retry_is_disabled_when_request_was_lost_after_restart
cancel_button_disables_while_cancel_request_is_pending
```

Run:

```bash
bun run --filter ui test -- tests/components/ActivityBubble.test.tsx
```

Expected: FAIL，当前 ActivityBubble 只渲染 running jobs，完成带错误后 warning 会立即消失，Jobs store 也没有请求元数据或 dismiss。

**Step 4: 复用 Jobs/SSE 完成检查与恢复闭环**

- `JobEntry` 增加仅 UI 会话使用的 `request?: StartPipelineRequest`；`jobsStore` 增加 `rememberRequest(id, request)`、`dismiss(id)` 和本会话 `dismissedIds`，不持久化、不复制后端 Job registry。
- `started()` 和 `setSnapshot()` 合并同 id 的现有 request/warnings，不能因 SSE 到达顺序或短线重连覆盖本地请求；浏览器/应用重启后 request 丢失属于第一版明确限制。
- `dismiss()` 删除卡片并记录 id，后续 SSE Snapshot 跳过已 dismiss id，避免已关闭警告在短线重连后重新出现；`clear()` 同时清空 dismissedIds。
- ActivityBubble 同时显示 running、failed 和 completed_with_errors，直到用户 dismiss；普通 completed/cancelled 不保留卡片。
- warning 行使用 Task 5 的 `code/pageId/nodeId`，点击时调用现有 `selectionStore.setPage(pageId)`，存在 nodeId 时再 `selectMany([nodeId])`。
- `FailedPagesActions` 从 warnings 去重有效 PageId；Retry 克隆原请求，只覆盖 `pages`，保留 steps、textNodeIds、targetLanguage、readingOrder、font 和 prompt。成功启动后 dismiss 旧卡片。
- 同一 UI 会话的 SSE 短线重连通过 merge 保留 request/warnings；应用重启后若只剩 JobSummary，则只显示摘要和 dismiss，定位与 Retry disabled 并显示原因。不建立持久任务数据库。
- Cancel 第一次点击后局部禁用，等待现有 cancel 请求完成；不增加 cancelling 后端状态。

Run:

```bash
bun run --filter ui test -- tests/components/ActivityBubble.test.tsx
```

Expected: PASS；失败结果不会瞬间消失，定位和重试均有明确可见结果。

**Step 5: 写导出 UI 失败测试**

覆盖：当前页/全部 Rendered 与 Inpainted 打开同一个对话框；默认 PNG；JPEG 显示且校验 1..=100 quality；PNG/WebP 隐藏 quality；提交时请求字段正确；失败保持弹窗并显示 ActivityBubble error；成功才关闭。PSD/KHR 继续走原直接导出入口。

Run:

```bash
bun run --filter ui test -- tests/components/ExportDialog.test.tsx \
  tests/components/MenuBar.test.tsx \
  tests/lib/io/pagesIo.test.ts
```

Expected: FAIL，当前 MenuBar 直接导出 PNG，用户无法选择新格式或 quality，错误也只写 console。

**Step 6: 实现最小导出对话框**

- `ExportDialog` 只接收 `role: rendered | inpainted`、可选 pages、open 和 onOpenChange；复用现有 Dialog、Select、Input、Button。
- `imageFormat` 默认 PNG；选择 JPEG 时显示数字 quality，默认 90，客户端先验证整数 1..=100；WebP 明确标记 lossless。
- `pagesIo.exportCurrentProjectAs()` 增加可选 `{ imageFormat, quality }`，成功返回 `true`，失败调用现有 `editorUiStore.showError()` 并返回 `false`，不再只 `console.error` 后抛出。
- MenuBar 现有三个 Rendered/Inpainted 项只负责设置 role/pages 并打开同一个 Dialog；PSD/KHR 保持直接导出。
- 提交中禁用关闭和重复提交；成功保存文件后关闭，失败保持输入供修正。

**Step 7: 国际化与可访问性**

- 九个现有 locale 同步增加电商设置、角色、质量 code、重试、运行冲突、导出格式、lossless 和 quality 文案；不只更新中英文。
- 导出 Dialog 必须有 title/description、可关联 label、初始焦点和 Escape 行为；warning、Retry、Dismiss、Cancel 使用原生 Button，可键盘访问并提供 aria-label。
- 错误只显示安全后端消息，不显示 API key 或 OCR 正文。

**Step 8: 完整 UI 回归**

```bash
bun run --filter ui test -- tests/lib/io/pipeline.test.ts \
  tests/components/MenuBar.test.tsx \
  tests/components/CanvasToolbar.test.tsx \
  tests/components/TextBlocksPanel.test.tsx \
  tests/components/ActivityBubble.test.tsx \
  tests/components/ExportDialog.test.tsx \
  tests/lib/io/pagesIo.test.ts
bun run format:check
bun run lint:ui
```

Expected: PASS；设置 → 执行 → 状态 → 定位 → 重试 → 导出形成完整可见闭环，没有新增 store、通知系统或 API 客户端。

**Commit boundary:**

```bash
git add ui/lib/io/pipeline.ts ui/lib/io/pagesIo.ts ui/lib/stores/jobsStore.ts \
  ui/components/MenuBar.tsx ui/components/canvas/CanvasToolbar.tsx \
  ui/components/panels/TextBlocksPanel.tsx ui/components/ActivityBubble.tsx \
  ui/components/export/ExportDialog.tsx \
  ui/components/operations/FailedPagesActions.tsx \
  ui/tests/lib/io/pipeline.test.ts ui/tests/lib/io/pagesIo.test.ts \
  ui/tests/components/MenuBar.test.tsx \
  ui/tests/components/CanvasToolbar.test.tsx \
  ui/tests/components/TextBlocksPanel.test.tsx \
  ui/tests/components/ActivityBubble.test.tsx \
  ui/tests/components/ExportDialog.test.tsx \
  ui/public/locales/{en-US,es-ES,ja-JP,ko-KR,pt-BR,ru-RU,tr-TR,zh-CN,zh-TW}/translation.json
git commit -m "feat(ui): complete ecommerce operation workflow"
```

### Task 10：选择并规划第一条远程 OCR Provider

本 Task 是停止式研究门，不修改 production 文件，也不创建 `remote/` 目录、`ProviderTarget`、`engine_providers` map 或 `EngineCtx` secret 字段。

**Files:**

- Read: `crates/koharu-app/src/config.rs:59-133`
- Read: `crates/koharu-app/src/pipeline/engine.rs:41-95`
- Read: `crates/koharu-app/src/llm.rs`
- Read: `crates/koharu-rpc/src/routes/config.rs`
- Read: `crates/koharu-secrets/src/*`
- Verify: `$KOHARU_ECOMMERCE_BENCHMARK_DIR/manifest.json`

**Step 1: 建立具体 API 候选门禁**

候选必须同时满足：返回 text、confidence、真实 line/word polygon、direction/rotation；允许受控样本；密钥可放入现有 keychain；许可、价格和数据保留策略可接受。缺任一项立即淘汰，不写适配器。

**Step 2: 在同一 50 图 manifest 上运行候选**

将候选输出转换成 Task 4 测试文件已经使用的内存 metric DTO，只把聚合指标写入本地 `results/remote-ocr.json`，不得把商家图片、OCR 正文或 API key 写入仓库和日志。

Expected: 至少一个具体 API 达到 Task 4 的四项阈值；否则本路线图在远程 OCR 能力处标记 BLOCKED，不创建 production skeleton。

**Step 3: 为胜出 API 单独写可执行 child plan**

子计划必须先重新读取胜出 API 的官方文档，并包含确切 Provider 名称、请求/响应 DTO、超时/重试、配置字段、keychain key、Engine id、测试和生成物。第一条远程 OCR 使用具体字段和现有 `ProviderConfig`；出现第二个真实远程 Engine 后，才评估提取按 Engine 的 target map 或共享 HTTP/auth helper。

子计划必须包含 SettingsDialog 中该具体 OCR Engine 的 provider/model 选择、连通性错误和本地/远程状态展示，复用现有 providers 设置与 keychain，不提前做通用 Engine 配置 UI。还必须验证：缺配置在网络前失败、API key 不进入 JSON/Debug/事件/Scene、HTTP/MCP/CLI 读取同一服务端配置、`StartPipelineRequest` 不变化、本地 Engine 不读取远程密钥。默认测试使用内存响应，不访问网络。

**Commit boundary:** 本研究门无 production commit；只有具体 Provider 子计划审查通过后才实施和提交。

### Task 11：文档、全量门禁与真实电商验收

**Files:**

- Modify: `docs/zh-CN/project-functional-analysis.md`
- Modify: `docs/zh-CN/explanation/how-koharu-works.md`
- Modify: `docs/zh-CN/reference/settings.md`
- Verify: local ecommerce benchmark directory

**Step 1: 文档同步**

记录 TextRole、保护词、术语表、真实几何限制、quality warnings、批量重试、导出选项和 UI 操作入口。只有 Task 10 的具体 Provider 子计划已经实施时才记录远程 Engine；否则明确标记为尚未提供，不能把研究门写成现有功能。不具备逐行几何的混合节点仍安全跳过。

**Step 2: Rust/UI/生成物门禁**

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
git diff --check
```

Apple Silicon 额外：

```bash
bun cargo check -p koharu --all-targets --features=metal
bun cargo build --release -p koharu --features=metal
```

**Step 3: 真实图片停止条件**

每个样本从 Source 完整重跑，先清理旧 translation、sprite、sprite_transform、Segment、Inpainted 和 Rendered 派生物。mask 外像素验收使用运行前输入基面与各 Inpainter 生产路径最终有效 mask；不得用 page-end `inspect_page()` 的推断替代。

- 50 张基准中文检测召回 ≥ 90%。
- 有效混合节点逐行 polygon 覆盖 ≥ 95%，无几何节点全部安全提示。
- 保护品牌/英文误改率 0；mask 外像素差异 0。
- 数字、价格、百分比、SKU 和单位保持率 100%。
- 字体变更无需额外点击即可更新；列表浏览不增加字体缓存。
- 自动检查能标记全部预植入错误样本。
- 三个 pipeline UI 入口在未显式选择语言时都使用项目目标语言；运行冲突和网络错误在 ActivityBubble 可见。
- completed_with_errors 不会瞬间消失；可跳转到 PageId/NodeId，批量失败页可重试，成功页不重复修改。
- UI 可选择 PNG/JPEG/WebP 和 JPEG quality；PNG/JPEG/WebP/PSD/KHR 输出可打开且尺寸符合请求。
- Task 6/10 若未通过停止门，验收报告必须写 NOT APPLICABLE/BLOCKED，不得按已实现计入通过率。

**Commit boundary:** `docs: document ecommerce translation workflow`

## 6. 风险与缓解

| 风险 | 缓解 |
| --- | --- |
| 模块拆分引入行为回归 | 先跑现有测试，机械移动，独立提交，不混功能 |
| TextRole/项目配置改变旧 Postcard | Task 3 使用带 magic/version 的新 snapshot，并提供真实 V1 decoder；不修改旧 Patch 布局 |
| Provider 输出破坏品牌/数字 | strict parse 后、ops 前做原子后验验证 |
| 新 OCR 没有真实行框 | Task 4 基准门禁；不达标不接入 production |
| 质量检查误报阻塞旧流程 | 仅 ecommerce.enabled 启用；旧项目默认关闭 |
| UI 文件继续膨胀 | 新设置/样式/操作放归属目录，父组件只接线 |
| 三个入口目标语言或错误语义漂移 | Task 9 统一使用一个 UI pipeline helper，不复制目标语言和错误处理 |
| 应用重启后无法重试旧请求 | 仍可定位和 dismiss；没有本地 request 时禁用 Retry，不伪造请求 |
| 远程密钥泄漏 | 复用 keychain；secret 不进入 Scene、Debug、事件或 request |
| 批量队列过度设计 | 第一版仅单写入 guard + PageId retry，不做持久队列 |
| 模型/测试占满磁盘 | 默认测试无模型；dev/test profile 关闭 incremental，验收后清 target |

## 7. 提交与执行边界

- 每个 Task 单独 PR/commit，不跨 Task 暂存文件。
- 先写失败测试并记录预期 FAIL，再做最小实现和 PASS。
- 生成物与 schema 只在对应 API Task 提交，之后再运行 `check:generated`。
- 不手写 CHANGELOG；继续使用现有 git-cliff 流程。
- 不提交本地 benchmark 图片、模型、字体缓存、`target/`、`.next/` 或 `.codegraph` 运行文件。
- 本规划不授权实施或提交；实施前应从 Task 0 开始，并保留当前无关 dirty state。

## 8. Ponytail/YAGNI 明确删除项

- 不新建 Ecommerce Engine trait；现有 `Engine` 已足够。
- 不新建第二套 Provider registry；复用现有 provider 配置和 keychain。
- 不新建第二套 job/event/store；复用 Jobs/SSE/ActivityBubble。
- 不新建 pipeline UI store、toast 系统或导出状态机；复用 jobsStore、editorUiStore.error 和一个 ExportDialog。
- 不新建几何 DTO；优先复用 `TextRegion`、`TextData.line_polygons` 和 `Transform`。
- 不新建质量 Artifact；第一版复用 page-end metadata inspection 与 warning status，像素不变量留在 Inpainter 生产测试。
- 不新建字体缓存管理器；停止列表自动下载即可。
- 不增加首版文件名模板、通用 Engine Provider map 或只包装一个控件的组件。
- 不做持久任务队列、渐变/曲线富文本和第四个 Inpainter，直到基准或真实吞吐证明需要。
