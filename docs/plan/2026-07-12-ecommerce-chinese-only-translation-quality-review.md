# 电商图片仅中文翻译与渲染质量计划最终审查

**审查对象：** `docs/plan/2026-07-11-ecommerce-chinese-only-translation-quality-plan.md`

**审查日期：** 2026-07-12

**审查方式：** 只读、基于当前仓库真实源码

## 最终结论

**BLOCK**

计划结构、API 兼容和大部分 TDD 顺序已经较完整，但仍有两个核心不变量没有成立：无 `line_polygons` 的混合节点使用等高猜测，以及 Flux2 在共享 helper 之后再次扩张 mask。按当前文档直接实施，仍可能擦除英文。

## 问题清单

### BLOCKER 1：无 polygons 混合节点的等高切分不安全

- **计划位置：** `L222-L230`
- **源码证据：** `crates/koharu-core/src/scene.rs:279`、`crates/koharu-ml/src/types.rs:23`、`crates/koharu-ml/src/comic_text_detector/postprocess.rs:85`
- **具体问题：** 缺少 `line_polygons` 时，源码只有节点矩形，没有真实行高、间距或基线。2–4 行等高切分只能猜测，不能证明中文区域不会覆盖相邻英文。
- **实施影响：** Segment、Inpaint 和 Renderer 可能继续触及英文，违反“英文保持原像素”。
- **最小修正方向：** 混合节点无 polygons 时返回 `None`；没有可靠逐行几何前删除等高 fallback。

### BLOCKER 2：Flux2 在共享 helper 后仍会扩张 mask

- **计划位置：** `L394-L471`
- **源码证据：** `crates/koharu-ml/src/flux2_klein/mod.rs:66`、`crates/koharu-ml/src/flux2_klein/mod.rs:339`、`crates/koharu-ml/src/flux2_klein/latents.rs:61`、`crates/koharu-app/src/pipeline/engines/flux2_klein.rs:64`
- **具体问题：** Flux2 默认 `mask_padding = 16`，在应用层完成二次裁剪后仍会在模型内部膨胀 mask。closure 观察到的 mask 不是模型内部最终使用的 latent mask。
- **实施影响：** 英文 ROI 可能重新进入推理范围，现有 closure 测试会错误通过。
- **最小修正方向：** 关闭重复 padding，或在 Flux2 预处理后重新应用 allowed support；同时验证 crop/full-frame 两条输出路径的 mask 外像素。

### HIGH 1：CLI 无 translator 时 Renderer 执行过早

- **计划位置：** `L500-L512`
- **源码证据：** `crates/koharu-app/bin/pipeline.rs:222`、`crates/koharu-app/bin/pipeline.rs:273`、`crates/koharu-app/src/pipeline/engines/renderer.rs:50`
- **具体问题：** CLI 先运行包含 Renderer 的 pipeline，之后才执行 `synthesize_translations()`；检查 step 列表不能证明 Renderer 获得了 translation。现有 fallback 还会复制完整 OCR，与 HanOnly 冲突。
- **最小修正方向：** 改为两阶段执行：OCR 后按策略生成 fallback，再运行 Renderer，并测试真实阶段边界。

### HIGH 2：垂直 hard-line 仍可能软换列

- **计划位置：** `L730-L754`
- **源码证据：** `crates/koharu-renderer/src/layout.rs:261`、`crates/koharu-renderer/src/layout.rs:564`
- **具体问题：** hard-line 分支不设置 `max_width`，但始终设置 `max_height`；对 `VerticalRl`，`max_height` 正是软换列约束。
- **最小修正方向：** hard-line 模式不设置任一轴的软换行约束，仅由外部宽高比较驱动字号二分，并增加垂直单行/双行测试。

### HIGH 3：engine 测试不能证明生产接线

- **计划位置：** `L321-L336`、`L460-L471`
- **源码证据：** `crates/koharu-app/src/pipeline/engines/lama.rs:27`、`crates/koharu-app/src/pipeline/engines/flux2_klein.rs:62`、`crates/koharu-app/src/pipeline/engines/ctd_segment.rs:24`
- **具体问题：** 三个 engine 测试若只再次调用共享 helper，在生产 engine 绕过 helper 时仍会通过；CTD 也没有无模型的最终写入测试边界。
- **最小修正方向：** 允许最小 backend-local inference closure；CTD 提取生产实际调用的 `finalize_segment_mask()`，测试直接调用生产边界。

### HIGH 4：完全越界 quad 会被夹到图像边缘

- **计划位置：** `L340-L378`
- **源码证据：** `crates/koharu-ml/src/types.rs:23`、`crates/koharu-ml/src/comic_text_detector/postprocess.rs:170`
- **具体问题：** 计划将完全越界 quad 的所有点 clamp 到边缘后仍绘制 polygon，可能生成退化的边缘像素。
- **最小修正方向：** clamp 前检查 bbox；完全越界、非有限或退化 quad 直接跳过，并断言 support 全零。

### MEDIUM 1：严格翻译的零 ops 原子性测试边界未定义

- **计划位置：** `L641-L664`
- **源码证据：** `crates/koharu-app/src/llm.rs:218`、`crates/koharu-app/src/pipeline/engines/llm_translate.rs:23`
- **具体问题：** 计划禁止 provider fake/hook，却要求 engine 层证明 strict Provider 错误时 ops 为空，没有指定可测试的两阶段生产函数。
- **最小修正方向：** 明确纯 `build_han_only_translation_ops(...)`；`run()` 只能在 strict await 和全部校验成功后调用。

## 缺失的阻断性测试

- 混合节点无 polygons 时必须返回 unsupported，且 Segment/Inpaint/Renderer 均不触及英文。
- Flux2 默认 padding、crop/full-frame 两条路径下，有效 mask 和最终输出的英文 ROI 均不变化。
- Lama、AOT、Flux2 生产调用点绕过共享 helper 时测试必须失败。
- CTD 测试必须验证生产 finalization 后实际准备写入的 mask。
- CLI 无 translator 时必须在 fallback 完成后才调用 Renderer，HanOnly 不复制英文。
- 垂直 hard-line 单行/双行不得产生额外列。
- 完全越界 quad 必须得到全零 support。

## 可删除或简化项

`计划:L245: DTO: EligiblePageLines 仅包装两个 Vec；可直接返回具名 tuple/type alias。`

`计划:L323: 重复测试: 删除三个仅重复调用共享 helper 的 engine 测试，改为共享表驱动测试和真正调用生产接线的最小测试。`

`计划:L514: UI helper: 两个新文件只封装两个短数组；可直接修正两个调用点，并把自定义流程断言并入现有 MenuBar 测试。`

`net: -35 lines possible.`

## Task 执行就绪度

| Task | 状态 | 主要原因 |
| --- | --- | --- |
| Task 1 | READY | Serde、Default、ToSchema、四入口借用 builder、Orval 命名和生成物顺序均兼容当前仓库。 |
| Task 2 | NOT READY | 无 polygons 混合节点的等高切分无法保证英文安全。 |
| Task 3 | NOT READY | Flux2 内部 padding 绕过最终交集，engine/CTD 测试不能证明生产接线。 |
| Task 4 | NOT READY | 严格 parser 可执行，但 Provider 失败到零 ops 的生产测试边界未定义。 |
| Task 5 | NOT READY | hard-line 对垂直布局仍会因 `max_height` 软换列。 |
| Task 6 | NOT READY | 依赖 Task 2–5 的核心不变量，当前不能作为最终门禁。 |

## 必须修改项

1. 删除混合无 polygons 的等高 fallback，改为 unsupported。
2. 处理 Flux2 内部 `mask_padding` 和 full-frame 输出边界，并增加有效 mask/输出像素测试。
3. 修正 CLI 无 translator 的 fallback 与 Renderer 执行时序，同时应用 HanOnly scope。
4. 为三种 Inpainter 和 CTD 指定真正调用生产接线的无模型测试边界。
5. hard-line 模式不得设置任一轴的软换行约束，并补充垂直测试。
6. 越界或退化 quad 在 rasterization 前直接跳过。
7. 明确严格翻译“全部验证完成后才构造 ops”的可测试纯函数边界。

## 审查验证记录

- 计划引用的现有 Rust、TypeScript、Cargo、OpenAPI、Orval、package scripts 和测试路径均存在。
- Markdown 共 38 个代码围栏，数量成对。
- `git diff --no-index --check /dev/null docs/plan/2026-07-11-ecommerce-chinese-only-translation-quality-plan.md` 未发现空白错误。
- 审查过程未修改计划、源码、测试、配置或生成物，也未执行提交。
