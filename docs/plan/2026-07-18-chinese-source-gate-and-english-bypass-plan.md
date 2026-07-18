# 中文源文字门禁与纯英文跳过 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** 增加可保存的“源文字”选择；选择中文时，纯英文不形成可见文本框、不进入完整 OCR 或任何后续阶段，中英混排只处理中文，`S型曲线` 等单字母中文短语整体处理。

**Architecture:** 复用现有 `SourceTextPolicy` 作为一期选择器（`han_only` = 中文，`all_text` = 兼容模式），不新增重复配置。新增一个内部 Source Gate：Detector 只产生不可见候选；Gate 先用现有 PP-OCRv5 词框拒绝明显纯英文，只有 PP-OCRv5 判定含 Han 的候选才在原始候选 crop 上调用 PaddleOCR-VL，并复用当前严格字符对齐规则生成最终中文几何与权威 OCR 文本。HanOnly 下 Gate 取代所选 OCR step，完成后才允许 Font、Segment、Translate、Typography、Inpaint 和 Render；零中文目标时由 driver 清理自动派生物并整页短路，AllText 仍运行用户选择的原有 OCR。

**Tech Stack:** Rust、Tauri、Tokio、PP-OCRv5/ocrs-cjk、PaddleOCR-VL、Petgraph artifact DAG、React/Next.js、TypeScript、Vitest/MSW、OpenAPI/Orval。

---

## 需求闭环

| 输入 | 中文模式行为 |
| --- | --- |
| `SLENDER WAIST`、`Peach Booty`、完整英文句子 | Detector 可产生内部候选；Source Gate 删除候选；不显示框、不调用完整 OCR、不运行后续阶段，Source 像素不变 |
| `蜜桃臀`、中文句子 | 保留中文候选，完整 OCR 后正常翻译、擦除、排版、渲染 |
| `Peach 蜜桃臀`、PP-OCRv5 可分成 `AI` + `智能塑形` 的输入 | 完整英文词/缩写保留，只把连续中文目标几何和权威中文 OCR 文本交给后续阶段 |
| `S型曲线`、`A版` | 单个拉丁字母不构成受保护英文词，整个短语作为中文目标 |
| `AI智能塑形` 被 PP-OCRv5 返回为一个不可分词框，或其他低置信度、非有限/越界、无法安全分离输入 | 安全跳过并保留 Source，不按比例猜测字符几何 |
| `all_text` | 保持现有所有文字框、节点级 OCR 和后续行为 |

严格说明：系统不可能在完全不读取候选内容时知道它是不是英文。PP-OCRv5 只做一次本地、临时的词框/脚本判定；没有 Han 的候选不会进入 PaddleOCR-VL。PP-OCRv5 判定含 Han 的候选必须再由 PaddleOCR-VL 对原始候选 crop 做字符对齐校验，校验失败不写入节点、不进入业务流程。

## 文件与职责规划

### 新建

- `crates/koharu-app/src/pipeline/engines/source_language_gate.rs`：PP-OCRv5 预筛、PaddleOCR-VL 权威校验、中文目标分类、候选节点更新/删除、零目标自动派生物清理。
- `ui/tests/components/SourceTextPolicySettings.test.tsx`：设置保存与 UI 选项回归。

### 修改

- `crates/koharu-core/src/protocol.rs`：把 `SourceTextPolicy` 放到共享协议层，并加入 `PipelineConfigPatch`。
- `crates/koharu-core/src/lib.rs`：导出 `SourceTextPolicy`。
- `crates/koharu-app/src/config.rs`：重导出策略、应用配置补丁并保留旧 TOML 默认值。
- `crates/koharu-app/src/pipeline/artifacts.rs`：增加 DAG-only `SourceTextBoxes` token。
- `crates/koharu-app/src/pipeline/engine.rs`：锁定 Detector → Source Gate → Font/Bubble/Segment 的相对顺序。
- `crates/koharu-app/src/pipeline/mod.rs`：HanOnly 自动注入 Gate、移除重复 OCR step，并在无中文目标时短路页面。
- `crates/koharu-app/src/pipeline/engines/mod.rs`：注册门禁模块。
- `crates/koharu-app/src/pipeline/engines/{pp_doclayout.rs,anime_text.rs,ctd_full.rs,comic_text_bubble.rs}`：中文模式候选节点默认不可见。
- `crates/koharu-app/src/pipeline/engines/paddle_ocr.rs`：把现有 PP/VL 字符对齐校验移动到 Gate，AllText 保持原节点级 OCR。
- `crates/koharu-app/src/pipeline/engines/{yuzumarker_font.rs,bubble_segmentation.rs}`：消费 Gate 完成 token；AllText 仍保留 Detector edge。
- `crates/koharu-app/src/pipeline/engines/support.rs`：让 `new_text_node()` 接收可见性；普通阶段过滤不可见候选/保护节点；Source 像素恢复读取受保护的英文行；复用现有 Han/完整 Latin 词规则。
- `ui/components/SettingsDialog.tsx`：增加“源文字”选择并保存 `sourceTextPolicy`。
- `ui/hooks/useCurrentPage.ts`：不向 UI 暴露 `visible: false` 的 provisional Text 节点。
- `ui/tests/hooks/useCurrentPage.test.tsx`：不可见候选过滤测试。
- `ui/public/locales/*/translation.json`：源文字设置文案。
- `ui/openapi.json`、`ui/lib/api/schemas/pipelineConfigPatch.ts`：Task 1 同步生成并提交的 API 产物。
- `docs/zh-CN/reference/settings.md`：中文模式行为和限制。

### 明确不新增

- 不新增依赖、Provider、语言识别云服务、第二套 `source_language` 配置、全局测试 hook 或单实现 trait。
- 一期不承诺区分“纯汉字日文”和中文；如果以后必须严格区分，再增加经过验收的语言分类模型。

### 执行前状态保护

- 先运行 `git status --short` 保存基线；当前已有改动均视为用户内容。
- 每个提交前运行 `git diff -- <本 Task 文件>`；基线已修改的文件使用 `git add -p` 只暂存本 Task hunk，禁止 reset、checkout 或覆盖无关改动。
- 计划中的 `git add` 列表是允许范围，不代表可以暂存文件内既有无关 hunk。

---

### Task 1: 打通可保存的“源文字”选择

**Files:**
- Modify: `crates/koharu-core/src/protocol.rs`
- Modify: `crates/koharu-core/src/lib.rs`
- Modify: `crates/koharu-app/src/config.rs`
- Modify: `ui/components/SettingsDialog.tsx`
- Create: `ui/tests/components/SourceTextPolicySettings.test.tsx`
- Modify generated: `ui/openapi.json`
- Modify generated: `ui/lib/api/schemas/pipelineConfigPatch.ts`
- Modify generated if export order changes: `ui/lib/api/schemas/index.ts`
- Modify: `ui/public/locales/en-US/translation.json`
- Modify: `ui/public/locales/zh-CN/translation.json`
- Modify: `ui/public/locales/{es-ES,ja-JP,ko-KR,pt-BR,ru-RU,tr-TR,zh-TW}/translation.json`

- [ ] **Step 1: 写 Rust 配置补丁失败测试**

在 `crates/koharu-app/src/config.rs` 测试模块增加：

```rust
#[test]
fn config_patch_updates_source_text_policy_without_changing_engines() {
    let mut config = AppConfig::default();
    let detector = config.pipeline.detector.clone();

    apply_patch(
        &mut config,
        ConfigPatch {
            pipeline: Some(PipelineConfigPatch {
                source_text_policy: Some(SourceTextPolicy::AllText),
                ..Default::default()
            }),
            ..Default::default()
        },
    );

    assert_eq!(config.pipeline.source_text_policy, SourceTextPolicy::AllText);
    assert_eq!(config.pipeline.detector, detector);
}
```

- [ ] **Step 2: 写 UI 保存失败测试**

在 `ui/tests/components/SourceTextPolicySettings.test.tsx` 通过 MSW 返回带完整 engine catalog 的 `AppConfig`，打开 `SettingsDialog` 的 `engines` tab，选择“全部文字”，断言请求体严格为：

```ts
expect(patches.at(-1)?.pipeline?.sourceTextPolicy).toBe('all_text')
expect(patches.at(-1)?.pipeline?.detector).toBe('pp-doclayout-v3')
```

再把服务端响应切回 `han_only`，重新渲染并断言选择器显示“中文（推荐）”。

- [ ] **Step 3: 运行测试并确认 FAIL**

Run:

```bash
bun cargo test -p koharu-app config_patch_updates_source_text_policy_without_changing_engines --lib
bun run --filter ui test -- tests/components/SourceTextPolicySettings.test.tsx
```

Expected: Rust 因 `PipelineConfigPatch::source_text_policy` 不存在而编译失败；UI 因缺少选择器或 PATCH 字段失败。

- [ ] **Step 4: 把策略类型移动到共享协议层并支持 PATCH**

在 `crates/koharu-core/src/protocol.rs` 定义共享类型并加入补丁：

```rust
#[derive(
    Debug, Copy, Clone, Default, PartialEq, Eq, Serialize, Deserialize, JsonSchema, ToSchema,
)]
#[serde(rename_all = "snake_case")]
pub enum SourceTextPolicy {
    #[default]
    HanOnly,
    AllText,
}

#[derive(Debug, Clone, Default, Serialize, Deserialize, JsonSchema, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct PipelineConfigPatch {
    pub source_text_policy: Option<SourceTextPolicy>,
    pub detector: Option<String>,
    pub font_detector: Option<String>,
    pub segmenter: Option<String>,
    pub bubble_segmenter: Option<String>,
    pub ocr: Option<String>,
    pub translator: Option<String>,
    pub typography_planner: Option<String>,
    pub inpainter: Option<String>,
    pub renderer: Option<String>,
}
```

从 `crates/koharu-app/src/config.rs` 删除重复枚举并重导出：

```rust
pub use koharu_core::SourceTextPolicy;
```

在 `apply_patch()` 的 pipeline 分支最前面应用：

```rust
if let Some(v) = p.source_text_policy {
    config.pipeline.source_text_policy = v;
}
```

- [ ] **Step 5: 直接在现有 EnginesPane 增加选择器**

`appConfigToPatch()` 的 pipeline 对象加入：

```ts
sourceTextPolicy: cfg.pipeline.source_text_policy ?? 'han_only',
```

在 `EnginesPane` 的引擎列表前增加，不创建新 helper 文件：

```tsx
<div className='space-y-1.5'>
  <Label className='text-xs'>{t('settings.sourceText')}</Label>
  <Select
    value={pipeline.source_text_policy ?? 'han_only'}
    onValueChange={(value) =>
      onChange({
        ...pipeline,
        source_text_policy: value as import('@/lib/api/schemas').SourceTextPolicy,
      })
    }
  >
    <SelectTrigger data-testid='source-text-policy' className='w-full'>
      <SelectValue />
    </SelectTrigger>
    <SelectContent>
      <SelectItem value='han_only'>{t('settings.sourceTextChinese')}</SelectItem>
      <SelectItem value='all_text'>{t('settings.sourceTextAll')}</SelectItem>
    </SelectContent>
  </Select>
  <p className='text-xs text-muted-foreground'>{t('settings.sourceTextDescription')}</p>
</div>
```

文案含义固定为：中文模式只将中文目标交给完整 OCR 和后续阶段；全部文字保持兼容行为。

- [ ] **Step 6: 立即生成 API 类型并检查唯一预期差异**

```bash
bun run generate:api
git diff -- ui/openapi.json ui/lib/api/schemas/pipelineConfigPatch.ts ui/lib/api/schemas/index.ts
```

Expected: `PipelineConfigPatch` 只增加 `sourceTextPolicy?: SourceTextPolicy | null` 及必要 import；`StartPipelineRequest`、HTTP 路径和其他请求 JSON 不变。

- [ ] **Step 7: 运行定向回归并确认 PASS**

```bash
bun cargo test -p koharu-app config::tests --lib
bun run --filter ui test -- tests/components/SourceTextPolicySettings.test.tsx
bun run --filter ui build
```

Expected: PASS；旧 TOML 仍默认 `han_only`，`all_text` 可往返，Task 1 提交点可独立完成 TypeScript 构建。

- [ ] **Step 8: 提交 Task 1**

```bash
git add crates/koharu-core/src/protocol.rs crates/koharu-core/src/lib.rs crates/koharu-app/src/config.rs ui/components/SettingsDialog.tsx ui/tests/components/SourceTextPolicySettings.test.tsx ui/public/locales ui/openapi.json ui/lib/api/schemas/pipelineConfigPatch.ts ui/lib/api/schemas/index.ts
git commit -m "feat: expose source text policy setting"
```

---

### Task 2: 实现 PP 预筛 + VL 权威校验的中文目标决策

**Files:**
- Create: `crates/koharu-app/src/pipeline/engines/source_language_gate.rs`
- Modify: `crates/koharu-app/src/pipeline/engines/mod.rs`
- Move logic from: `crates/koharu-app/src/pipeline/engines/paddle_ocr.rs`
- Reuse: `crates/koharu-app/src/pipeline/engines/support.rs`
- Reuse: `crates/koharu-ml/src/pp_ocr_v5.rs`

- [ ] **Step 1: 写纯函数失败测试**

在新模块增加以下内存测试；`word()` 的 `top/bottom` 参数用于验证多行几何：

```rust
fn word(
    text: &str,
    line_index: usize,
    left: f32,
    top: f32,
    right: f32,
    bottom: f32,
) -> PpOcrWordBox {
    PpOcrWordBox {
        line_index,
        text: text.into(),
        bbox: [left, top, right, bottom],
        confidence: 0.9,
    }
}

#[test]
fn gate_pp_prefilter_rejects_pure_english_without_vl() {
    let words = [
        word("SLENDER", 0, 0.0, 0.0, 40.0, 20.0),
        word("WAIST", 0, 45.0, 0.0, 80.0, 20.0),
    ];
    assert!(!pp_may_contain_han(&words));
}

#[test]
fn gate_vl_validation_keeps_same_line_single_label_and_excludes_other_lines() {
    let same_line = select_chinese_target(
        "S型曲线",
        &[
            word("S", 0, 0.0, 0.0, 8.0, 20.0),
            word("型曲线", 0, 8.0, 0.0, 40.0, 20.0),
        ],
        [10, 20, 110, 70],
        200,
        100,
    )
    .unwrap();
    assert_eq!(same_line.text, "S型曲线");
    assert_eq!(same_line.bbox, [10.0, 20.0, 50.0, 40.0]);

    let other_line = select_chinese_target(
        "S\n中文",
        &[
            word("S", 0, 0.0, 0.0, 8.0, 10.0),
            word("中文", 1, 10.0, 15.0, 40.0, 35.0),
        ],
        [10, 20, 110, 70],
        200,
        100,
    )
    .unwrap();
    assert_eq!(other_line.text, "中文");
    assert_eq!(other_line.bbox, [20.0, 35.0, 50.0, 55.0]);
    assert_eq!(other_line.protected_lines, vec![("S".into(), [10.0, 20.0, 18.0, 30.0])]);
}

#[test]
fn gate_vl_validation_keeps_only_han_beside_complete_english() {
    let target = select_chinese_target(
        "Peach蜜桃臀",
        &[
            word("Peach", 0, 0.0, 0.0, 40.0, 20.0),
            word("蜜桃臀", 0, 45.0, 0.0, 100.0, 20.0),
        ],
        [10, 20, 110, 70],
        200,
        100,
    )
    .unwrap();
    assert_eq!(target.text, "蜜桃臀");
    assert_eq!(target.bbox, [55.0, 20.0, 110.0, 40.0]);
    assert_eq!(target.protected_lines, vec![("Peach".into(), [10.0, 20.0, 50.0, 40.0])]);
}

#[test]
fn gate_vl_validation_rejects_mismatch_unseparated_and_invalid_geometry() {
    assert!(select_chinese_target(
        "Peach蜜桃臀",
        &[
            word("Beach", 0, 0.0, 0.0, 40.0, 20.0),
            word("蜜桃臀", 0, 45.0, 0.0, 100.0, 20.0),
        ],
        [0, 0, 100, 50],
        100,
        50,
    )
    .is_none());
    assert!(select_chinese_target(
        "AI智能塑形",
        &[word("AI智能塑形", 0, 0.0, 0.0, 100.0, 20.0)],
        [0, 0, 100, 50],
        100,
        50,
    )
    .is_none());
}
```

- [ ] **Step 2: 运行测试并确认 FAIL**

```bash
bun cargo test -p koharu-app pipeline::engines::source_language_gate::tests --lib
```

Expected: FAIL，因为模块、PP 预筛和带 VL 正文的严格分类函数尚不存在。

- [ ] **Step 3: 移动并收紧现有严格校验**

从 `paddle_ocr.rs::build_pp_ocr_word_box_update()` 移动字符对齐、置信度、坐标和 overlap 校验，不复制。生产决策类型固定为：

```rust
const MIN_WORD_CONFIDENCE: f32 = 0.5;

#[derive(Clone, Debug, PartialEq)]
struct SourceTarget {
    text: String,
    bbox: [f32; 4],
    line_polygons: Vec<[[f32; 2]; 4]>,
    protected_lines: Vec<(String, [f32; 4])>,
}

fn pp_may_contain_han(words: &[PpOcrWordBox]) -> bool {
    words.iter().any(|word| contains_han(&word.text))
}

fn bbox_quad([left, top, right, bottom]: [f32; 4]) -> [[f32; 2]; 4] {
    [[left, top], [right, top], [right, bottom], [left, bottom]]
}
```

`select_chinese_target(vl_text, words, crop_bounds, image_width, image_height)` 必须按以下固定算法实现：

1. 复用当前 `vl_chars`/`vl_offset` 顺序对齐；PP 与 VL 的 Latin 字符必须完全相同，Han 只允许 Han→Han 对齐；未消费完整 VL 正文时失败。
2. 验证 confidence、有限坐标、crop 内非退化 bbox、line_index 非递减及同一行 x 坐标非递减。
3. 每个 PP item 使用 VL 对齐后的权威文本计算 `contains_han` 与 `contains_protected_latin_word`；同一 item 同时含两者时失败，不按字符比例切框。
4. 只遍历“至少一个 item 含 Han”的 line；完全不含 Han 的其他 line 永不进入目标。
5. Han line 没有 protected item 时选择该行全部连续 item，因此同排 `S` + `型曲线` 合并；有 protected item 时，以 protected item 为分隔符，只允许唯一一个包含 Han 的连续非 protected run。
6. 每个目标行生成一个绝对坐标 quad；节点 bbox 是这些 quad 的 union。任一目标 quad 与被排除 item 的 bbox 相交时失败。
7. 所有被排除的权威非 Han item 按行合并为 `protected_lines`；它们只用于最后从 Source 恢复像素，不进入普通 `text_nodes()`。
8. `SourceTarget.text` 按 line_index 顺序以 `\n` 连接；`line_polygons.len()` 必须等于非空文本行数。

不要新增第二个 validator、trait 或通用 geometry 模块。`intersect_bbox()` 和 bbox union 保持为该文件私有小函数。

- [ ] **Step 4: 运行测试并确认 PASS**

```bash
bun cargo test -p koharu-app pipeline::engines::source_language_gate::tests --lib
bun cargo test -p koharu-app pipeline::engines::paddle_ocr::tests --lib
```

Expected: 新测试 PASS；原 `paddle_ocr` 校验测试移动后由 Gate 测试覆盖，AllText 的节点级 OCR 测试仍 PASS；不加载模型。

- [ ] **Step 5: 提交 Task 2**

```bash
git add crates/koharu-app/src/pipeline/engines/source_language_gate.rs crates/koharu-app/src/pipeline/engines/paddle_ocr.rs crates/koharu-app/src/pipeline/engines/mod.rs
git commit -m "feat: validate Chinese source targets"
```

---

### Task 3: 接入真实生产门禁并隐藏 provisional 英文框

**Files:**
- Modify: `crates/koharu-app/src/pipeline/engines/source_language_gate.rs`
- Modify: `crates/koharu-app/src/pipeline/engines/support.rs`
- Modify: `crates/koharu-app/src/pipeline/engines/pp_doclayout.rs`
- Modify: `crates/koharu-app/src/pipeline/engines/anime_text.rs`
- Modify: `crates/koharu-app/src/pipeline/engines/ctd_full.rs`
- Modify: `crates/koharu-app/src/pipeline/engines/comic_text_bubble.rs`
- Modify: `ui/hooks/useCurrentPage.ts`
- Modify: `ui/tests/hooks/useCurrentPage.test.tsx`

- [ ] **Step 1: 写生产 dispatch 失败测试**

在 `source_language_gate.rs` 增加不加载模型的双 closure seam 测试；PP closure 记录所有候选，VL closure 只接收 PP 判定含 Han 的 crop：

```rust
#[tokio::test]
async fn production_gate_removes_english_and_keeps_only_chinese() {
    let fixture = gate_fixture_with_english_and_mixed_nodes();
    let vl_calls = AtomicUsize::new(0);
    let ops = dispatch_source_gate(
        &fixture.image,
        &fixture.scene,
        fixture.page,
        |node_id, _crop| Ok(fixture.word_boxes(node_id)),
        |crops| async {
            vl_calls.fetch_add(crops.len(), Ordering::Relaxed);
            Ok(vec!["Peach蜜桃臀".to_string()])
        },
    )
    .await
    .unwrap();
    let scene = apply_ops(fixture.scene, ops);
    assert_eq!(vl_calls.load(Ordering::Relaxed), 1);
    assert!(scene.node(fixture.page, fixture.english).is_none());
    let mixed = text(&scene, fixture.page, fixture.mixed);
    assert_eq!(mixed.text.as_deref(), Some("蜜桃臀"));
    assert_eq!(mixed.line_polygons.as_ref().unwrap().len(), 1);
    assert!(scene.node(fixture.page, fixture.mixed).unwrap().visible);
    let protected = protected_source_lines_for_page(&scene, fixture.page);
    assert_eq!(protected.len(), 1);
    assert_eq!(protected[0].1.text, "Peach");
    assert_eq!(text_nodes(&scene, fixture.page).len(), 1);
}

#[tokio::test]
async fn production_gate_empty_targets_preserves_repair_brush_and_its_inpainted_result() {
    let fixture = gate_fixture_with_brush_and_inpainted();
    let ops = dispatch_source_gate(
        &fixture.image,
        &fixture.scene,
        fixture.page,
        |_node_id, _crop| Ok(vec![word("English", 0, 0.0, 0.0, 40.0, 20.0)]),
        |_crops| async { panic!("pure English must not call or load PaddleOCR-VL") },
    )
    .await
    .unwrap();
    let scene = apply_ops(fixture.scene, ops);
    assert!(find_image_node(&scene, fixture.page, ImageRole::Source).is_some());
    assert!(find_mask_node(&scene, fixture.page, MaskRole::BrushInpaint).is_some());
    assert!(find_image_node(&scene, fixture.page, ImageRole::Inpainted).is_some());
    assert!(find_image_node(&scene, fixture.page, ImageRole::Rendered).is_none());
    assert!(find_mask_node(&scene, fixture.page, MaskRole::Segment).is_none());
    assert!(find_mask_node(&scene, fixture.page, MaskRole::Bubble).is_none());
}
```

`gate_fixture_*()`、`apply_ops()` 和 `text()` 都定义在同一 `#[cfg(test)]` 模块，仅构造内存 Scene/图片；不得增加 production fixture 或全局 hook。

- [ ] **Step 2: 写 UI provisional 隐藏失败测试**

在 `ui/tests/hooks/useCurrentPage.test.tsx`：

```tsx
it('omits invisible provisional text nodes', () => {
  const page = samplePage()
  page.nodes.t2.visible = false
  expect(textNodesOf(page).map((node) => node.id)).toEqual(['t1'])
})
```

- [ ] **Step 3: 运行测试并确认 FAIL**

```bash
bun cargo test -p koharu-app production_gate_ --lib
bun run --filter ui test -- tests/hooks/useCurrentPage.test.tsx
```

Expected: Rust dispatch/清理不存在；UI 仍返回 invisible 节点。

- [ ] **Step 4: 实现生产门禁 seam**

生产与测试必须共用以下唯一入口：

```rust
async fn dispatch_source_gate<WordBoxes, Validate, Fut>(
    image: &DynamicImage,
    scene: &Scene,
    page: PageId,
    mut word_boxes: WordBoxes,
    mut validate: Validate,
) -> Result<Vec<Op>>
where
    WordBoxes: FnMut(NodeId, &DynamicImage) -> Result<Vec<PpOcrWordBox>>,
    Validate: FnMut(Vec<DynamicImage>) -> Fut,
    Fut: std::future::Future<Output = Result<Vec<String>>>,
{
    let candidates = source_gate_candidates(image, scene, page)?;
    let mut ops = Vec::new();
    let mut pending = Vec::new();
    let mut accepted = 0_usize;
    for candidate in candidates {
        let words = word_boxes(candidate.node_id, &candidate.crop)?;
        if pp_may_contain_han(&words) {
            pending.push((candidate, words));
        } else {
            ops.push(remove_node(scene, page, candidate.node_id)?);
        }
    }

    let vl_texts = if pending.is_empty() {
        Vec::new()
    } else {
        validate(
            pending
            .iter()
            .map(|(candidate, _)| candidate.crop.clone())
            .collect::<Vec<_>>(),
        )
        .await?
    };
    anyhow::ensure!(vl_texts.len() == pending.len(), "source gate OCR count mismatch");
    for ((candidate, words), vl_text) in pending.into_iter().zip(vl_texts) {
        match select_chinese_target(
            &vl_text,
            &words,
            candidate.crop_bounds,
            image.width(),
            image.height(),
        ) {
            Some(target) => {
                accepted += 1;
                ops.extend(update_target_ops(scene, page, candidate.node_id, target)?);
            }
            None => ops.push(remove_node(scene, page, candidate.node_id)?),
        }
    }
    if accepted == 0 {
        ops.extend(zero_target_cleanup(scene, page));
    }
    Ok(ops)
}
```

`source_gate_candidates()` 接受 detector 产生的 invisible 候选以及旧项目中已经存在的 visible Text 节点，但忽略 `detector == "pp-ocr-v5-source-gate-protected"` 的保护节点，并只保留有限、正面积且与 Source 相交的 bbox。复用 `page.nodes.iter().enumerate()` 构造可逆删除 op：

```rust
fn remove_node(scene: &Scene, page: PageId, id: NodeId) -> Result<Op> {
    let page_ref = scene.page(page).ok_or_else(|| anyhow::anyhow!("page not found"))?;
    let (prev_index, (_, prev_node)) = page_ref
        .nodes
        .iter()
        .enumerate()
        .find(|(_, (node_id, _))| **node_id == id)
        .ok_or_else(|| anyhow::anyhow!("node not found"))?;
    Ok(Op::RemoveNode {
        page,
        id,
        prev_node: prev_node.clone(),
        prev_index,
    })
}
```

`zero_target_cleanup()` 固定删除 `Rendered`、`Segment`、`Bubble` 和旧的 `pp-ocr-v5-source-gate-protected` metadata；只有页面不存在 `BrushInpaint` 时才删除 `Inpainted`。它使用同一个 `remove_node()`，不得新增 role-cleanup abstraction。Source、BrushInpaint 及存在 Brush 时的 Inpainted 永不删除。

`update_target_ops()` 更新原节点为 visible 中文目标，并为每个 `target.protected_lines` 添加一个 `visible:false`、`detector="pp-ocr-v5-source-gate-protected"` 的 Text 节点。保护节点只保存权威英文文本、绝对 bbox quad 和 Source 像素恢复所需 transform；translation、font、style、sprite 均为空。

同时修改 `support.rs::text_nodes()`，在匹配 `NodeKind::Text` 前过滤 `node.visible`，使所有普通 OCR/Font/Segment/Translate/Inpaint/Renderer 输入只看到中文目标。`protected_source_lines_for_page()` 改为直接遍历 raw page nodes：visible Text 保留当前全部恢复逻辑，marker 保护节点直接把安全 bbox 加入 Source 恢复列表。不要新增第二个公开 text collector。

生产 `Model` 直接持有现有两个模型，不增加 trait：

```rust
pub struct Model {
    vl: tokio::sync::OnceCell<std::sync::Mutex<PaddleOcrVl>>,
    word_boxes: tokio::sync::Mutex<PpOcrV5>,
    cpu: bool,
}

let pp = self.word_boxes.lock().await;
dispatch_source_gate(
    &image,
    ctx.scene,
    ctx.page,
    |_, crop| pp.word_boxes(crop),
    |crops| async move {
        let vl = self
            .vl
            .get_or_try_init(|| async {
                let backend = shared_llama_backend(ctx.runtime)?;
                let loaded = PaddleOcrVl::load(ctx.runtime, self.cpu, backend).await?;
                Ok::<_, anyhow::Error>(std::sync::Mutex::new(loaded))
            })
            .await?;
        let mut vl = vl
            .lock()
            .map_err(|_| anyhow::anyhow!("PaddleOCR mutex poisoned"))?;
        Ok(vl
            .inference_images(&crops, PaddleOcrVlTask::Ocr, MAX_NEW_TOKENS)?
            .into_iter()
            .map(|output| output.text)
            .collect())
    },
)
.await
```

目标中文节点 op 固定为：

```rust
Op::UpdateNode {
    page,
    id: node_id,
    patch: NodePatch {
        transform: Some(target_transform),
        visible: Some(true),
        data: Some(NodeDataPatch::Text(TextDataPatch {
            source_lang: Some(Some("zh".into())),
            detector: Some(Some("pp-ocr-v5-source-gate".into())),
            line_polygons: Some(Some(target.line_polygons)),
            text: Some(Some(target.text)),
            translation: Some(None),
            font_prediction: Some(None),
            style: Some(None),
            sprite: Some(None),
            sprite_transform: Some(None),
            typography_plan_verified: Some(false),
            ..Default::default()
        })),
    },
    prev: NodePatch::default(),
}
```

设置 gate detector 名称可阻止 `expanded_text_block_crop_bounds()` 再按旧 CTD metadata 扩张回英文区域；Gate 已写入 VL 权威中文 OCR，HanOnly 不再运行第二次 OCR。

- [ ] **Step 5: Detector 根据策略创建 provisional 节点**

把 `new_text_node()` 改成：

```rust
pub fn new_text_node(bbox: [f32; 4], text_data: TextData, visible: bool) -> Node {
    Node {
        id: NodeId::new(),
        transform: Transform {
            x: bbox[0],
            y: bbox[1],
            width: bbox[2] - bbox[0],
            height: bbox[3] - bbox[1],
            rotation_deg: text_data.rotation_deg.unwrap_or(0.0),
        },
        visible,
        kind: NodeKind::Text(text_data),
    }
}
```

四个 detector 调用统一传：

```rust
let visible = ctx.options.source_text_policy == SourceTextPolicy::AllText;
let node = new_text_node(bbox, text, visible);
```

`ui/hooks/useCurrentPage.ts::textNodesOf()` 在 kind 检查前增加：

```ts
if (node.visible === false) continue
```

- [ ] **Step 6: 运行定向测试并确认 PASS**

```bash
bun cargo test -p koharu-app pipeline::engines::source_language_gate::tests --lib
bun cargo test -p koharu-app pipeline::engines::support::tests --lib
bun run --filter ui test -- tests/hooks/useCurrentPage.test.tsx
```

Expected: PASS；纯英文 VL closure 调用数为 0；混排只对可能含 Han 的原始 crop 调用 VL；Brush 的 Inpainted 结果保留；测试只使用内存图片和 closure，不加载模型。

- [ ] **Step 7: 提交 Task 3**

```bash
git add crates/koharu-app/src/pipeline/engines/source_language_gate.rs crates/koharu-app/src/pipeline/engines/support.rs crates/koharu-app/src/pipeline/engines/pp_doclayout.rs crates/koharu-app/src/pipeline/engines/anime_text.rs crates/koharu-app/src/pipeline/engines/ctd_full.rs crates/koharu-app/src/pipeline/engines/comic_text_bubble.rs ui/hooks/useCurrentPage.ts ui/tests/hooks/useCurrentPage.test.tsx
git commit -m "feat: gate visible text nodes by Chinese source"
```

---

### Task 4: 建立 Source Gate DAG 和零目标整页短路

**Files:**
- Modify: `crates/koharu-app/src/pipeline/artifacts.rs`
- Modify: `crates/koharu-app/src/pipeline/engine.rs`
- Modify: `crates/koharu-app/src/pipeline/mod.rs`
- Modify: `crates/koharu-app/src/pipeline/engines/source_language_gate.rs`
- Modify: `crates/koharu-app/src/pipeline/engines/paddle_ocr.rs`
- Modify: `crates/koharu-app/src/pipeline/engines/yuzumarker_font.rs`
- Modify: `crates/koharu-app/src/pipeline/engines/bubble_segmentation.rs`
- Modify one-line `needs`: `crates/koharu-app/src/pipeline/engines/{ctd_segment.rs,llm_translate.rs,typography.rs,lama.rs,aot.rs,flux2_klein.rs,renderer.rs}`

- [ ] **Step 1: 写 DAG 与整页短路失败测试**

在 `pipeline/engine.rs` 增加：

```rust
#[test]
fn orders_source_gate_after_detector_and_before_every_downstream_stage() {
    let ids = ordered_ids(&[
        "pp-doclayout-v3",
        "pp-ocr-v5-source-gate",
        "yuzumarker-font-detection",
        "speech-bubble-segmentation",
        "comic-text-detector-seg",
        "llm",
        "cloud-typography-planner",
        "lama-manga",
        "koharu-renderer",
    ]);
    let position = |id| ids.iter().position(|item| *item == id).unwrap();
    assert!(position("pp-doclayout-v3") < position("pp-ocr-v5-source-gate"));
    for consumer in [
        "yuzumarker-font-detection",
        "speech-bubble-segmentation",
        "comic-text-detector-seg",
        "llm",
        "cloud-typography-planner",
        "lama-manga",
        "koharu-renderer",
    ] {
        assert!(position("pp-ocr-v5-source-gate") < position(consumer));
    }
}
```

在 `pipeline/mod.rs` 使用现有 `Registry::insert_test_engine()` 增加：

```rust
#[tokio::test]
async fn han_only_empty_source_gate_stops_every_downstream_engine() {
    let fixture = PipelineFixture::pure_english_full_pipeline();
    let outcome = fixture.run().await.unwrap();
    assert_eq!(outcome.warning_count, 0);
    assert_eq!(fixture.calls("detector"), 1);
    assert_eq!(fixture.calls("source-gate"), 1);
    for id in ["font", "bubble", "segment", "translator", "typography", "inpaint", "renderer"] {
        assert_eq!(fixture.calls(id), 0, "unexpected downstream call: {id}");
    }
}
```

同一测试模块再增加：

- `han_only_downstream_only_existing_english_runs_gate_and_skips_renderer`：spec 只有 Renderer，Scene 已有 visible English Text；Gate=1，Renderer=0，英文节点与 stale Rendered 被清理。
- `han_only_zero_text_standalone_renderer_keeps_existing_behavior_without_loading_gate`：Scene 无 Text、spec 只有 Renderer；Gate load/run=0，Renderer=1。
- `repair_region_never_injects_source_gate`：`options.region=Some(_)`；Gate=0，单 Inpainter=1。
- `all_text_keeps_selected_ocr_and_never_injects_gate`：AllText 下 catalog infos 仍含所选 OCR，Gate=0。
- `han_only_replaces_selected_ocr_with_gate`：HanOnly 下 catalog infos 不含 `paddle-ocr-vl-1.6`/其他所选 OCR，只含 Gate，避免 VL 重复调用。

- [ ] **Step 2: 运行测试并确认 FAIL**

```bash
bun cargo test -p koharu-app orders_source_gate_after_detector_and_before_every_downstream_stage --lib
bun cargo test -p koharu-app han_only_empty_source_gate_stops_every_downstream_engine --lib
bun cargo test -p koharu-app han_only_downstream_only_existing_english_runs_gate_and_skips_renderer --lib
```

Expected: FAIL，因 artifact、自动注入和短路尚不存在。

- [ ] **Step 3: 增加 DAG-only artifact 并修改依赖**

在 `Artifact` 增加：

```rust
/// Candidate text boxes that passed the configured source-text gate.
/// DAG-only ordering token; accepted nodes remain ordinary Text nodes.
SourceTextBoxes,
```

`Artifact::ready()` 对它返回 `true`。Source Gate 实际写入权威 OCR 文本，但只声明内部 DAG token，避免作为普通 OCR 出现在 engine catalog：

```rust
EngineInfo {
    id: "pp-ocr-v5-source-gate",
    name: "PP-OCRv5 Source Gate",
    needs: &[Artifact::TextBoxes],
    produces: &[Artifact::SourceTextBoxes],
    load: |runtime, cpu| Box::pin(async move {
        let word_boxes = PpOcrV5::load(runtime).await?;
        Ok(Box::new(Model {
            vl: tokio::sync::OnceCell::new(),
            word_boxes: tokio::sync::Mutex::new(word_boxes),
            cpu,
        }) as Box<dyn Engine>)
    }),
}
```

Gate load 只加载 PP-OCRv5；PaddleOCR-VL 使用 Task 3 的 `OnceCell`，仅在 pending Han crops 非空时加载。因此纯英文页不下载、不加载、不调用 PaddleOCR-VL。

在 `pipeline/mod.rs::step_for()` 把 `Artifact::SourceTextBoxes` 映射到 `PipelineStep::Ocr`，确保 Gate 有现成 OCR 进度标签；不新增 PipelineStep。

Font Detector、Bubble Segmenter 以及 Segment、Translator、Typography、三个 Inpainter、Renderer 的 `needs` 各追加 `Artifact::SourceTextBoxes`。保留原 needs：`TextBoxes` 继续提供 AllText 的 Detector edge，OcrText/Translations 等原 edge 不变；只有 HanOnly 注入 Gate 时才新增 Gate → downstream edge。

- [ ] **Step 4: 中文模式自动注入内部 gate**

在 `pipeline/mod.rs` 提取：

```rust
struct ResolvedInfos {
    infos: Vec<&'static EngineInfo>,
    source_stage_selected: bool,
}

fn touches_text_pipeline(info: &EngineInfo) -> bool {
    const TEXT_ARTIFACTS: &[Artifact] = &[
        Artifact::TextBoxes,
        Artifact::OcrText,
        Artifact::SourceTextBoxes,
        Artifact::FontPredictions,
        Artifact::SegmentMask,
        Artifact::BubbleMask,
        Artifact::Translations,
        Artifact::TypographyStyles,
        Artifact::Inpainted,
        Artifact::RenderedSprites,
        Artifact::FinalRender,
    ];
    info.needs
        .iter()
        .chain(info.produces.iter())
        .any(|item| TEXT_ARTIFACTS.contains(item))
}

fn infos_for_spec(spec: &PipelineSpec) -> Result<ResolvedInfos> {
    let mut infos = spec
        .steps
        .iter()
        .map(|id| Registry::find(id))
        .collect::<Result<Vec<_>>>()?;
    let source_stage_selected = infos.iter().any(|info| {
        info.produces.contains(&Artifact::TextBoxes)
            || info.produces.contains(&Artifact::OcrText)
    });
    if spec.options.source_text_policy == SourceTextPolicy::HanOnly
        && spec.options.region.is_none()
        && infos.iter().any(|info| touches_text_pipeline(info))
    {
        infos.retain(|info| !info.produces.contains(&Artifact::OcrText));
        if !infos.iter().any(|info| info.id == "pp-ocr-v5-source-gate") {
            infos.push(Registry::find("pp-ocr-v5-source-gate")?);
        }
    }
    Ok(ResolvedInfos { infos, source_stage_selected })
}
```

`run()` 使用它替换当前直接映射 `spec.steps` 的代码。HanOnly Gate 已完成 PaddleOCR-VL，所以移除所有所选 `OcrText` producer，避免重复 OCR；AllText 和 `region: Some(_)` 完全不改 infos。不得修改 HTTP `StartPipelineRequest`。

- [ ] **Step 5: 在 gate 后、加载下游 engine 前短路**

在 `source_language_gate.rs` 暴露一个 `pub(crate) fn has_gate_candidates(scene, page) -> bool`，直接遍历 raw page nodes：匹配 Text 且 detector 不是 `pp-ocr-v5-source-gate-protected`。它必须能看到 Detector 刚写入的 `visible:false` provisional 节点；不得使用已经过滤 invisible 的 `support::text_nodes()`。

在 Gate `registry.get()` 前调用它，避免零 Text standalone Renderer 加载 Gate 模型：

```rust
if info.id == "pp-ocr-v5-source-gate" {
    let has_candidates = source_language_gate::has_gate_candidates(
        &session.scene_snapshot(),
        *page_id,
    );
    if !has_candidates {
        completed += 1;
        if resolved.source_stage_selected {
            completed += (total_steps - seq - 1) as u64;
            continue 'pages;
        }
        continue;
    }
}
```

把当前 `if ops.is_empty() { continue; }` 改成“非空才 apply”，然后无论 Gate 返回空 ops 还是 RemoveNode ops，都在 apply 后执行：

```rust
if !ops.is_empty() {
    // 保留当前 Op::Batch、Typography apply_if_epoch 和普通 session.apply 分支原代码。
}
if info.id == "pp-ocr-v5-source-gate"
    && text_nodes(&session.scene_snapshot(), *page_id).is_empty()
{
    completed += (total_steps - seq - 1) as u64;
    continue 'pages;
}
```

不要为一次调用提取 `apply_step_ops()`；只用 `if !ops.is_empty()` 包住当前 Batch/apply 原代码，并把 Gate post-check 放到该分支之后。上述检查保证：Detector/OCR 源阶段无候选时整页短路；零 Text standalone Renderer 跳过 Gate 后继续；已有 Text 的 downstream-only 路径先运行 Gate，再按结果决定是否短路。

- [ ] **Step 6: 删除 PaddleOCR-VL 内重复 PP-OCRv5 分支**

从 `paddle_ocr.rs` 删除已经移动到 Gate 的 `word_boxes: AsyncMutex<Option<PpOcrV5>>`、`dispatch_inline_word_boxes()`、`build_pp_ocr_word_box_update()` 和重复测试。`Model` 恢复只持有：

```rust
pub struct Model(Mutex<PaddleOcrVl>);
```

`run()` 保持普通节点级 `inference_images()`。HanOnly 的 resolved infos 已移除该 engine；AllText 不注入 Gate，仍对所有 detector 节点执行用户选择的原有 OCR。

- [ ] **Step 7: 运行定向回归并确认 PASS**

```bash
bun cargo test -p koharu-app pipeline::engine::tests --lib
bun cargo test -p koharu-app pipeline::tests --lib
bun cargo test -p koharu-app pipeline::engines::source_language_gate::tests --lib
bun cargo test -p koharu-app pipeline::engines::paddle_ocr::tests --lib
```

Expected: PASS；纯英文 full pipeline 只有 Detector + PP Gate 调用且 VL=0；已有纯英文节点的 renderer-only 路径 Renderer=0；零 Text standalone Renderer=1；Repair Brush Gate=0；AllText 保留原 OCR。

- [ ] **Step 8: 提交 Task 4**

```bash
git add crates/koharu-app/src/pipeline/artifacts.rs crates/koharu-app/src/pipeline/engine.rs crates/koharu-app/src/pipeline/mod.rs crates/koharu-app/src/pipeline/engines/source_language_gate.rs crates/koharu-app/src/pipeline/engines/paddle_ocr.rs crates/koharu-app/src/pipeline/engines/yuzumarker_font.rs crates/koharu-app/src/pipeline/engines/bubble_segmentation.rs crates/koharu-app/src/pipeline/engines/ctd_segment.rs crates/koharu-app/src/pipeline/engines/llm_translate.rs crates/koharu-app/src/pipeline/engines/typography.rs crates/koharu-app/src/pipeline/engines/lama.rs crates/koharu-app/src/pipeline/engines/aot.rs crates/koharu-app/src/pipeline/engines/flux2_klein.rs crates/koharu-app/src/pipeline/engines/renderer.rs
git commit -m "feat: stop Chinese pipelines after empty source gate"
```

---

### Task 5: 锁定英文不框选、不修改和混排只处理中文

**Files:**
- Modify: `crates/koharu-app/src/pipeline/mod.rs`
- Modify: `crates/koharu-app/src/pipeline/engines/source_language_gate.rs`
- Modify: `crates/koharu-app/src/pipeline/engines/support.rs`
- Modify: `ui/tests/components/TextBlocksPanel.test.tsx`

- [ ] **Step 1: 增加端到端、无模型验收测试**

使用 fake Detector/Source Gate/下游 engines 与内存 Source 图片覆盖以下测试名和断言：

- `pure_english_has_no_visible_text_nodes_and_source_pixels_are_unchanged`：所有下游计数为 0、Scene 无 Text/派生层、Source hash 和像素不变。
- `complete_english_word_is_preserved_while_adjacent_han_runs_downstream`：fake downstream 只观察到中文 bbox；最终英文 ROI 与 Source 逐像素相等。
- `single_latin_label_with_han_is_one_translation_target`：fake Translator 收到且只收到 `S型曲线`。
- `single_latin_on_another_line_is_protected_not_translated`：输入 `S\n中文`，Translator 只收到中文，`S` 保护 ROI 与 Source 相等。
- `pp_false_han_requires_vl_confirmation_before_downstream`：PP closure 返回 Han、VL 返回纯英文或不匹配；节点删除，所有下游计数为 0。
- `separable_ai_han_keeps_ai_and_translates_han`：PP 返回 `AI` + `智能塑形` 时只翻译中文；单框 `AI智能塑形` 安全跳过。
- `unsafe_mixed_geometry_is_removed_and_never_reaches_downstream`：候选被删除，所有下游计数为 0。
- `downstream_only_existing_english_runs_gate_before_renderer`：Scene 已有 visible English、spec 只有 Renderer；Gate 先运行且 Renderer=0。
- `empty_targets_keep_repair_brush_inpainted_pixels`：Brush 与 Inpainted 均保留，Rendered/Segment/Bubble 被删除。
- `all_text_keeps_existing_nodes_and_runs_existing_ocr_path`：gate 计数为 0，现有 OCR 观察到全部节点。

像素断言使用不同颜色标记英文 ROI、中文 ROI 和背景；最终图必须满足：英文 ROI 与 Source 逐像素相等，只有中文 ROI 允许变化。

- [ ] **Step 2: 运行验收测试并确认 FAIL**

```bash
bun cargo test -p koharu-app pure_english_has_no_visible_text_nodes_and_source_pixels_are_unchanged --lib
bun cargo test -p koharu-app complete_english_word_is_preserved_while_adjacent_han_runs_downstream --lib
bun cargo test -p koharu-app single_latin_label_with_han_is_one_translation_target --lib
bun cargo test -p koharu-app single_latin_on_another_line_is_protected_not_translated --lib
bun cargo test -p koharu-app downstream_only_existing_english_runs_gate_before_renderer --lib
bun cargo test -p koharu-app empty_targets_keep_repair_brush_inpainted_pixels --lib
bun cargo test -p koharu-app all_text_keeps_existing_nodes_and_runs_existing_ocr_path --lib
```

Expected: 至少一个测试暴露尚未接线的生产路径；如果全部提前 PASS，先确认测试确实调用 `run()` 和生产 gate dispatch，而不是只测纯函数。

- [ ] **Step 3: 只修验收暴露的生产接线缺口**

不再增加新类型。允许的修正仅包括：Source Gate 决策、保护节点生成、ops 顺序、driver 短路、Segment 最终 support intersection、Renderer Source 像素恢复和 stale layer 清理。先运行现有生产 seam 测试，只有失败时才修改对应 engine：

```bash
bun cargo test -p koharu-app segment_dispatch_word_box_inline_mixed_keeps_english_roi_zero --lib
bun cargo test -p koharu-app final_inpaint_mask_keeps_word_box_english_word_zero --lib
bun cargo test -p koharu-app lama_inpaint_dispatch_receives_final_mask --lib
bun cargo test -p koharu-app aot_inpaint_dispatch_receives_final_mask_and_preserves_repair_region --lib
bun cargo test -p koharu-app flux2_inpaint_dispatch_receives_final_mask --lib
bun cargo test -p koharu-app han_only_renderer_restores_validated_english_pixels_from_source --lib
```

Expected: 全部 PASS。若已 PASS，不修改 `ctd_segment.rs`、三个 Inpainter 或 `renderer.rs`，也不把未修改文件加入提交。

- [ ] **Step 4: UI 验收**

在 `TextBlocksPanel.test.tsx` 增加 Scene 中同时存在 `visible:false` 英文 provisional node 与 `visible:true` 中文 node 的 fixture，断言面板只显示中文节点，英文没有 Generate 按钮或编号框。

```bash
bun run --filter ui test -- tests/components/TextBlocksPanel.test.tsx tests/hooks/useCurrentPage.test.tsx
```

Expected: PASS。

- [ ] **Step 5: 提交 Task 5**

```bash
git add crates/koharu-app/src/pipeline/mod.rs crates/koharu-app/src/pipeline/engines/source_language_gate.rs crates/koharu-app/src/pipeline/engines/support.rs ui/tests/components/TextBlocksPanel.test.tsx
git commit -m "test: lock Chinese-only source pipeline behavior"
```

---

### Task 6: 更新文档并执行质量门禁

**Files:**
- Modify: `docs/zh-CN/reference/settings.md`

- [ ] **Step 1: 更新中文使用文档**

文档必须说明：

- “设置 → 引擎 → 源文字 → 中文（推荐）”的操作方法；
- 纯英文只有内部候选判定，不形成可见框，不进入完整 OCR 或后续；
- 完整英文词与中文混排只处理中文；`S型曲线` 整体处理；
- PP-OCRv5 无法提供可分离词框时安全跳过，不按比例猜测 `AI智能塑形` 等单框内部字符位置；
- 低置信度/无法安全分离时保留 Source；
- 纯汉字日文与中文不能仅凭脚本绝对区分；
- `all_text` 是兼容模式。

- [ ] **Step 2: 在 Task 1 生成物已经提交后运行完整质量门禁**

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
git diff --check
git status --short
```

Expected: 全部退出码 0；默认测试不下载模型；lint 只允许仓库已有且未触及的 warning；`check:generated` 重新生成后看到已提交的 Task 1 生成物，退出码 0 且不留下 diff。

- [ ] **Step 3: 人工验收当前电商样例**

用当前问题图片运行中文 Full Pipeline：

- `SLENDER WAIST`、`S-CURVE`、`PEACH HIP` 无红框、无 TextBlocksPanel 条目、像素不变；
- 中文块正常进入 OCR/翻译/擦除/排版/渲染；
- `Peach 蜜桃臀` 只显示中文目标框并保留 `Peach`；
- `S型曲线` 整体进入中文目标；
- `S\n中文` 只显示中文目标，独立 `S` 不擦除；
- 可分框 `AI` + `智能塑形` 只处理中文，单框不可分时整块安全跳过；
- Activity 日志证明纯英文页没有 PaddleOCR-VL、Font、Segment、Translator、Typography、Inpainter、Renderer 调用。

- [ ] **Step 4: 提交 Task 6**

```bash
git add docs/zh-CN/reference/settings.md
git commit -m "docs: document Chinese source text gating"
```

---

## 最终停止条件

- 中文模式下，纯英文单词和句子没有持久 Text 节点、UI 框或完整 OCR 调用。
- PP-OCRv5 判断无 Han 时 PaddleOCR-VL 调用数为 0；判断含 Han 时，只有通过 VL 原 crop 严格字符对齐的候选才能公开。
- 中英混排只产生 visible 中文目标；被排除英文只保留 invisible Source 像素保护 metadata，不进入普通 `text_nodes()`；英文 ROI 在 Segment、Inpaint、Render 后均与 Source 一致。
- `S型曲线`、`A版` 的单字母只在与 Han 同排连续时并入目标；不同排单字母保留。
- PP-OCRv5 返回不可分的 `AI智能塑形` 单框时安全跳过；提供 `AI` + `智能塑形` 独立框时只处理中文。
- 零中文目标时所有后续 engine 在加载/运行前停止，Source 和 Repair Brush 语义保留。
- downstream-only 已有纯英文节点先过 Gate；零 Text standalone Renderer 和 `region: Some(_)` Repair Brush 不加载 Gate。
- `all_text` 保留用户选择的原 OCR；旧 TOML、HTTP 路径、`StartPipelineRequest` 和手动 Repair Brush行为无回归。
- 自动化测试不加载或下载真实模型；人工样例完成最终视觉确认。
