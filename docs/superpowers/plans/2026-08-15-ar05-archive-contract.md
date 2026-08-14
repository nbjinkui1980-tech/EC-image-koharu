# Lane 执行合同:L-AR05-ARCHIVE — 归档预算(Archive 读取预算 + Import 原子发布 + History frame 上限)

- 状态:**Phase 3 一次批准已覆盖(2026-08-13 授予)**;LOOP-3 本地认领已登记
- 认领基线:Phase 3 起点 main `b68f123e`;执行分支 `audit-remediation-phase3`(认领时 tip `24bef959`)
- 提交/回滚单元:T02/T03/T04 各一个单卡 commit(可独立 revert);lane 收口 docs(evidence) 单独一个 commit
- 前置依赖:T03 ← T02(本 lane 内)+ AR04-T03 ✅`eb8b9ccc` + AR02-T04 ◐(实现已集成,逐卡证据待补录);T04 ← AR04-T03 ✅
- 执行环境偏差(继承):子代理模型映射故障未修(16 次失败),codegraph + 直接工具单执行器;lane 收口独立 review 重试,失败则对抗性自审并落档

## 范围文件域(域外改动禁止)

| 卡 | 允许文件 |
|---|---|
| AR05-T02 | `crates/koharu-app/src/archive.rs`(含 #[cfg(test)])(≤1) |
| AR05-T03 | `crates/koharu-rpc/src/routes/projects.rs`、`crates/koharu-app/src/archive.rs`、`crates/koharu-app/src/session.rs`(≤3) |
| AR05-T04 | `crates/koharu-app/src/history.rs`(含 #[cfg(test)])(≤1) |

新依赖:无。串行点:不占用 lockfile/format/Orval/Next build;Rust 卡共用受保护 target,串行。

## 决策点(预算常量,执行时定值并回填)

- T02 预算:`MAX_ENTRIES`、`MAX_ENTRY_BYTES`(单项实际展开)、`MAX_TOTAL_BYTES`(总实际展开)、`MAX_RATIO = 100`(TASKS 定)、伪造声明 size 拒绝阈值 = `MAX_ENTRY_BYTES`。定值依据:`.khr` = project.toml + scene.bin + history.log + blobs/*(webp/jpg,Stored);blobs 单项/总量需覆盖真实大项目(百页级)。
- T04 预算:TASKS 已定 frame 上限 16 MiB。

## 卡:AR05-T02 — Archive 实际读取预算

- **验收标准(TASKS 原文)**:文件:`crates/koharu-app/src/archive.rs`。RED:entry、单项、总展开、100:1、伪造 size 的 limit+1 未拒绝或先大分配。GREEN:按实际读取 bytes 流式写 staging;批准预算常量;不建 quota framework。验证:`bun cargo test -p koharu-app archive`。
- **现状(RED-0 源码实证)**:`extract_khr_bytes`(archive.rs:117-141)无任何预算:`Vec::with_capacity(entry.size() as usize)` 直接信任 zip 目录声明 size(伪造巨值 → 大分配);无 entry 数/单项/总展开/压缩比检查;`read_to_end` 整读后再整写(非流式,不按实际读取 bytes 计费)。
- **设计**:
  - archive.rs 新增批准预算常量(见决策点);`extract_khr_bytes` 改流式:按实际读取 bytes 边读边写 staging,累计单项与总量,超限即 Err;不建 quota framework
  - 伪造声明 size:`entry.size() > MAX_ENTRY_BYTES` 在读取前直接 Err;分配上界按 `min(declared, MAX_ENTRY_BYTES + 1)`
  - entry 数:循环前 `archive.len() > MAX_ENTRIES` → Err
  - 压缩比:单项实际展开 > max(compressed × 100, 小文件下限) → Err(blobs Stored 天然 ratio≈1,不受影响)
  - 失败时已写入 staging 的清理归调用方(T03);T02 只保证 Err 语义与有界分配
- **RED 断言**(`bun cargo test -p koharu-app archive`):
  1. `archive_rejects_entry_count_above_budget` — MAX_ENTRIES+1 个 entry → Err;当前无检查 → FAIL
  2. `archive_rejects_forged_entry_size_above_budget_before_allocating` — entry 声明 size = MAX_ENTRY_BYTES+1(实际数据很短)→ 读取前 Err;当前 with_capacity(declared) → FAIL
  3. `archive_rejects_single_entry_bytes_above_budget` — 单项实际展开 > MAX_ENTRY_BYTES → Err;当前无检查 → FAIL
  4. `archive_rejects_total_expanded_bytes_above_budget` — 各 entry 低于单项预算、合计 > MAX_TOTAL_BYTES → Err;当前无检查 → FAIL
  5. `archive_rejects_compression_ratio_above_100_to_1` — Deflated entry 展开/压缩 > 100:1 且超下限 → Err;当前无检查 → FAIL
  6. 锁:现有 round-trip / staging 两测试保持 PASS
- **目标文件**:上表 T02 行(≤1)
- **验收命令**:`bun cargo test -p koharu-app archive`
- **证据记录**:RED / GREEN / commit SHA(执行时填)

## 卡:AR05-T03 — Import 原子发布与 cleanup

- **验收标准(TASKS 原文)**:依赖:AR02-T04、AR04-T03、AR05-T02。文件:`crates/koharu-rpc/src/routes/projects.rs`、`crates/koharu-app/src/archive.rs`、`session.rs`。RED:超限/损坏/非法 BlobRef 导入留下 staging/final 或改变当前项目。GREEN:所有验证完成后才 publish/open;失败统一 cleanup。验证:`bun cargo test -p koharu-rpc import`。
- **现状(RED-0 源码实证)**:`sanitize_and_publish_import`(projects.rs:218)已是 allocate → extract → open_untrusted → rename 顺序,错误路径 `remove_dir_all(staging)`;已有测试锁损坏 zip/损坏项目/截断 history/非法 blob 不发布(projects.rs:619-641)与成功发布(653)。缺口:T02 预算拒绝路径无测试锁(超限后 staging 不残留、final 不产生、当前项目不切换);cleanup 对 final 已存在/部分 rename 边界未锁。
- **设计**:
  - 现有测试模块补 RED:超 T02 预算归档(伪造 size / 超总量)→ `sanitize_and_publish_import` Err;断言 staging 目录不存在、final 不存在、config 当前项目未变
  - 若现状已满足,按 AR03-T03 先例激活范围缩减预案,RED 转锁回归并记录;若 cleanup 有缺口则最小修复
- **RED 断言**(`bun cargo test -p koharu-rpc import`):
  1. `failed_import_oversized_archive_leaves_no_staging_or_final` — 超预算归档 → Err;staging/final 均不存在;当前项目不变 → 现状待证
  2. 锁:既有 projects.rs:619-641 四测试保持 PASS
- **目标文件**:上表 T03 行(≤3)
- **验收命令**:`bun cargo test -p koharu-rpc import`
- **证据记录**:RED / GREEN / commit SHA(执行时填)

## 卡:AR05-T04 — History frame 分配前上限

- **验收标准(TASKS 原文)**:依赖:AR04-T03。文件:`crates/koharu-app/src/history.rs`。RED:16 MiB+1 或 `u32::MAX` 长度头触发大分配或被当截断尾忽略。GREEN:分配前拒绝;完整超限 frame 是 corruption/error。验证:`bun cargo test -p koharu-app history_frame_limit`。
- **现状(RED-0 源码实证)**:`replay_with_policy`(history.rs:215)读 4 字节 LE 长度头 → `u32::from_le_bytes(...) as usize`,随后按 len 分配并读 frame;损坏尾容忍逻辑(记录末个全有效 frame 偏移)可能把超限长度头当"截断尾"静默忽略;无 16 MiB 上限。
- **设计**:
  - 批准常量 `MAX_HISTORY_FRAME_BYTES = 16 MiB`;长度头 > 上限 → 分配前 Err(corruption),不落入截断尾容忍分支
  - 区分:长度头本身超上限 = corruption(即使后续字节不足);未超上限但字节不足 = 截断尾(既有容忍语义,AR04-T03)
- **RED 断言**(`bun cargo test -p koharu-app history_frame_limit`):
  1. `history_frame_limit_rejects_oversize_length_header_before_allocating` — 写入 len = 16 MiB+1 的头 + 不足字节 → Err(corruption),非静默截断;现状 FAIL
  2. `history_frame_limit_rejects_u32_max_length_header` — len = u32::MAX → Err(corruption);现状 FAIL
  3. 锁:未超上限截断尾仍按 AR04-T03 语义回滚/容忍(既有 session/history 测试 PASS)
- **目标文件**:上表 T04 行(≤1)
- **验收命令**:`bun cargo test -p koharu-app history_frame_limit`
- **证据记录**:RED / GREEN / commit SHA(执行时填)
