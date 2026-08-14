# Lane 执行合同:L-AR08 — ZIP 安全(entry 路径验证 + 全量预验证与预算)

- 状态:**Phase 3 一次批准已覆盖(2026-08-13 授予)**;LOOP-3 本地认领已登记
- 认领基线:Phase 3 起点 main `b68f123e`;执行分支 `audit-remediation-phase3`(认领时 tip `5ffd1bfb`)
- 提交/回滚单元:T01/T02 各一个单卡 commit(可独立 revert);lane 收口 docs(evidence) 单独一个 commit
- 前置依赖:无(队列标注依赖已齐)
- 执行环境偏差(继承):子代理模型映射故障未修(13 次失败),codegraph + 直接工具单执行器;lane 收口独立 review 重试,失败则对抗性自审并落档

## 范围文件域(域外改动禁止)

| 卡 | 允许文件 |
|---|---|
| AR08-T01 | `ui/lib/io/saveBlob.ts`、`ui/tests/lib/io/saveBlob.test.ts`(≤2) |
| AR08-T02 | `ui/lib/io/saveBlob.ts`、`ui/tests/lib/io/saveBlob.test.ts`(≤2) |

## 卡:AR08-T01 — ZIP entry 路径验证

- **验收标准(TASKS 原文)**:RED:`..`、`.`、空段、POSIX absolute、drive、UNC、反斜杠 traversal 产生写入。GREEN:一次纯验证/规范化边界;目标严格为选择目录后代。
- **现状(RED-0 源码实证)**:`saveBlob.ts:34-44` unzipSync 后逐 entry 仅 `replace(/\\/g,'/')` 拼 `${folder}/${normalized}` 直接 mkdir+writeFile——`../x`、绝对路径、盘符、UNC 全部可写出选择目录。
- **设计**:纯函数 `sanitizeZipEntryName(name): string | null`——反斜杠归一后:以 `/` 结尾视为目录条目跳过(返回特殊语义);任一段为 ``/`.`/`..`、含空段、POSIX absolute(`/…`)、drive(`C:`)、UNC(`//…`)→ null 拒绝。验证在写入边界前一次性完成。
- **RED 断言**(`bun run --cwd ui test -- tests/lib/io/saveBlob.test.ts`;mock plugin-dialog/plugin-fs,zip 用 fflate zipSync 构造):
  1. `zip_entry_traversal_rejected` — 含 `../evil.png` 的 zip → 拒绝,零 writeFile → 当前写出 → FAIL
  2. `zip_entry_absolute_drive_unc_rejected` — `/abs.png`、`C:/win.png`、`//unc/x.png`、反斜杠 `..\\evil.png` → 各拒绝 → FAIL
  3. `zip_entry_dot_and_empty_segment_rejected` — `a/./b.png`、`a//b.png` → 拒绝;目录条目 `a/` 跳过不写文件 → FAIL
  4. 锁:正常嵌套 `dir/page.png` → 写入 `${folder}/dir/page.png`
- **目标文件**:上表 T01 行(≤2)
- **验收命令**:`bun run --cwd ui test -- tests/lib/io/saveBlob.test.ts`、`bun run lint:ui`
- **证据记录**:RED / GREEN / commit SHA

## 卡:AR08-T02 — ZIP 全量预验证与预算

- **验收标准(TASKS 原文)**:RED:非法/超预算 ZIP 在发现错误前已经 mkdir/write。GREEN:所有 entry 与总预算在第一次写前验证;零部分文件。停止:若 unzipSync 无法在分配前有界,回 PLAN。
- **停止条件裁决(侦查取证,2026-08-14)**:fflate 0.8.3 流式 `Unzip`(`onfile` 预验 + `ondata` chunk 累计 + `terminate()`)支持分配前有界——**不回 PLAN**。
- **现状(RED-0 源码实证)**:逐 entry mkdir+write,无预验证(第 N 个非法时前 N-1 个已写盘);无任何预算(ZIP bomb 声明尺寸伪造可致大分配/大写盘)。
- **设计**:`extractZipSafely(bytes)` 两阶段:阶段一(流式遍历)每 entry 先 `sanitizeZipEntryName` + 声明尺寸累计预算(entries ≤ 4096、总解压 ≤ 4 GiB、单文件 ≤ 2 GiB——**预算值系本合同决策点**:TASKS 未给批准值,按导出场景防 bomb 推定),ondata 实际解压累计超预算即 terminate;阶段一全部通过前不写任何文件;阶段二才 mkdir/write。伪声明尺寸(声明小实际大)由 ondata 实际累计兜底 terminate。
- **RED 断言**:
  1. `zip_invalid_entry_zero_partial_writes` — 合法×2 + 非法×1 混合 zip → 拒绝且 writeFile/mkdir 零调用;当前前 2 个已写 → FAIL
  2. `zip_over_budget_rejected_before_writes` — 声明尺寸累计超预算(构造多文件声明总和 >4 GiB,内容小——fflate zipSync 尺寸由内容定……改用预算覆盖缝:cfg(test)式预算 setter,测试用 3 张小图超小预算)→ 拒绝零写入;当前无预算 → FAIL
  3. `zip_declared_size_lie_caught_by_actual_count` — ondata 实际累计超预算 → terminate 零写入(覆盖缝小预算下构造)
  4. 锁:合法 zip 全量写出(T01 锁用例不破)
- **目标文件**:上表 T02 行(≤2)
- **验收命令**:`bun run --cwd ui test -- tests/lib/io/saveBlob.test.ts`
- **证据记录**:RED / GREEN / commit SHA

---

## Lane 收口门禁(Wave 4 gate 对齐)

- `bun run test:ui`、`bun run lint:ui`、`bun run format:check`、`bun run --cwd ui build`(UI 卡固定门禁,T03 教训)
- `bun cargo check --workspace --all-targets`(零 Rust 面,确认无意外牵连)
- 独立 scoped code-review 零发现(重试子代理;故障则对抗性自审并落档偏差)
- traversal 拒绝与零部分写入可重复演示(测试输出)

## 风险与决策点(批准时一并确认)

- **预算值自定(决策点)**:TASKS 未给批准值;entries ≤4096 / 总解压 ≤4 GiB / 单文件 ≤2 GiB,按"导出用户自己项目内容、防恶意/损坏 zip"场景推定
- Windows path smoke 无法在 macOS 实测——以反斜杠归一 + drive/UNC 拒绝的单元测试覆盖,落档说明
- 大 ZIP UI smoke 以预算覆盖缝的小预算测试替代,落档说明
