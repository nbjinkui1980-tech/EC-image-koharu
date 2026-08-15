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
- **证据(T02 收口,2026-08-14)**:
  - RED:`bun cargo test -p koharu-app history` → exit 101,`6 passed; 3 failed`(apply/undo/redo 三个 `*_leaves_no_trace` 均 FAIL 于 "scene must not change"——先改内存后持久化实证;锁测试 `apply_success_publishes_after_durable_frame` PASS)
  - GREEN:同命令 → exit 0,`9 passed; 0 failed`;`bun cargo test -p koharu-app session` → exit 0,`14 passed; 0 failed`
  - 门禁:`bun cargo test -p koharu-app` 全 suite → exit 0;`clippy -p koharu-app --all-targets -D warnings` → exit 0;`fmt -p koharu-app --check` → exit 0
  - 注入接缝:`History::fail_next_frame_write`(cfg(test) 字段,write_frame 写盘前 bail;先例 `compact_apply_sync`)
  - 设计落地:候选态先行(scratch scene apply → durable frame → 一次性发布);undo/redo 改 peek(`back()/last().cloned()`)使失败时栈不变;`write_frame` 签名改为携带目标 epoch
  - Commit:`141f19ed`(1 文件,+135/-10;`session.rs` 未动——发布语义全部在 History 内)

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
- **证据(T03 收口,2026-08-14)**:
  - RED:`bun cargo test -p koharu-app session` → exit 101,`15 passed; 3 failed`(partial/undecodable 坏尾未截断 FAIL、open 未 fail-stop FAIL;锁 `untrusted_open_stays_strict_on_corrupt_tail` PASS)
  - GREEN:同命令 → exit 0,`18 passed; 0 failed`;`bun cargo test -p koharu-app history` → exit 0,`9 passed; 0 failed`
  - 门禁:`bun cargo test -p koharu-app` 全 suite → exit 0;`clippy -p koharu-app --all-targets -D warnings` → exit 0;`fmt -p koharu-app --check` → exit 0
  - 注入接缝:thread-local `fail_next_tail_rollback`(cfg(test),防并行测试互窃注入;消费式 `take_tail_rollback_fault`;测试各自 `clear_tail_rollback_fault` 开局)
  - 设计落地:`ReplayOutcome{epoch, bad_tail_offset}`;`rollback_corrupt_tail` = `set_len(good_end)`+`sync_all`,失败即 open bail(fail-stop);`replay` 公开签名不变;strict(untrusted)语义不变
  - 回滚前后 log 字节样例:`[frame1 完整] + [len=64][8B 垃圾]` → open 后 log 回到 frame1 原长;apply 追加位置 = 末好帧末尾;重开观察到且仅观察到完整帧(name=second,epoch 2)
  - Commit:`eb8b9ccc`(2 文件,+185/-6)

---

## Lane 收口门禁(Wave 3 gate 对齐)

- `bun cargo test -p koharu-app`、`-p koharu-rpc`、`-p koharu-llm` 全绿
- `bun cargo clippy --workspace --all-targets -- -D warnings`、`bun cargo fmt --all -- --check`
- `bun cargo check --workspace --all-targets`
- `bun run check:generated`(无 OpenAPI 面变更,确认零漂移)
- 独立 scoped code-review 零发现(子代理基础设施当前故障——每次收口重试;仍故障则降级对抗性自审并落档偏差)
- history fault injection 与损坏尾回滚可重复演示(注入点 + log 字节样例)

**Lane 收口证据(2026-08-14)**:

- 门禁:`bun cargo test -p koharu-app -p koharu-rpc -p koharu-llm` → exit 0;`clippy --workspace --all-targets -D warnings` → exit 0;`fmt --all --check` → exit 0;`check --workspace --all-targets` → exit 0;`bun run check:generated` → exit 0(零漂移,本 lane 无 OpenAPI 面)
- 环境事件:CHECK/GEN 曾被一个 `tauri dev` 会话持有的共享 target 租约阻塞(00:44 启动,非本 lane 工作);用户确认并关闭后门禁完成——非 lane 缺陷,仅记录
- 独立 review(偏差记录):oracle 第 8 次启动失败(`siliconflow/moonshotai/Kimi-K2.7-Code` 子代理映射仍未修)→ 降级为对抗性自审(同一结构化清单,`31ec8c62..eb8b9ccc` file:line 取证):**零 blocker/major/minor**;1 informational——真实 mid-write io 失败可能在盘上留部分帧尾(内存零痕迹),由 T03 回滚在下次 open 接管,by design。**与 AR03 拖欠项一并,待子代理修复后补独立 review**
- 依赖传播:AR05-T04 ←AR04-T03 唯一依赖已满足 → 🔴→🟡;AR05-T03 仍等 AR05-T02(🟡 未完成)维持 🔴
- 可重复演示:T02 `fail_next_frame_write` / T03 `fail_next_tail_rollback` 注入点 + 回滚前后 log 字节样例(卡级证据区)

## 风险与决策点(批准时一并确认)

- 每次 mutation 一次 scene clone:`Op::Batch` 已内建同构 clone(T01),BlobRef 为廉价引用克隆;接受此成本换崩溃安全语义
- `#[cfg(test)]` 故障注入 hook:沿用 `compact_apply_sync` 既定测试接缝模式,不进生产代码路径
- trusted 打开遇坏尾从"静默容忍+坏尾后追加"改为"持久回滚,回滚失败则 fail-stop":行为变化即本卡验收要求
- strict(untrusted)语义不变;`replay` 公开 API 签名保持(内部转 `replay_with_policy`)
