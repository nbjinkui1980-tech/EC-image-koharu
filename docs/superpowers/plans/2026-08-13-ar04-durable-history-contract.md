# Lane 执行合同:L-AR04 — Durable history(Apply/Undo/Redo durable commit + 损坏尾回滚 fail-stop)

- 状态:**Phase 3 一次批准已覆盖(2026-08-13 授予)**;LOOP-3 本地认领已登记
- 认领基线:Phase 3 起点 main `b68f123e`;执行分支 `audit-remediation-phase3`(认领时 tip `b0437c98`)
- 提交/回滚单元:T02/T03 各一个单卡 commit(可独立 revert);lane 收口 docs(evidence) 单独一个 commit
- 前置依赖:AR04-T01 Batch 原子 ✅(`fb7d5546`,scratch-scene 原子发布先例)

## 范围文件域(域外改动禁止)

| 卡 | 允许文件 |
|---|---|
| AR04-T02 | `crates/koharu-app/src/history.rs`、`crates/koharu-app/src/session.rs`(≤2) |
| AR04-T03 | `crates/koharu-app/src/history.rs`、`crates/koharu-app/src/session.rs`(≤2) |

注:`history.rs` 是跨 lane 文件(AR05-T04 frame 上限 ←AR04-T03),本 lane 不预先实现 frame 上限;AR05-T03/T04 🔴 等本 lane T03。

## 卡:AR04-T02 — Apply/Undo/Redo durable commit

- **验收标准(TASKS 原文)**:RED:encode/write/flush/sync 失败后 scene、epoch、两栈、事件或 log length 改变。GREEN:候选状态完成后先 durable frame,再一次发布内存状态。
- **现状(RED-0 源码实证)**:`History::apply/undo/redo` 先 `op.apply(scene)` 改内存并 `epoch+=1`,后 `write_frame`(flush+`sync_data`);持久化失败时 scene/epoch 已变、undo 栈未压入,RPC 层因 Err 不广播 → 内存/磁盘/客户端三方分叉。事件由 RPC 层在 Ok 后广播,事件腿经由"Err 不广播+内存已变"体现。
- **设计**:候选态先行(与 `Op::Batch` scratch 模式同构):
  1. `candidate = scene.clone()`;在 candidate 上 apply(undo/redo 为 inverse);
  2. `write_frame`(frame epoch = self.epoch+1,flush+`sync_data`)失败 → 整体 Err,内存态零变更;
  3. 一次性发布:`*scene = candidate`、`epoch += 1`、栈操作、redo 清空(apply)。
- **故障注入(测试接缝)**:`#[cfg(test)]` hook 使下一次 `write_frame` 在写盘前返回 io 错误(先例:session.rs `compact_apply_sync` cfg(test) 接缝)。encode 失败腿:Op 为纯数据枚举,postcard 编码实际不可达失败;所有 `write_frame` 错误走同一传播路径,由 write/flush/sync 注入腿统一覆盖,合同中注明此等价性。
- **RED 断言**(`bun cargo test -p koharu-app history` / `bun cargo test -p koharu-app session`):
  1. `apply_frame_write_failure_leaves_no_trace` — 注入写失败 → apply 返回 Err;scene 不变、epoch 不变、undo/redo 栈不变、log 字节长度不变
  2. `undo_frame_write_failure_leaves_no_trace` — 同上(undo 路径;redo 栈不变)
  3. `redo_frame_write_failure_leaves_no_trace` — 同上(redo 路径)
  4. `apply_success_publishes_after_durable_frame` — 锁定测试:成功路径行为不变(scene 应用、epoch+1、undo 可逆、log 多一帧)
- **目标文件**:`history.rs`、`session.rs`(≤2)
- **验收命令**:`bun cargo test -p koharu-app history`、`bun cargo test -p koharu-app session`
- **证据记录**:RED / GREEN / 注入接缝位置 / commit SHA

## 卡:AR04-T03 — 损坏尾回滚与 fail-stop

- **验收标准(TASKS 原文)**:RED:部分 frame + rollback/truncate 失败后仍允许 mutation 或在坏尾追加。GREEN:无法恢复时 session fail-stop;重开只观察完整 pre/post state。
- **现状(RED-0 源码实证)**:trusted `replay_with_policy` 对截断长度/截断帧体/不可解码帧仅 `tracing::warn` + break;坏尾字节留在盘上,`History::open`(append)在其后追加新帧,新帧下次 replay 不可见(静默丢失)。无回滚、无 fail-stop。untrusted open 已走 strict(replay 错即 open 失败),保留。
- **设计**:
  1. `replay_with_policy` trusted 路径记录首个坏尾字节偏移(末好帧结束位置),返回恢复结果(epoch + Option<bad_tail_offset>);
  2. `open_inner`:trusted 且检测到坏尾 → `File::set_len(offset)` + `sync_all` 持久回滚;回滚失败 → open bail(**fail-stop**:不产出可写 session,杜绝 mutation 与坏尾后追加);
  3. 回滚成功后 `History::open` 追加位置 = 末好帧末尾;重开只观察完整帧。
  4. 回滚失败的故障注入同样走 `#[cfg(test)]` hook(T03 测试不依赖平台权限行为)。
- **RED 断言**(`bun cargo test -p koharu-app history` / `session`):
  1. `trusted_open_rolls_back_partial_tail` — 完整帧 + 半帧(截断 payload)→ open 成功,log 被截到末好帧末尾(无坏尾字节),随后 apply 追加位置正确;重开 replay 观察到且仅观察到完整帧
  2. `trusted_open_rolls_back_undecodable_tail` — 完整帧 + 完整长度但不可解码帧 → 同上
  3. `tail_rollback_failure_fail_stops_open` — 注入回滚失败 → open 返回 Err,不产出 session
  4. `untrusted_open_stays_strict_on_corrupt_tail` — 锁定测试:untrusted 遇坏尾 open 失败(现状保留)
- **目标文件**:`history.rs`、`session.rs`(≤2)
- **验收命令**:`bun cargo test -p koharu-app history`、`bun cargo test -p koharu-app session`
- **证据记录**:RED / GREEN / 回滚前后 log 字节样例 / commit SHA

---

## Lane 收口门禁(Wave 3 gate 对齐)

- `bun cargo test -p koharu-app`、`-p koharu-rpc`、`-p koharu-llm` 全绿
- `bun cargo clippy --workspace --all-targets -- -D warnings`、`bun cargo fmt --all -- --check`
- `bun cargo check --workspace --all-targets`
- `bun run check:generated`(无 OpenAPI 面变更,确认零漂移)
- 独立 scoped code-review 零发现(子代理基础设施当前故障——每次收口重试;仍故障则降级对抗性自审并落档偏差)
- history fault injection 与损坏尾回滚可重复演示(注入点 + log 字节样例)

## 风险与决策点(批准时一并确认)

- 每次 mutation 一次 scene clone:`Op::Batch` 已内建同构 clone(T01),BlobRef 为廉价引用克隆;接受此成本换崩溃安全语义
- `#[cfg(test)]` 故障注入 hook:沿用 `compact_apply_sync` 既定测试接缝模式,不进生产代码路径
- trusted 打开遇坏尾从"静默容忍+坏尾后追加"改为"持久回滚,回滚失败则 fail-stop":行为变化即本卡验收要求
- strict(untrusted)语义不变;`replay` 公开 API 签名保持(内部转 `replay_with_policy`)
