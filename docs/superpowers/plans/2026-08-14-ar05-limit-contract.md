# Lane 执行合同:L-AR05-LIMIT — 体积/批量预算(Route body limits + 批量导入预算/decode admission)

- 状态:**Phase 3 一次批准已覆盖(2026-08-13 授予)**;LOOP-3 本地认领已登记
- 认领基线:Phase 3 起点 main `b68f123e`;执行分支 `audit-remediation-phase3`(认领时 tip `b6b60c28`)
- 提交/回滚单元:T01/T06 各一个单卡 commit(可独立 revert);lane 收口 docs(evidence) 单独一个 commit
- 前置依赖:AR01-T01 ✅(含于 `AR01-T00~T03` ✅ 行);AMEND-02 已批准(2026-08-10,预算值冻结);AR05-T01 无其他在途依赖
- 执行环境偏差(继承 AR03/AR04):子代理模型映射故障仍未修(9 次启动失败),plan/explore/oracle 不可用 → codegraph + 直接工具单执行器;lane 收口独立 review 重试,失败则对抗性自审并落档

## 范围文件域(域外改动禁止)

| 卡 | 允许文件 |
|---|---|
| AR05-T01 | `crates/koharu-rpc/src/api.rs`(含其 `#[cfg(test)]` 测试) |
| AR05-T06 | `crates/koharu-rpc/src/routes/pages.rs`(含其 `#[cfg(test)]` 测试) |

不新增依赖;不启用 tower-http 新 feature;不改 workspace/Cargo.toml。

## 卡:AR05-T01 — Route-specific body limits

- **验收标准(TASKS 原文)**:RED:control、mask、archive 的 limit+1 均继承 1 GiB 全局上限并到达 handler。GREEN:1 MiB/64 MiB/512 MiB 分层;超限在 handler 前 413。
- **现状(RED-0 源码实证)**:`api.rs` 仅 `DefaultBodyLimit::max(1 GiB)` 全局 layer 挂在 `/api/v1` nest 上(行 19、93);无路由级差异。
- **分层映射(实测取证)**:
  - control = 1 MiB:全部 JSON 小 body 路由(history/config/meta/llm/ai/pipelines/projects(除 import)/pages 的 JSON 路由等)
  - mask = 64 MiB:`PUT /api/v1/pages/{id}/masks/{role}`(`body: Bytes`;role ∈ brushInpaint/segment/bubble,scene.rs:261 camelCase)
  - archive = 512 MiB:`POST /api/v1/projects/import`(`body: Bytes`,application/zip)
  - multipart `POST /api/v1/pages`:**不受 DefaultBodyLimit 约束**(axum Multipart 不读该 extension),由 T06 预算治理;binary.rs 全 GET 无 body
- **设计(决策点)**:utoipa-axum 0.2 的 `routes!` 返回 `UtoipaMethodRouter` 元组,无 per-route layer;`OpenApiRouter::layer` 仅 router 级。为守住单文件域,采用 **api.rs 内路径分类 middleware**:`middleware::from_fn` 在路由前按 path 分类,向 request extensions 插入对应 `DefaultBodyLimit`(Bytes/Json 提取器读取,后写覆盖全局 1 GiB backstop)。路由改名静默错层的风险由 body_limit 锁测试兜底(每层限值行为被测试钉死)。
- **RED 断言**(`bun cargo test -p koharu-rpc body_limit`;真实 axum::serve + 手写 HTTP/1.1 client,零新依赖;auth 用 `SecurityContext::from_secret` + Bearer,先例 koharu-rpc-security 测试):
  1. `body_limit_control_tier_413` — PATCH `/api/v1/config` 1 MiB+1 body:当前到达 handler(非 413)→ RED FAIL;GREEN 后 413
  2. `body_limit_mask_tier_413` — PUT `/api/v1/pages/{id}/masks/segment` 64 MiB+1:当前到达 handler → RED FAIL;GREEN 后 413
  3. `body_limit_archive_tier_413` — POST `/api/v1/projects/import` 512 MiB+1(流式写,小 buffer 循环):当前到达 handler → RED FAIL;GREEN 后 413(early-reject,不写满)
  4. `body_limit_at_tier_passes` — 锁:三层各发 limit 恰好值(垃圾内容)→ 到达 handler(非 413,handler 自身 4xx);小 control 请求正常 2xx
- **目标文件**:`api.rs`(≤1)
- **验收命令**:`bun cargo test -p koharu-rpc body_limit`
- **证据记录**:RED / GREEN / 各层请求-响应状态样例 / commit SHA
- **证据(T01 收口,2026-08-14)**:
  - RED:`bun cargo test -p koharu-rpc body_limit` → exit 101,`1 passed; 3 failed`(control 400/mask 400/archive 500——超限 body 全部到达 handler 实证;锁 `body_limit_at_tier_passes` PASS)
  - GREEN 首轮失败取证:中间件插错 extension 类型(`DefaultBodyLimit` 本身),提取器查找的是 crate 私有 `DefaultBodyLimitKind`——公开逐请求应用点只有 `DefaultBodyLimit::max(n).apply(&mut request)`(axum-core 0.5.6 源码取证);修正后同命令 → exit 0,`4 passed; 0 failed`
  - 门禁:`bun cargo test -p koharu-rpc` 全 suite → exit 0;`clippy -p koharu-rpc --all-targets -D warnings` → exit 0;`fmt` → exit 0
  - 机制:路径分类 middleware(`/api/v1/projects/import` → 512 MiB;`/api/v1/pages/*/masks/*` → 64 MiB;其余 → 1 MiB);全局 1 GiB layer 保留为最外 backstop;tier 中间件居内层后写覆写胜出;Multipart 不受 DefaultBodyLimit 约束(由 T06 治理)
  - 环境事件:用户在途修改 `package.json`/`scripts/dev.ts`(dev 守卫进程树终止)——非本 lane 域,未提交
  - Commit:`85bc85c1`(1 文件,+203/-0)

## 卡:AR05-T06 — 批量图片总预算与 decode admission(multipart)

- **验收标准(TASKS 原文)**:RED:257 files、512 MiB+1 encoded、1 GiB+1 decoded、3 个 pending decode 或一张损坏图产生部分 blob/scene。GREEN:按批准值在 mutation 前累计;decode semaphore=2;失败零副作用。
- **批准预算(AMEND-02)**:单次 ≤256 文件;总编码 ≤512 MiB;总解码 RGBA ≤1 GiB;同时 decode ≤2;单图 decode 上限 512 MiB 不变(`blobs.rs:23` `DECODED_RGBA_BUDGET`,不在本卡域)。
- **现状(RED-0 源码实证)**:`create_pages` 全量收集 multipart 入内存 → 自然排序 → `admit_source_image` rayon `into_par_iter` **无界并发** decode admission → blob `put_bytes` → 单次 `Op::Batch` apply。admission 已先于 mutation(零副作用骨架在位);无文件数/编码总量/解码总量任何预算,无 decode 并发上限。
- **设计**:
  - 预算常量集中:`MAX_IMPORT_FILES=256`、`MAX_IMPORT_ENCODED_BYTES=512 MiB`、`MAX_IMPORT_DECODED_RGBA_BYTES=1 GiB`、`DECODE_CONCURRENCY=2`
  - 计数 + 编码总量:multipart 收集循环内累计,超限即在读取阶段拒绝(任何 admission/blob/scene 之前),同时给收集侧内存封顶
  - 解码总量:admission 得 (w,h) 后累计 Σw·h·4 ≤1 GiB,超限在 blob 写之前拒绝
  - decode 并发:admission 移入**专用 rayon pool(`num_threads(2)`)**,decode 并发恒 ≤2
  - 零副作用:所有预算与 admission 拒绝均早于 blob 写与 scene mutation(既有顺序已满足,本卡在其上插门)
  - **RED-0 脚手架(先例 AR03-T02 stub)**:先落常量 + `#[cfg(test)]` 预算覆盖缝 + decode in-flight  gauge(仅观测,不执行),使测试可编译
- **RED 断言**(`bun cargo test -p koharu-rpc page_import_budget`;复用 pages.rs 既有 multipart harness/`encoded_image`/seeded_session):
  1. `page_import_budget_rejects_over_file_count` — 覆盖 max_files=2,发 3 张合法 PNG:当前全部导入 → FAIL
  2. `page_import_budget_rejects_over_encoded_bytes` — 覆盖 encoded=100B,两文件合计 101B+:当前成功 → FAIL
  3. `page_import_budget_rejects_over_decoded_rgba` — 覆盖 decoded 小预算,合法小图但 ΣRGBA 超预算:当前成功 → FAIL
  4. `page_import_budget_decode_concurrency_two` — 8 张图,gauge 观测最大在飞 decode:当前 rayon 无界(>2)→ FAIL;GREEN 后 ≤2
  5. `page_import_budget_corrupt_image_zero_side_effects` — 锁:1 张损坏图混入 → 报错且 scene pages 与 blob 目录均不变(现状即满足)
  6. `page_import_budget_default_small_import_ok` — 锁:默认预算下小批量导入正常
- **目标文件**:`routes/pages.rs`(≤1)
- **验收命令**:`bun cargo test -p koharu-rpc page_import_budget`
- **证据记录**:RED-0 / RED / GREEN / gauge 观测样例 / commit SHA
- **证据(T06 收口,2026-08-14)**:
  - RED-0 脚手架:预算常量 + thread-local 覆盖缝 + decode gauge(仅观测)先落,测试可编译
  - RED:`bun cargo test -p koharu-rpc page_import_budget` → exit 101,`2 passed; 4 failed`(三预算未拒 FAIL + gauge 观测并发 >2 FAIL;两锁 PASS)
  - GREEN:同命令 → exit 0,`6 passed; 0 failed`
  - 门禁:`bun cargo test -p koharu-rpc` 全 suite → exit 0;`clippy -p koharu-rpc --all-targets -D warnings` → exit 0;`fmt` → exit 0
  - 设计落地:计数+编码总量在收集循环累计(读取阶段即拒);解码总量 admission 后、blob 写前拒绝;专用 rayon pool(`num_threads(2)`,OnceLock 静态)钉死 decode 并发;413 + 稳定 `import budget exceeded` 前缀;覆盖缝/gauge 均 cfg(test)
  - gauge 观测样例:RED 无界 rayon 观测 in-flight max >2 → GREEN 池化后 ≤2
  - Commit:`2d74327a`(1 文件,+310/-13)

---

## Lane 收口门禁(Wave 4 gate 对齐)

- `bun cargo test -p koharu-rpc`、`-p koharu-app`、`-p koharu-llm` 全绿
- `bun cargo clippy --workspace --all-targets -- -D warnings`、`bun cargo fmt --all -- --check`
- `bun cargo check --workspace --all-targets`
- `bun run check:generated`(无 OpenAPI 面变更,确认零漂移)
- 独立 scoped code-review 零发现(重试子代理;故障则对抗性自审并落档偏差)
- body 分层 413 与批量预算拒绝可重复演示(请求-响应样例 / gauge 观测)
- **环境前置**:确认无 `tauri dev` 会话持有共享 target 租约(AR04 收口曾因此阻塞 CHECK/GEN)

**Lane 收口证据(2026-08-14)**:

- 门禁:`bun cargo test -p koharu-rpc -p koharu-app -p koharu-llm` → exit 0;`clippy --workspace --all-targets -D warnings` → exit 0;`fmt --all --check` → exit 0;`check --workspace --all-targets` → exit 0;`bun run check:generated` → exit 0(零漂移,本 lane 无 OpenAPI 面变更——分层/预算不改 schema)
- 独立 review(偏差记录):oracle 第 10 次启动失败 → 对抗性自审(`3df24940..2d74327a` file:line 取证):**零 blocker/major**;**1 minor 接受**:multipart 单字段在编码总量拒绝前会完整缓冲(TASKS"mutation 前累计"语义已满足——拒绝在任何 mutation 之前;真流式中段拒绝需 field.chunk() 重构,超 TASKS 范围);逐项核查:路径分类无逃逸(尾斜杠/大小写/无路由均 404;multipart 不受 DefaultBodyLimit 约束由 T06 接管并有大预算测试实证)、u64 溢出无、零副作用经 snapshot 相等性断言证明、decode 池无线程死锁面。**与 AR03/AR04 拖欠项一并,待子代理修复后补独立 review**
- 依赖传播:无(无卡依赖 AR05-T06)
- 可重复演示:body_limit 4 测试(真实 server+手写 client)+ page_import_budget 6 测试(gauge 观测 RED >2 → GREEN ≤2)

## 风险与决策点(批准时一并确认)

- T01 路径分类 middleware:路由改名会静默错层 —— 由 4 个分层锁测试钉死行为,改名即测试红
- T01 大 body 测试:archive 层 RED 需流式写 512 MiB+1(localhost 秒级,1 MiB 循环 buffer,无大分配);GREEN 后 early-413 不写满
- T06 RED-0 脚手架(常量+覆盖缝+gauge 无执行)使 RED 可编译运行;gauge 先行是并发 RED 的唯一诚实测法
- T06 decode 并发用专用 rayon pool(num_threads=2)而非新 semaphore 依赖;blob 写保持在全局 pool(非 decode,不占预算语义)
- T06 blob `put_bytes` 中途 IO 失败产生孤儿 CAS blob 不在 RED 列举场景内,本卡不处理(无 quota framework,AMEND-02 边界)
