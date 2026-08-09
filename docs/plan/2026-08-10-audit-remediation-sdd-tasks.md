# 全项目审查修复 SDD Phase 3 TASKS

**状态：APPROVED — 2026-08-10；Phase 3 TASKS 与规格补丁已批准；未授权 IMPLEMENT。**

**规格：** `docs/plan/2026-08-10-audit-remediation-sdd-spec.md`
**计划：** `docs/plan/2026-08-10-audit-remediation-sdd-plan.md`
**受控 checkpoint：** `f7e24dd6`
**分支：** `codex/audit-remediation-sdd`

本文件只把已批准计划拆成可执行任务卡。除静态门禁任务外，每张卡都必须先得到目标断言 RED，再写最小实现，最后得到同字节测试 GREEN。本文获批前不得修改产品代码、依赖、CI 或运行配置。

## 1. Phase 3 已批准规格补丁

### AMEND-01：删除 `/pages/from-paths`，复用现有安全路径

批准内容：删除只接收任意绝对路径的 `/pages/from-paths` API。Tauri dialog 选择后，复用现有 `@tauri-apps/plugin-fs.readFile` 临时 scope 读取 bytes，再走已有 multipart `POST /pages`；Web 路径不变。

理由：仓库已经有 `readTauriFiles()` 和 multipart upload。删除重复的后端文件读取入口比新增一次性 capability 签发、存储、消费和过期协议更小，也直接消除 Axum 控制面对本机路径的读取能力。

状态：用户已于 2026-08-10 批准并同步到 SPEC/PLAN。

### AMEND-02：冻结批量图片预算

批准内容：

| Resource | Proposed limit |
| --- | ---: |
| 单次导入文件数 | 256 |
| 单次导入总编码 bytes | 512 MiB |
| 单次导入总解码 RGBA bytes | 1 GiB |
| 同时 decode 数 | 2 |

现有单图 decode 上限 512 MiB 保持不变。超限应在任何 blob/scene mutation 前失败。状态：用户已于 2026-08-10 批准并同步到 SPEC/PLAN。

## 2. 全局执行协议

每张任务卡固定执行：

1. `RED-0`：测试 harness 编译/启动成功。
2. `RED-1`：只因该卡目标断言失败；编译、fixture、端口或环境失败不算 RED。
3. `GREEN-1`：同一测试字节通过，不得弱化、skip、retry 或改快照接受旧行为。
4. `GREEN-2`：相邻模块 suite 通过。
5. `WAVE-GREEN`：唯一 verifier 串行执行本波完整门禁。

静态格式、Clippy、audit、build 和 policy 卡以对应失败命令作为可执行 RED，不新增无意义测试。

通用停止条件：

- 单卡预计超过 5 个文件，先回 TASKS 拆分。
- 需要改变认证方案、批准预算、项目/history/API 格式、发布主体或 G005，回 SPEC。
- 需要批准范围外的新依赖、通用 service/queue/policy 框架，回 PLAN。
- 拒绝路径仍有部分 scene/history/file/job、根外读取或 secret 泄漏，当前卡不得 GREEN。
- 并行 Rust 卡使用独立 `CARGO_TARGET_DIR=/tmp/koharu-sdd-<task>`；format/audit/lockfile/Orval/Next build 只允许单一 owner 串行执行。

## 3. Wave 1 — 依赖表面与现有门禁

### AR14-T04 — Rust advisory 定向升级

- 依赖：无；独占 `Cargo.toml`、`Cargo.lock`。
- 文件：`Cargo.toml`、`Cargo.lock`。
- RED：`bun cargo audit` 报 quick-xml advisory；`bun cargo tree -i quick-xml@0.39.4` 可达 Tauri/plist。
- GREEN：只定向升级 Tauri/plist 链，使 `quick-xml >= 0.41`；不批量升级。
- 验证：`bun cargo audit`；`bun cargo check --workspace --all-targets`；三平台 Tauri build。

### AR14-T05 — Next/sharp 直接告警定向升级

- 依赖：AR14-T04 完成后，独占 lockfile/build。
- 文件：`ui/package.json`、`bun.lock`。
- RED：`bun audit --registry https://registry.npmjs.org` 中 reachable direct High/Critical。
- GREEN：只升级 Next/sharp 兼容修复版本。
- 验证：官方 registry audit、`bun run test:ui`、`bun run --cwd ui build`。

### AR14-T01 — 清零 5 个 Clippy 错误

- 文件：`crates/koharu-app/src/pipeline/engines/ctd_segment.rs`、`renderer/mod.rs`、`support/mod.rs`、`crates/koharu-app/src/session.rs`。
- RED：当前 `type_complexity`、`unnecessary_map_or`、`too_many_arguments`、`redundant_closure`、`collapsible_if`。
- GREEN：最小 type alias/参数聚合或直接建议替换；不重构 pipeline。
- 验证：`CARGO_TARGET_DIR=/tmp/koharu-sdd-ar14-t01 bun cargo clippy --workspace --all-targets -- -D warnings`。

### AR14-T02 — UI format 基线

- 文件：`ui/components/panels/RenderControlsPanel.tsx`、两份 locale JSON、`ui/tests/setup.ts`。
- RED：`bun run format:check` 当前只报告这 4 个文件。
- GREEN：只运行 Oxfmt 的文件级机械格式化。
- 验证：`bun run format:check`、`git diff --check`。

### AR14-T07 — 默认 Next/Turbopack 标准布局

- 依赖：AR14-T05。
- 文件：优先零文件；若确认配置缺陷，限 `ui/next.config.ts`、`ui/package.json`、`bun.lock`。
- RED：标准 checkout + frozen install 的 `bun run --cwd ui build` 失败。
- GREEN：修复标准 root/config；不得把 webpack 设为永久替代。
- 验证：Linux CI、macOS/Windows 标准布局；外置 `node_modules` 仅额外 smoke。

### Wave 1 gate

`cargo audit`、workspace check/clippy、UI audit/format/test/default build 全部通过；Tauri/Next 锁版本冻结后才进入依赖其 API 的任务。

## 4. Wave 2 — 共享不变量

### AR13-T01 — 5xx 脱敏与 Sentry PII

- 文件：`crates/koharu-rpc/src/error.rs`、`crates/koharu/src/sentry.rs`、`ui/instrumentation-client.ts`、`ui/components/AppErrorBoundary.tsx`、新 `scripts/sentry-policy.test.ts`。
- RED：内部错误含绝对路径、anyhow cause、provider body、假 secret；HTTP 5xx/Sentry payload 泄漏原文。
- GREEN：5xx 只返回稳定有界消息；内部 cause 只进脱敏 tracing；`send_default_pii=false`。
- 验证：`bun cargo test -p koharu-rpc api_error`、`bun cargo test -p koharu sentry`、`bun test scripts/sentry-policy.test.ts`。

### AR02-T01 — BlobRef parse/Serde 唯一不变量

- 文件：`crates/koharu-core/src/blob.rs`。
- RED：空、63/65 位、uppercase、Unicode、斜杠、反斜杠、点段、绝对路径均能构造或反序列化。
- GREEN：只接受精确 64 位 `[0-9a-f]`；Serde 复用同一 parser。
- 验证：`CARGO_TARGET_DIR=/tmp/koharu-sdd-ar02-t01 bun cargo test -p koharu-core blob`。

### AR02-T02 — BlobStore containment 纵深防御

- 依赖：AR02-T01。
- 文件：`crates/koharu-app/src/blobs.rs`。
- RED：绕过 Serde 的非法 BlobRef 可生成根外路径或读取根外哨兵。
- GREEN：路径生成返回 `Result<PathBuf>` 并证明是 blob root 下的预期两级 hash 路径。
- 验证：`CARGO_TARGET_DIR=/tmp/koharu-sdd-ar02-t02 bun cargo test -p koharu-app blobs`。

### AR02-T03 — HTTP encoded traversal

- 依赖：AR02-T01、T02。
- 文件：`crates/koharu-rpc/src/binary.rs`、现有 `tests/integration-tests/tests/binary.rs`。
- RED：真实 Axum URI `aa%2Fetc%2Fpasswd` 等 percent-decoded separator 可读根外哨兵。
- GREEN：非法格式稳定 4xx；合法不存在仍 404。
- 验证：`bun cargo test -p integration-tests binary` 或该仓库等价 binary 定向命令。

### AR02-T04 — 恶意 Scene/history/archive BlobRef

- 依赖：AR02-T01、T02。
- 文件：`crates/koharu-app/src/session.rs`、`crates/koharu-rpc/src/routes/projects.rs`。
- RED：恶意 `.khr` 或 history Op 含非法 BlobRef 后成为当前项目或触发读取。
- GREEN：反序列化失败，staging/final 清理，当前 session 不变。
- 验证：`bun cargo test -p koharu-app session`、`bun cargo test -p koharu-rpc import`。

### AR02-T05 — BlobRef OpenAPI/Orval 同步

- 依赖：AR02-T01。
- 文件：OpenAPI source 注解、`ui/openapi.json`、`ui/lib/api/schemas/blobRef.ts`；生成文件只由命令产生。
- RED：`bun run check:generated` 显示 BlobRef schema 缺长度/pattern。
- GREEN：生成 schema 表达 64 位小写 hex。
- 验证：`bun run check:generated`。

### AR04-T01 — 混合/嵌套 Batch 原子

- 文件：`crates/koharu-core/src/op.rs`。
- RED：Batch 前段成功、后段失败后 Scene bytes 或 `prev_*` 改变。
- GREEN：在 scratch Scene/ops 顺序执行，全部成功后一次发布；保留 inverse 和单 undo 粒度。
- 验证：`CARGO_TARGET_DIR=/tmp/koharu-sdd-ar04-t01 bun cargo test -p koharu-core batch`。

### Wave 2 gate

Core/App/RPC 定向 suite、fmt/check/clippy 通过；两层 BlobRef 拒绝和 Batch 失败原子性均有 RED/GREEN 证据。

## 5. Wave 3 — 网络与持久化边界

### AR01-T00 — 已批准认证直接依赖边

- 文件：`Cargo.toml`、`crates/koharu/Cargo.toml`、`crates/koharu-rpc/Cargo.toml`、`Cargo.lock`。
- RED：认证代码需要 OS CSPRNG 和 URL-safe token encoding，但目标 crate 无直接依赖。
- GREEN：只增加已批准 `getrandom 0.3.4`、`base64 0.22` 直接边；不引入新版本。
- 验证：`bun cargo check -p koharu -p koharu-rpc --all-targets`、`bun cargo tree -p koharu`。

### AR01-T01 — REST/SSE/Binary 统一认证层

- 依赖：AR13-T01、AR01-T00。
- 文件：新 `crates/koharu-rpc/src/security.rs`、`server.rs`、`api.rs`、`lib.rs`。
- RED：bootstrap API、普通 API、SSE、blob、download 的无/错 credential 请求到达 handler。
- GREEN：immutable `SecurityContext`；Host→Origin→Auth→Readiness→handler；cookie/Bearer 正确通过。
- 验证：`CARGO_TARGET_DIR=/tmp/koharu-sdd-ar01-t01 bun cargo test -p koharu-rpc auth`。

### AR01-T02 — MCP Bearer 与 route ordering

- 依赖：AR01-T01。
- 文件：`crates/koharu-rpc/src/mcp/mod.rs`、`server.rs`、`security.rs`。
- RED：无/错 Bearer 可创建 MCP session 或产生副作用。
- GREEN：MCP session 创建前统一 Bearer 校验；cookie 不授权 MCP。
- 验证：`bun cargo test -p koharu-rpc mcp_auth`。

### AR01-T03 — CORS 与 Host allowlist

- 依赖：AR01-T01。
- 文件：`crates/koharu-rpc/src/security.rs`、`server.rs`。
- RED：evil/`null` origin、origin reflection、伪 Host 或 wildcard CORS 被接受。
- GREEN：生产同源；开发冻结 localhost；不允许 origin 无 ACAO；Host 在 handler 前拒绝。
- 验证：`bun cargo test -p koharu-rpc origin_host`。

### AR03-T01 — Provider URL 与 authority 规范化

- 依赖：AR13-T01。
- 文件：`crates/koharu-llm/src/providers/openai_compatible.rs`、`providers/mod.rs`。
- RED：非 HTTP(S)、userinfo、fragment 可联网；effective port/scheme/host 比较错误。
- GREEN：复用 `url` 的唯一 authority 比较；同 authority path 变化相等。
- 验证：`bun cargo test -p koharu-llm authority`。

### AR03-T02 — Config authority 冲突

- 依赖：AR03-T01。
- 文件：`crates/koharu-app/src/config.rs`、`crates/koharu-rpc/src/routes/config.rs`。
- RED：已有 secret 的 provider 改 authority 且未提供新 secret 时旧 secret 被复用或配置部分更新。
- GREEN：mutation 前返回 409；显式新 secret 后才提交；同 authority path 保留 secret。
- 验证：`bun cargo test -p koharu-app provider_authority`、`bun cargo test -p koharu-rpc config_conflict`。

### AR03-T03 — Redirect 与 provider 错误脱敏

- 依赖：AR03-T01、AR13-T01。
- 文件：`openai_compatible.rs`、`crates/koharu-app/src/llm.rs`、`crates/koharu-rpc/src/routes/llm.rs`。
- RED：mock A 跨 authority redirect 到 B，B 收到 Authorization；大 body/secret 出现在公开错误。
- GREEN：provider 专用 redirect policy 去除敏感 header；只保留有界摘要。
- 验证：`bun cargo test -p koharu-llm redirect`、App/RPC provider error tests。

### AR04-T02 — Apply/Undo/Redo durable commit

- 依赖：AR04-T01。
- 文件：`crates/koharu-app/src/history.rs`、`session.rs`。
- RED：encode/write/flush/sync 失败后 scene、epoch、两栈、事件或 log length 改变。
- GREEN：候选状态完成后先 durable frame，再一次发布内存状态。
- 验证：`bun cargo test -p koharu-app history`、`bun cargo test -p koharu-app session`。

### AR04-T03 — 损坏尾回滚与 fail-stop

- 依赖：AR04-T02。
- 文件：`crates/koharu-app/src/history.rs`、`session.rs`。
- RED：部分 frame + rollback/truncate 失败后仍允许 mutation 或在坏尾追加。
- GREEN：无法恢复时 session fail-stop；重开只观察完整 pre/post state。
- 验证：`bun cargo test -p koharu-app history`。

### Wave 3 gate

App/RPC/LLM suite 全绿；REST/SSE/MCP credential matrix、provider redirect、history fault injection 均可重复。

## 6. Wave 4 — 启动集成、预算与任务生命周期

### AR01-T04 — Desktop secret 与一次性 session exchange

- 依赖：AR01-T01、T03。
- 文件：新 `crates/koharu/src/security.rs`、`app.rs`、`crates/koharu-rpc/src/security.rs`、`server.rs`。
- RED：外部浏览器加载 HTML 可匿名获得 cookie，或 Desktop 必须人工输入 token。
- GREEN：Tauri-only IPC 提供一次性 bootstrap proof；受 Bearer 保护的 exchange 返回 `HttpOnly; SameSite=Strict; Path=/` cookie；proof 只用一次。
- 验证：`bun cargo test -p koharu desktop_auth`、`bun cargo test -p koharu-rpc desktop_cookie`。

### AR01-T04B — UI session bootstrap

- 依赖：AR01-T04。
- 文件：新 `ui/lib/auth.ts`、新 `ui/components/AuthBootstrap.tsx`、`ui/app/providers.tsx`、新对应 test。
- RED：Desktop UI 未交换 session 就开始 API/SSE；Headless browser 无法显式输入 token，或 token 被写入 URL/localStorage。
- GREEN：Desktop 经 Tauri IPC 取一次性 proof；Headless token 只驻留内存并交换 HttpOnly cookie；bootstrap 完成后才挂载 app children。
- 验证：AuthBootstrap Vitest；检查 URL、storage 和错误输出不含 token。

### AR01-T04C — 统一 fetch/SSE credential 行为

- 依赖：AR01-T04B。
- 文件：`ui/lib/api/fetch.ts`、`ui/lib/events.ts`、新 fetch test、现有 `ui/tests/lib/events.test.ts`。
- RED：API 与 SSE 在 session cookie 下行为不一致，或 reconnect 泄漏 token 到 URL。
- GREEN：两者只使用同源 cookie；automation Bearer 不注入浏览器 bundle；401 明确返回 bootstrap 层。
- 验证：fetch/events Vitest、Desktop/headless browser smoke。

### AR01-T05 — Headless/remote fail closed

- 依赖：AR01-T02、T04。
- 文件：`crates/koharu/src/cli.rs`、`app.rs`、`security.rs`、相邻 tests。
- RED：Headless 缺 secret、remote 缺 Host allowlist 仍监听。
- GREEN：env/secret file 显式注入；缺项非零退出且端口未监听；不内建 TLS。
- 验证：`bun cargo test -p koharu headless_security`。

### AR01-T06 — Docker auth smoke

- 依赖：AR01-T05；不得与 AR10-T03 同时写 Dockerfile。
- 文件：`Dockerfile`、现有运行文档、`scripts/supply-chain-policy.test.ts`。
- RED：默认容器匿名可用或未提供 secret 仍启动。
- GREEN：容器要求 secret + Host allowlist；文档不把 token 放 URL。
- 验证：本地 docker build/run，REST/SSE/MCP 带假 Bearer；不 push。

### AR05-T01 — Route-specific body limits

- 依赖：AR01-T01。
- 文件：`crates/koharu-rpc/src/api.rs`、相邻 router tests。
- RED：control、mask、archive 的 limit+1 均继承 1 GiB 全局上限并到达 handler。
- GREEN：1 MiB/64 MiB/512 MiB 分层；超限在 handler 前 413。
- 验证：`bun cargo test -p koharu-rpc body_limit`。

### AR05-T02 — Archive 实际读取预算

- 文件：`crates/koharu-app/src/archive.rs`。
- RED：entry、单项、总展开、100:1、伪造 size 的 limit+1 未拒绝或先大分配。
- GREEN：按实际读取 bytes 流式写 staging；批准预算常量；不建 quota framework。
- 验证：`bun cargo test -p koharu-app archive`。

### AR05-T03 — Import 原子发布与 cleanup

- 依赖：AR02-T04、AR04-T03、AR05-T02。
- 文件：`crates/koharu-rpc/src/routes/projects.rs`、`crates/koharu-app/src/archive.rs`、`session.rs`。
- RED：超限/损坏/非法 BlobRef 导入留下 staging/final 或改变当前项目。
- GREEN：所有验证完成后才 publish/open；失败统一 cleanup。
- 验证：`bun cargo test -p koharu-rpc import`。

### AR05-T04 — History frame 分配前上限

- 依赖：AR04-T03。
- 文件：`crates/koharu-app/src/history.rs`。
- RED：16 MiB+1 或 `u32::MAX` 长度头触发大分配或被当截断尾忽略。
- GREEN：分配前拒绝；完整超限 frame 是 corruption/error。
- 验证：`bun cargo test -p koharu-app history_frame_limit`。

### AR05-T05A — Tauri picker 统一返回 File

- 依赖：AMEND-01（已批准）。
- 文件：`ui/lib/io/openFiles.ts`、`ui/lib/io/pagesIo.ts`、对应 UI test。
- RED：Tauri picker 返回路径并调用 raw path API。
- GREEN：dialog 后用已有 `readTauriFiles`，与 Web 一样返回 `File[]` 并走 multipart。
- 验证：对应 UI test；三平台 dialog 临时 scope smoke。

### AR05-T05B — 删除后端 from-paths API

- 依赖：AR05-T05A、AMEND-01（已批准）。
- 文件：`crates/koharu-rpc/src/routes/pages.rs`、OpenAPI snapshot、生成物（单独生成命令）。
- RED：policy/OpenAPI 仍暴露 `/pages/from-paths`。
- GREEN：删除 route、request schema 和 raw filesystem read；multipart 保持。
- 验证：RPC pages tests、`bun run check:generated`、全仓 `rg 'from-paths|uploadPagesByPaths'` 无运行时调用。

### AR05-T06 — 批量图片总预算与 decode admission

- 依赖：AMEND-02（已批准）、AR05-T01；只覆盖 multipart。
- 文件：`crates/koharu-rpc/src/routes/pages.rs`、相邻 tests。
- RED：257 files、512 MiB+1 encoded、1 GiB+1 decoded、3 个 pending decode 或一张损坏图产生部分 blob/scene。
- GREEN：按批准值在 mutation 前累计；decode semaphore=2；失败零副作用。
- 验证：`bun cargo test -p koharu-rpc page_import_budget`。

### AR06-T01 — 有界 JobRegistry

- 依赖：AR13-T01。
- 文件：可选新 `crates/koharu-app/src/jobs.rs`、`app.rs`、`lib.rs`。
- RED：257 个 completed 永久保留，或淘汰 Running。
- GREEN：现有状态上最小 `VecDeque` 顺序索引；completed=256，Running 永不淘汰。
- 验证：`bun cargo test -p koharu-app jobs`。

### AR06-T02 — SSE/Operations/MCP 统一 registry

- 依赖：AR06-T01。
- 文件：`crates/koharu-rpc/src/bootstrap.rs`、`events.rs`、`routes/operations.rs`、`mcp/mod.rs`。
- RED：三入口 snapshot/lookup 在淘汰边界不一致或 unknown lookup 创建记录。
- GREEN：只读同一个 bounded registry。
- 验证：`bun cargo test -p koharu-rpc job_registry`。

### AR06-T03 — Pipeline 单槽 admission

- 依赖：AR06-T01、T02。
- 文件：`routes/pipelines.rs`、`routes/operations.rs`、`mcp/mod.rs`、App job state。
- RED：HTTP/MCP 两个 pending pipeline 同时进入；终态后 slot 不释放。
- GREEN：project-keyed semaphore=1；RAII permit；第二个 429 + `Retry-After: 1`。
- 验证：`bun cargo test -p koharu-rpc pipeline_admission`。

### AR06-T04 — AI 双槽 admission

- 依赖：AR06-T01、T02。
- 文件：`routes/ai.rs`、`routes/operations.rs`、App job state。
- RED：3 个 pending AI 同时进入；cancel/error/panic 后 Job 永久 Running。
- GREEN：全局 semaphore=2；completion guard/RAII 清理。
- 验证：`bun cargo test -p koharu-rpc ai_admission`；禁止真实模型。

### AR06-T05 — Bulk import 单槽 admission

- 依赖：AR05-T03、AR06-T01。
- 文件：`routes/projects.rs`、App admission state、相邻 tests。
- RED：两个 pending archive import 同时进入，或超预算/panic 后 slot 不释放。
- GREEN：读取大 body 前 admission=1；所有路径释放并 cleanup。
- 验证：`bun cargo test -p koharu-rpc import_admission`。

### AR13-T02 — 破坏性 Project ID 精确匹配

- 依赖：AR13-T01。
- 文件：`crates/koharu-app/src/projects.rs`、`crates/koharu-rpc/src/routes/projects.rs`。
- RED：`my-project!`、大小写、空格、encoded separator 可作用于 `my-project`。
- GREEN：create display name 仍 slugify；open/delete 只接受精确 canonical ID。
- 验证：App projects + RPC project tests。

### AR13-T03 — Mask 复用 generated API

- 依赖：AR01-T03。
- 文件：`ui/hooks/useMaskDrawing.ts`、`ui/tests/hooks/useMaskDrawing.test.tsx`。
- RED：mask 仍走重复 raw fetch。
- GREEN：复用 generated `putMask`；错误与 scene invalidation 保持。
- 验证：对应 Vitest。

### AR13-T04 — Export generated API 保留 filename

- 依赖：AR01-T03。
- 文件：`ui/lib/io/scene.ts`、`ui/lib/api/fetch.ts`、Orval config、`ui/tests/lib/io/scene.test.ts`。
- RED：generated 调用丢 `Content-Disposition` 或 blob type。
- GREEN：由生成器保留完整 Response；不手改 `generated.ts`。
- 验证：scene test、`bun run check:generated`。
- 停止：Orval 无法保留 headers 时回 SPEC，不得静默丢 filename。

### Wave 4 gate

两条 Critical 利用链、history/archive/image 预算、Desktop/headless/Docker auth、Job admission/retention 全部绿色后才能完成本波。

## 7. Wave 5 — 独立领域 lane

### AR07-T01 — Axum HTML CSP 与 Tauri CSP

- 依赖：AR14-T04。
- 文件：`crates/koharu-rpc/src/server.rs`、`crates/koharu/tauri.conf.json`、新 `scripts/tauri-security-config.test.ts`。
- RED：HTML 无 CSP，Tauri `csp=null`。
- GREEN：规格冻结指令由实际 HTML 响应发出，配置同步非 null。
- 验证：RPC response test、policy test、三平台 asset/SSE/Sentry/updater smoke。

### AR07-T02 — Webview navigation 同源限制

- 文件：`crates/koharu/src/app.rs`。
- RED：外部 origin 可替换主 Webview。
- GREEN：只允许本次 service origin；外链交 opener；dev origin 单独冻结。
- 验证：`bun cargo test -p koharu navigation_`、Tauri smoke。

### AR07-T03 — 删除全盘 FS scope

- 依赖：AR05-T05A、AR08-T02。
- 文件：`crates/koharu/capabilities/default.json`、`scripts/tauri-security-config.test.ts`。
- RED：capability 含 `fs:scope "**"`。
- GREEN：只保留 dialog 动态临时 scope 与实际命令。
- 验证：policy test、`bun run build`、三平台 open/save/ZIP smoke；重启后旧授权失效。

### AR08-T01 — ZIP entry 路径验证

- 文件：`ui/lib/io/saveBlob.ts`、`ui/tests/lib/io/saveBlob.test.ts`。
- RED：`..`、`.`、空段、POSIX absolute、drive、UNC、反斜杠 traversal 产生写入。
- GREEN：一次纯验证/规范化边界；目标严格为选择目录后代。
- 验证：saveBlob Vitest、Windows path smoke。

### AR08-T02 — ZIP 全量预验证与预算

- 依赖：AR08-T01。
- 文件：`saveBlob.ts`、`saveBlob.test.ts`。
- RED：非法/超预算 ZIP 在发现错误前已经 mkdir/write。
- GREEN：所有 entry 与总预算在第一次写前验证；零部分文件。
- 验证：saveBlob Vitest、大 ZIP UI smoke。
- 停止：若 `unzipSync` 无法在分配前有界，回 PLAN 选择直接保存 ZIP 或后端受限提取。

### AR09-T01 — SHA-256 下载/缓存不变量

- 文件：`crates/koharu-runtime/src/downloads.rs`、`install.rs`、`Cargo.toml`、根 `Cargo.toml`。
- RED：已有缓存不重验 digest；错误下载覆盖已验证安装；marker 不含 digest。
- GREEN：增加已批准 `sha2 0.10.9` 直接边；缓存、下载、解压前共用验证；source id 含 digest。
- 验证：runtime downloads/install tests。

### AR09-T02 — llama/ZLUDA artifact 描述

- 依赖：AR09-T01。
- 文件：`llama.rs`、`zluda.rs`、`downloads.rs`、`install.rs`。
- RED：artifact 缺 URL/digest/archive_kind/selected_files；mismatch 仍 extract/preload。
- GREEN：最小 `NativeArtifact` 数据结构；错误 digest 清 temp，不替换安装。
- 验证：llama/zluda tests；macOS/Windows/Linux真实 artifact smoke 后置。

### AR09-T03 — CUDA PyPI 官方 digest

- 依赖：AR09-T01。
- 文件：`cuda.rs`、`downloads.rs`、`install.rs`。
- RED：wheel metadata 缺 digest 仍安装，或 digest 变化不改变 source id。
- GREEN：只信任 PyPI `digests.sha256`；缺失 fail closed。
- 验证：cuda tests；Windows/Linux真实 wheel smoke 后置。

### AR10-T01 — Actions 固定完整 SHA

- 文件：新 `scripts/supply-chain-policy.test.ts`、`.github/workflows/build.yml`、`publish.yml`、`release.yml`。
- RED：任意非本地 `uses:` 不是 40 字符 SHA。
- GREEN：固定 commit SHA，并保留版本注释。
- 验证：policy test、`actionlint`。

### AR10-T02 — Release 最小权限与签名 CLI digest

- 依赖：AR10-T01。
- 文件：`release.yml`、`supply-chain-policy.test.ts`。
- RED：workflow 顶层写权限过宽；下载执行 CLI 无版本/digest 校验。
- GREEN：权限下放到 job；CLI 固定版本和 digest。
- 验证：policy test、release actionlint；凭据项保持 `PENDING-CREDENTIAL-QA`。

### AR10-T03 — 同 run artifact、Docker provenance 与 fork

- 依赖：AR10-T02；独占 Dockerfile。
- 文件：`release.yml`、`Dockerfile`、`supply-chain-policy.test.ts`。
- RED：Dockerfile 使用 `releases/latest`；container 重建/下载不同 binary；authority 不是 `nbjinkui1980-tech`。
- GREEN：只消费当前 run immutable artifact + digest；OCI source/revision/version；统一 fork。
- 验证：policy test、actionlint、本地 docker build/digest compare；不 push。

### AR11-T01 — Mask bitmap 页面代次

- 文件：`ui/hooks/useMaskDrawing.ts`、对应 test。
- RED：旧页面 late bitmap 绘制/上传到新页且未 close。
- GREEN：generation guard；所有 stale bitmap `close()`。
- 验证：useMaskDrawing Vitest、快速切页 smoke。

### AR11-T02 — Config 保存失败与乱序

- 文件：`ui/components/SettingsDialog.tsx`、新对应 test。
- RED：失败吞掉且 key draft 丢失；较旧响应覆盖较新编辑。
- GREEN：显式 error + draft 保留；latest mutation guard。
- 验证：SettingsDialog Vitest。

### AR11-T03 — Style mutation lossless queue

- 文件：`RenderControlsPanel.tsx`、`ui/lib/io/scene.ts`、现有两个 tests。
- RED：连续字号/颜色/描边更新互相覆盖。
- GREEN：按 project/page 串行；执行时读取最新 scene。
- 验证：RenderControlsPanel + scene Vitest。

### AR11-T04 — Auto-render project/page 隔离

- 文件：`ui/lib/io/scene.ts`、`ui/tests/lib/io/autoRender.test.ts`。
- RED：不同 page 互相取消；关闭项目后旧 timer 仍执行。
- GREEN：keyed Map debounce；按项目取消。
- 验证：autoRender fake-timer test。

### AR11-T05 — Scene 临时错误保留旧数据

- 文件：`ui/hooks/useScene.ts`、如必要 `ui/lib/api/index.ts`、对应 test。
- RED：5xx/network 被转换成无项目并清空旧 scene。
- GREEN：只对明确 no-project 清空；其他错误保留 data 并可重试。
- 验证：useScene Vitest。

### AR11-T06 — Verification URL allowlist

- 文件：`ui/lib/backend.ts`、`SettingsDialog.tsx`、新 backend test。
- RED：非 HTTPS、userinfo、未批准 host 可被打开。
- GREEN：固定认证 host + HTTPS；调用者不能扩展 allowlist。
- 验证：backend Vitest、Codex login smoke。

### AR12-T01 — Query 缓存 bytes，组件拥有 URL

- 文件：`ui/hooks/useBlobData.ts`、`ui/components/Image.tsx`、两个对应 tests。
- RED：query cache 持有 object URL；replacement/error/unmount 未 revoke。
- GREEN：cache 保存 Blob/bytes；组件 create/revoke。
- 验证：URL spy tests。

### AR12-T02 — FontFace owner

- 文件：`ui/components/ui/font-select.tsx`、现有 FontSelect test。
- RED：stale load 添加 face；unmount 不 delete。
- GREEN：组件 ownership + cancellation cleanup。
- 验证：FontSelect Vitest。

### AR12-T03 — UI jobs/downloads retention

- 文件：`ui/lib/stores/jobsStore.ts`、`downloadsStore.ts`、`ui/tests/lib/events.dispatch.test.ts`。
- RED：completed 无限增长或 Running 被 trim。
- GREEN：固定 bound，保留 Running；与后端 256 completed 对齐。
- 验证：events dispatch Vitest。

### AR12-T04 — Updater cleanup

- 文件：`ui/components/Updater.tsx`、现有 Updater test。
- RED：replacement/unmount 不 close 或重复 close。
- GREEN：明确 owner，只 close 一次。
- 验证：Updater Vitest；真实 updater 保持凭据门禁。

### AR12-T05 — 文本输入原生 undo/redo

- 文件：`ui/hooks/useKeyboardShortcuts.ts`、现有 test。
- RED：input/textarea/contenteditable 的 Ctrl/Cmd+Z/Y 触发 scene history。
- GREEN：editable target 直接保留浏览器行为。
- 验证：keyboard Vitest、三平台键盘 smoke。

### AR12-T06 — 字体收藏与删除按钮 a11y

- 文件：`font-select.tsx`、`Navigator.tsx`、两个现有 tests。
- RED：收藏 Enter/Space 冒泡选择字体；删除按钮无 accessible name/focus-visible。
- GREEN：独立键盘 target、accessible name、可见焦点。
- 验证：FontSelect + Navigator Vitest。

### Wave 5 gate

四条 lane 各自 GREEN 后才串行运行 runtime、UI、policy、Tauri/Docker build；平台和凭据 evidence 未完成时不得宣称发布就绪。

## 8. Wave 6 — AR-14 收敛

### AR14-T03A～E — Rust format 机械分片

共同 RED/GREEN：`bun cargo fmt --all -- --check`；每卡只允许 rustfmt 变化，前序任务已格式化的文件从卡中删除。

- `T03A`：`crates/koharu-app/bin/pipeline.rs`、`src/blobs.rs`、`pipeline/d0_visual_manifest_harness/mod.rs`、`source_gate_selection/device.rs`、`source_gate_selection/mod.rs`。
- `T03B`：`source_gate_selection/tests.rs`、`bubble_segmentation.rs`、`ctd_segment.rs`、`renderer/mod.rs`、`support/images.rs`。
- `T03C`：`yuzumarker_font.rs`、`pipeline/mod.rs`、App `renderer/mod.rs`、`session.rs`、`tests/pipeline_cli_admission.rs`。
- `T03D`：`tests/pipeline_smoke.rs`、Core `op.rs`、LLM `paddleocr_vl.rs`、ML `inpainting/mask.rs`、Renderer `tests/rendering.rs`。
- `T03E`：RPC `mcp/mod.rs`、`routes/pages.rs`、Runtime `runtime.rs`。

验证：`bun cargo fmt --all -- --check`、`git diff --check`。

### AR14-T06 — CI 完整门禁

- 依赖：AR10-T03；独占 workflows。
- 文件：`scripts/supply-chain-policy.test.ts`、`.github/workflows/lint.yml`、`test.yml`。
- RED：CI 缺 workspace/all-targets、fmt、Clippy、UI format/lint/test、generated、cargo/bun audit 任一阻断步骤。
- GREEN：全部阻断；allowlist 含 advisory/reachability/owner/expiry 且过期失败。
- 验证：policy test、actionlint、实际 PR workflow。

### FINAL-T01 — 单一 verifier 完整门禁

串行执行：

```bash
CARGO_TARGET_DIR=/tmp/koharu-sdd-final bun cargo fmt --all -- --check
CARGO_TARGET_DIR=/tmp/koharu-sdd-final bun cargo check --workspace --all-targets
CARGO_TARGET_DIR=/tmp/koharu-sdd-final bun cargo clippy --workspace --all-targets -- -D warnings
CARGO_TARGET_DIR=/tmp/koharu-sdd-final bun cargo test --workspace --tests
bun cargo audit
bun audit --registry https://registry.npmjs.org
bun run format:check
bun run lint:ui
bun run test:ui
bun run check:generated
bun run --cwd ui build
```

任何失败、skip、永久 ignore、默认 build 缺口或 HIGH 均阻止完成。

### FINAL-T02 — 平台与凭据状态

- macOS arm64、Windows x64、Ubuntu x64：workspace/UI/Tauri build 和规格 smoke。
- Ubuntu：Docker auth、SSE、MCP、non-loopback fail closed。
- Windows：drive/UNC ZIP、CUDA/ZLUDA digest。
- macOS：dialog scope、CSP、mask race。
- 真实 release tag、GHCR push、updater 签名、Winget、生产 Sentry 未获单独授权时保持 `PENDING-CREDENTIAL-QA`。

## 9. Phase 4 Entry Gate

进入 IMPLEMENT 前必须同时满足：

1. `[x]` AMEND-01 与 AMEND-02 已获人工决定，并同步回 approved SPEC/PLAN。
2. `[x]` 本 TASKS 文件状态改为 APPROVED；checkpoint commit 由本次批准创建。
3. `[ ]` 用户明确授权 Phase 4 IMPLEMENT；TASKS 批准本身不等于实现授权。

推荐批准文本：

```text
批准 Phase 3 TASKS；批准三份 SDD 文档 checkpoint commit。
```
