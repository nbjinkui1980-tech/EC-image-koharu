# Lane 执行合同:L-AR06 — Job 生命周期(有界注册表 + 统一读取 + 任务槽 admission)

- 状态:**Phase 3 一次批准已覆盖(2026-08-13 授予)**;LOOP-3 本地认领已登记
- 认领基线:Phase 3 起点 main `b68f123e`;执行分支 `audit-remediation-phase3`(认领时 tip `d46facce`)
- 提交/回滚单元:T01/T02/T03/T04 各一个单卡 commit(可独立 revert);lane 收口 docs(evidence) 单独一个 commit
- 前置依赖:AR13-T01 ✅(`88781c76`);AR06-T05 不在本 lane(等 L-AR05-ARCHIVE,维持 🔴)
- 执行环境偏差(继承):子代理模型映射故障未修(11 次启动失败),codegraph + 直接工具单执行器;lane 收口独立 review 重试,失败则对抗性自审并落档

## 范围文件域(域外改动禁止)

| 卡 | 允许文件 |
|---|---|
| AR06-T01 | 新 `crates/koharu-app/src/jobs.rs`、`crates/koharu-app/src/app.rs`、`crates/koharu-app/src/lib.rs`(或 shared-state 所在文件)(≤3) |
| AR06-T02 | `crates/koharu-rpc/src/bootstrap.rs`、`events.rs`、`routes/operations.rs`、`mcp/mod.rs`(≤4) |
| AR06-T03 | `crates/koharu-rpc/src/routes/pipelines.rs`、`routes/operations.rs`、`mcp/mod.rs`、`crates/koharu-app/src/app.rs`(≤4) |
| AR06-T04 | `crates/koharu-rpc/src/routes/ai.rs`、`routes/operations.rs`、`crates/koharu-app/src/app.rs`(≤3) |

## 卡:AR06-T01 — 有界 JobRegistry

- **验收标准(TASKS 原文)**:RED:257 个 completed 永久保留,或淘汰 Running。GREEN:现有状态上最小 `VecDeque` 顺序索引;completed=256,Running 永不淘汰。
- **现状(RED-0 源码实证)**:`AppSharedState.jobs: Arc<DashMap<String, JobSummary>>`(bootstrap.rs:45-47)无界;写入点 routes/ai.rs(insert ×2)、pipelines.rs(spawn/finish);无淘汰。
- **设计**:新 `koharu-app/src/jobs.rs` `BoundedJobRegistry`:`DashMap<String, JobSummary>`(O(1) lookup)+ `Mutex<VecDeque<String>>` 完成序索引;`insert` 时终态(Completed/CompletedWithErrors/Failed/Cancelled)入索引,Completed 类超 256 淘汰队首(仅当队首仍为终态;Running 永不淘汰);非终态重 insert(状态推进)不入索引。API:insert/get/iter-snapshot/remove?——保持调用面最小:insert、get、snapshot(Vec)、contains。`AppSharedState.jobs` 换型;所有写入点编译期强制对齐。
- **RED 断言**(`bun cargo test -p koharu-app jobs`):
  1. `jobs_completed_beyond_256_evicts_oldest` — 插入 257 个 completed → 最老 1 个被淘汰,registry 保持 256;当前 DashMap 无淘汰 → FAIL(编译期先行:RED-0 先落 BoundedJobRegistry 骨架使测试可编译,无淘汰逻辑)
  2. `jobs_running_never_evicted` — 256 completed + 1 Running,再插 completed → Running 仍在 → 当前 FAIL
  3. `jobs_terminal_reinsert_keeps_recency` — 同一 id 重复终态 insert 不重复占索引位
  4. 锁:`mcp_typography_get_job_rejects_unknown_id` 等既有 job 测试 GREEN 后仍过
- **目标文件**:上表 T01 行(≤3)
- **验收命令**:`bun cargo test -p koharu-app jobs`
- **证据记录**:RED / GREEN / commit SHA
- **证据(T01 收口,2026-08-14)**:
  - RED:`bun cargo test -p koharu-app jobs` → exit 101,3 failed(len 257/301/257,无淘汰实证)
  - GREEN:同命令 → `3 passed; 0 failed`;既有锁 `mcp_typography_*` 2 passed
  - 门禁:`bun cargo test -p koharu-app` 全 suite → 454 passed/0 failed;`clippy -p koharu-app -p koharu-rpc --all-targets -D warnings` → exit 0;`fmt` → exit 0
  - 域外最小牵连(类型换型编译强制,记录):`bootstrap.rs` jobs() 返回类型一行 + `mcp/mod.rs` get_job_from_registry 签名
  - 设计落地:DashMap + 完成序 VecDeque(parking_lot Mutex);终态入索引去重,Running 永不入索引(永不淘汰);get_mut 透传仅改 error 字段,不影响淘汰追踪
  - Commit:`2024de62`(5 文件,+152/-9)

## 卡:AR06-T02 — SSE/Operations/MCP 统一 registry

- **验收标准(TASKS 原文)**:RED:三入口 snapshot/lookup 在淘汰边界不一致或 unknown lookup 创建记录。GREEN:只读同一个 bounded registry。
- **设计**:三入口(SSE `snapshot_from`、operations `list_operations`、mcp `get_job_from_registry`)统一读 `BoundedJobRegistry`;snapshot 输出按完成序+插入序稳定排序(三入口同一函数产出);unknown lookup 不创建记录(既有行为,锁)。
- **RED 断言**(`bun cargo test -p koharu-rpc job_registry`):
  1. `job_registry_three_entries_consistent_at_eviction_boundary` — 填至淘汰边界,SSE snapshot/operations/mcp get_job 三方读数一致(同一 id 集合);当前各自迭代 DashMap(无序但同集)——此条现状可能凑巧 PASS,作为锁
  2. `job_registry_unknown_lookup_creates_nothing` — mcp get_job unknown → 错误且 registry 不增(锁,既有 mcp 测试已覆盖,收编)
- **目标文件**:上表 T02 行(≤4)
- **验收命令**:`bun cargo test -p koharu-rpc job_registry`
- **证据记录**:RED / GREEN / commit SHA
- **证据(T02 收口,2026-08-14)**:
  - 性质:T01 换型后三入口(SSE snapshot_from/operations list/mcp get_job)已结构性读同一 BoundedJobRegistry → 本卡为**纯锁定卡**(类 AR03 redirect 降级)
  - 锁:`job_registry_three_entries_consistent_at_eviction_boundary`(260 终态填入,三入口 id 集合一致且 == registry 现存 256,最老淘汰)+ `job_registry_unknown_lookup_creates_nothing` → `2 passed; 0 failed`
  - 生产改动:仅两行 pub(crate)(snapshot_from、get_job_from_registry 供测试访问)
  - 门禁:rpc suite 34P/0F;clippy/fmt 净
  - Commit:`533d116d`(3 文件,+93/-2)

## 卡:AR06-T03 — Pipeline 单槽 admission

- **验收标准(TASKS 原文)**:RED:HTTP/MCP 两个 pending pipeline 同时进入;终态后 slot 不释放。GREEN:project-keyed semaphore=1;RAII permit;第二个 429 + `Retry-After: 1`。
- **设计**:App 级 `pipeline_slots: DashMap<project_key, Arc<Semaphore(1)>>`(或单全局——TASKS 说 project-keyed);HTTP `start_pipeline` 与 MCP `koharu.start_pipeline` 同一 admission 入口;RAII guard 在 spawn 任务终态(含 panic)释放;未获槽 → 429 + `Retry-After: 1`。
- **RED 断言**(`bun cargo test -p koharu-rpc pipeline_admission`):
  1. `pipeline_admission_second_concurrent_gets_429` — 同一 project 并发两个 pipeline(HTTP 与 MCP 各一)→ 第二个 429 + Retry-After;当前双双进入 → FAIL
  2. `pipeline_admission_slot_released_after_finish` — 第一个终态后第三个可进;含 panic 路径释放
- **目标文件**:上表 T03 行(≤4)
- **验收命令**:`bun cargo test -p koharu-rpc pipeline_admission`
- **证据记录**:RED / GREEN / commit SHA
- **证据(T03 收口,2026-08-14)**:
  - RED:`pipeline_admission_second_concurrent_gets_429` FAIL(left 200,并发双进实证)+ permit 释放锁 PASS → exit 101
  - GREEN:`2 passed; 0 failed`(429 + Retry-After:1 ✓)
  - 设计落地:project-keyed `Semaphore(1)`(App.pipeline_slots);permit move 进任务体,终态/cancel/panic 随 future drop 释放;admission 先于 registry/event 副作用;HTTP 429 + Retry-After,MCP 经共享 spawn 路径继承;utoipa 加 429 description → openapi.json +3,generated.ts 无 body schema 不变,快照不受影响
  - 门禁:rpc suite 净;clippy/fmt 净
  - Commit:`eda1b3f3`(3 文件,+139/-5)

## 卡:AR06-T04 — AI 双槽 admission

- **验收标准(TASKS 原文)**:RED:3 个 pending AI 同时进入;cancel/error/panic 后 Job 永久 Running。GREEN:全局 semaphore=2;completion guard/RAII 清理。禁止真实模型。
- **设计**:App 级 AI `Semaphore(2)`;routes/ai.rs `start_codex_image_generation` acquire;RAII guard 在 spawned 任务完成/cancel/error/panic 时释放并把 job 置终态(现状 error 路径有终态,panic 路径无);测试用桩 AI 任务(禁真实模型)。
- **RED 断言**(`bun cargo test -p koharu-rpc ai_admission`):
  1. `ai_admission_third_concurrent_gets_429` — 3 个并发 AI 任务 → 第三个被拒;当前全进 → FAIL
  2. `ai_admission_slot_released_on_error_and_cancel` — error/cancel 后新任务可进;panic 任务(job 桩 panic)不泄漏槽且 job 置 Failed
- **目标文件**:上表 T04 行(≤3)
- **验收命令**:`bun cargo test -p koharu-rpc ai_admission`
- **证据记录**:RED / GREEN / commit SHA
- **证据(T04 收口,2026-08-14)**:
  - RED:`ai_admission_third_concurrent_gets_429` FAIL(left 200,三并发实证)+ panic 清理锁 PASS → exit 101
  - GREEN:`2 passed; 0 failed`
  - 设计落地:全局 `Semaphore(2)`(App.ai_slots);任务体 catch_unwind(futures::FutureExt),四终态(Completed/Cancelled/Failed/Panic)统一走 `finish_ai_job`(registry 终态 + JobFinished + unregister_cancel);panic 不再留永久 Running;第三个请求 429;无真实模型(admission 缝 + 清理 fn 直测)
  - 门禁:rpc 38P/app 454P 全绿;clippy/fmt 净
  - Commit:`cfc88385`(2 文件,+162/-23)

---

## Lane 收口门禁(Wave 4 gate 对齐)

- `bun cargo test -p koharu-app`、`-p koharu-rpc`、`-p koharu-llm` 全绿
- `bun cargo clippy --workspace --all-targets -- -D warnings`、`bun cargo fmt --all -- --check`
- `bun cargo check --workspace --all-targets`
- `bun run check:generated`(零漂移确认;若 429 响应模型进 OpenAPI 则重生成审查)
- `bun run test:ui`(SSE/jobsStore 读取面未改契约,确认零回归)
- 独立 scoped code-review 零发现(重试子代理;故障则对抗性自审并落档偏差)
- 淘汰边界/单槽/双槽 admission 可重复演示(测试输出)

**Lane 收口证据(2026-08-14)**:

- 门禁:`bun cargo test -p koharu-app`(454P/0F)、`-p koharu-rpc`(38P/0F)、`-p koharu-llm`(40P/0F);`clippy --workspace --all-targets -D warnings` → exit 0;`fmt --all --check` → exit 0;`check --workspace --all-targets` → exit 0;`check:generated` → 零漂移(429 注解增量已随 T03 提交);`bun run --cwd ui test` → 235 passed
- 独立 review(偏差记录):oracle 第 12 次失败(任务入队成功但运行时仍 `siliconflow/moonshotai/Kimi-K2.7-Code` 404)→ 对抗性自审(`551b3203..cfc88385`):**零 blocker/major**;1 informational——"cancelled" 字符串匹配是既有模式,finish_ai_job 集中化后被固化(非本卡引入)。逐项:淘汰去重/回转守卫/锁序无环、acquire-spawn 无 TOCTOU、permit 释放全路径覆盖、finish_ai_job 唯一调用点、无 cfg(test) 缝入生产。**五条 lane 独立 review 拖欠,待子代理修复后一并补**
- 依赖传播:无(AR06-T05 仍等 AR05-T03;AR07-T03 仍等 AR08-T02)
- 可重复演示:jobs 3 测试(淘汰边界)、pipeline_admission 2 测试(429+Retry-After)、ai_admission 2 测试(429+panic 清理)

## 风险与决策点(批准时一并确认)

- T01 换型 `AppSharedState.jobs` 牵连所有 insert/iter 调用点(ai/pipelines/mcp/operations/events)——编译期强制收齐;迭代顺序从 DashMap 无序变为完成序稳定序,UI jobsStore 按 id 键入 map 不受顺序影响
- T03 429 是新响应形态:utoipa 注解若加 429 响应体会触发 Orval 重生成;倾向仅状态码 + Retry-After 头(无 body schema),最小化 OpenAPI 面
- T04 禁真实模型:桩任务经测试专用注入(既有 ai 测试无模型路径先例——codex 任务 spawn 逻辑可注入失败)
- T05 不在本 lane;本 lane 不预先实现 import admission
