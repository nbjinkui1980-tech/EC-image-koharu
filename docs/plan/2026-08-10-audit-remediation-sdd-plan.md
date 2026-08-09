# 全项目审查修复 SDD 技术实施计划

**状态：APPROVED (AMENDED) — 2026-08-10；允许进入 Phase 3 TASKS 审阅，不直接授权产品实现。**

**规格：** `docs/plan/2026-08-10-audit-remediation-sdd-spec.md`（APPROVED）
**基线：** `a035d113`
**分支：** `codex/audit-remediation-sdd`
**发布主体：** `nbjinkui1980-tech`

本文件仅定义组件、依赖顺序、并行边界、风险、回滚和验证 checkpoint。获批后才能生成 Phase 3 的逐任务清单；每个任务仍须先产生目标断言 RED，再写最小实现。

## 1. Outcome

按批准规格完成 AR-01 至 AR-14，优先阻断两条 Critical 利用链和数据丢失路径，再收紧资源、桌面、UI 和供应链边界，最后把全部约束固化为 CI 与跨平台 evidence。

不重写 pipeline、OpenAPI/Orval、React Query、项目格式、history 格式或 G005 行为。

## 2. Dependency Map

```text
W0 规格/分支/失败基线
  ↓
W1 依赖表面冻结 + 现有门禁清噪
  ↓
W2 共享不变量
  ├─ AR-13 公开错误边界 ──→ Provider / Job 摘要
  ├─ AR-02 BlobRef ─────────→ Archive / multipart import
  └─ AR-04 Batch core ──────→ Durable history
  ↓
W3 网络与持久化边界
  ├─ AR-01 RPC auth core ───→ Desktop / Headless / Docker
  ├─ AR-03 Provider authority
  └─ AR-04 Durable history ─→ History frame budget
  ↓
W4 入口集成、资源预算与任务生命周期
  ↓
W5 可并行领域线
  ├─ Tauri CSP/FS + ZIP
  ├─ Native artifact digest
  ├─ UI consistency/resources
  └─ Release/container provenance
  ↓
W6 CI、审计、默认构建和完整 convergence
  ↓
W7 跨平台与凭据验收
```

## 3. Wave 0 — Authority and Baseline

### Scope

- 将批准规格和本计划纳入独立分支的受控基线，记录文件 digest。
- 保持 `.omc/`、`.omo/` 和 `.omx/ultragoal` G005-G009 完全排除。
- 记录当前已知失败：5 个 Clippy、Rust/UI format、2 个 RustSec、47 个 Bun advisories、当前外置依赖布局的 Turbopack panic。
- 只统计真实项目 size/entry 元数据，不读取内容。

### Calibration result

本机 managed projects 只有 1 个项目、21 个文件、约 1.4 MiB，最大文件约 392 KiB，`scene.bin` 约 3 KiB，样本不足以下调已批准预算。因此保持规格 AR-05 默认值，后续用合成 limit/limit+1 fixture 验证。

AR-06 初始容量提议：

| Resource | Proposed limit |
| --- | ---: |
| Pipeline running per project | 1 |
| AI jobs globally | 2 |
| Bulk imports globally | 1 |
| Completed job retention | 256 |

AR-05 批量图片预算（2026-08-10 补丁批准）：

| Resource | Approved limit |
| --- | ---: |
| Files per import | 256 |
| Total encoded bytes | 512 MiB |
| Total decoded RGBA bytes | 1 GiB |
| Concurrent decoders | 2 |

现有单图解码上限 512 MiB 保持。Tauri 图片选择统一通过 dialog 临时 scope `readFile` 转成 `File[]`，再复用 multipart；删除 `/pages/from-paths`，后端不再接受本机路径。

### Checkpoint C0

- 分支为 `codex/audit-remediation-sdd`，HEAD 基线可追溯至 `a035d113`。
- spec/plan digest 可复核。
- 尚未改变产品、依赖、CI 或认证行为。

## 4. Wave 1 — Dependency Surface Freeze and Gate Cleanup

此波只消除后续会造成重复返工的版本漂移与既有静态噪声。

### Components

- 定向升级 Tauri/plist，直到锁文件使用 `quick-xml >= 0.41`。
- 定向升级 Next/sharp 到修复直接告警的兼容版本；不批量升级传递依赖。
- 清零当前 Rust/UI format 和 5 个 Clippy 错误，不重构邻近代码。
- 明确默认生产构建：优先固定 Next workspace/Turbopack root；webpack 只作诊断备选。
- 冻结 Tauri、Next、Axum 和生成 API 表面，供后续安全任务使用。

### Risks and rollback

- 若依赖升级改变公开 API、静态导出或 Tauri capability schema，停止并返回规格审阅。
- 依赖变更使用单一 owner；更新 lockfile 时暂停其他 Cargo/Bun 任务。
- 可回退到上一锁文件，但不得用永久 advisory ignore 替代可用修复版本。

### Checkpoint C1

- format、workspace check/clippy、目标 UI tests、默认标准布局 build 通过。
- `cargo audit` 不再包含两条 quick-xml advisory。
- Next/sharp 的 reachable direct High 为零，或存在符合规格的短期记录。

## 5. Wave 2 — Shared Invariants

三条 lane 可以并行写独立文件；full gate 串行。

### Lane 2A — AR-13 public error boundary

- `ApiError` 分离稳定公开消息与只进入 tracing 的内部 cause。
- 5xx 不返回 anyhow chain、绝对路径或 provider body。
- Sentry `send_default_pii=false`。
- 不建立通用错误框架；Provider/Job 摘要在后续接入。

Checkpoint：绝对路径、嵌套 cause、假 secret 和 provider body 均不出现在 HTTP/SSE/Sentry payload；既有明确 4xx 保持。

### Lane 2B — AR-02 BlobRef and containment

- `BlobRef` 形成唯一 parse/Serde 边界：64 位小写 hex。
- 先机械迁移测试里的非法 placeholder，再收紧构造函数，避免一次修改跨越文件上限。
- BlobStore 再验证并返回 `Result<PathBuf>`；RPC 将非法格式映射为稳定 4xx。
- 不做非法旧数据 canonicalize 或兼容 shim。

Checkpoint：真实 Axum encoded-slash、Serde、absolute/parent/Unicode/uppercase 全部拒绝；合法 put/get 和 missing `404` 不变。

### Lane 2C — AR-04 Batch core atomicity

- `Op::Batch` 在 scratch scene 和 scratch ops 上顺序执行，全部成功后一次发布。
- 保留 AddPage→AddNode 等顺序依赖、inverse 和单 undo 粒度。
- 先采用完整 Scene clone 的最小正确实现；只有基准证明不可接受才返回 PLAN。

Checkpoint：混合/嵌套 Batch 最后一步失败时 scene 和 `prev_*` 字节不变；成功 Batch 回归通过。

### Wave gate C2

```bash
bun cargo fmt --all -- --check
bun cargo check -p koharu-core -p koharu-app -p koharu-rpc --all-targets
bun cargo clippy -p koharu-core -p koharu-app -p koharu-rpc --all-targets -- -D warnings
bun cargo test -p koharu-core
bun cargo test -p koharu-app
bun cargo test -p koharu-rpc
```

## 6. Wave 3 — Control Plane, Provider and Durable History

三条 lane 可并行，但各自共享文件由单一 owner。

### Lane 3A — AR-01 RPC security core

最小架构：

```text
AccessProfile + immutable SecurityContext
  ↓
Host policy → Origin/CORS policy → Auth → Readiness → Handler
  ├─ public static assets + minimal liveness
  ├─ cookie: authenticated browser session
  └─ bearer: REST/SSE/MCP/automation
```

- SecurityContext 是 router 构造参数，不放入业务 App，也不依赖 app ready。
- 所有 `/api/v1/**`、blob、download、SSE 和 `/mcp` 都在认证层内。
- `/mcp` 只接受 Bearer；cookie 变更请求还必须通过 Host/Origin。
- `OPTIONS` 不要求 Bearer，但只为允许 origin 返回允许头。
- token 使用 32-byte OS CSPRNG；传输编码使用 URL-safe base64；比较解码后的固定 32 bytes。

Checkpoint：无/错/正确 credential、cookie/bearer、Host、Origin、SSE、MCP 完整 router 矩阵；handler 前拒绝且无副作用。

### Lane 3B — AR-03 Provider authority

- 复用已安装 `url`，建立唯一规范化 authority 比较。
- scheme、host、有效端口变化且未显式提交新 secret 时返回 `409`。
- 同 authority 的 base path 变化保留 secret。
- 非 HTTP(S)、user-info、fragment 在联网前拒绝。
- 跨 authority redirect 不发送 Authorization；若 reqwest 默认行为不满足，由 provider 专用 redirect policy 修复。
- loopback/RFC1918 仍允许用户显式配置。

Checkpoint：mock provider/redirect 证明旧 secret 不外传；catalog/error/log 不含响应体或 secret。

### Lane 3C — AR-04 Durable history

- apply/undo/redo 在候选 Scene、epoch 和栈上完成。
- frame durable write 成功后才发布内存状态。
- append 前记录旧尾；write/flush/sync 失败先回滚尾部。
- 无法安全恢复时 session fail-stop，后续 mutation 明确拒绝。
- 不改变 frame、Op JSON 或 epoch 成功语义。

Checkpoint：可控 writer 故障注入证明 scene、epoch、两栈、事件和 log length 不变；重开只看到完整 pre/post state。

### Wave gate C3

- 三条 lane 的 crate suite 全绿。
- OpenAPI/Orval 无意外漂移。
- 未开始 Desktop cookie bootstrap、archive limit 或 UI 行为改动。

## 7. Wave 4 — Integration, Budgets and Admission

### Lane 4A — Desktop/Headless/Docker authentication integration

- Desktop 启动生成临时 master secret。
- Tauri WebView 通过仅 Tauri IPC 可访问的一次性 bootstrap proof，调用受 Bearer 保护的 session exchange，获得 `HttpOnly; SameSite=Strict; Path=/` session cookie；外部浏览器访问静态 HTML不能自动获得授权 cookie。
- Headless/Docker 从环境变量或 secret file 读取 master secret；缺失即 fail closed。
- Headless browser UI 可显式输入 token，只在内存中完成 session exchange；不写 URL、localStorage 或 bundle。
- 非 loopback 同时要求 secret、Host allowlist 和外部 TLS proxy。
- UI credential 行为集中在 bootstrap、`fetchApi` 和 SSE；不改 generated callers。

Checkpoint：Desktop 无人工 token；Headless/Docker 缺 secret 不监听；带 secret时 browser、REST、SSE、MCP 均通过。

### Lane 4B — AR-05 archive/history budget

- 使用批准常量，不建立通用 quota framework。
- ZIP 按实际读取字节流式写 staging，限制 entry、单项、总展开量和压缩比。
- history frame 在分配前检查 16 MiB。
- control、mask、archive 使用独立 body limit。
- 所有失败沿用 staging cleanup，不发布项目。

Checkpoint：limit−1/limit/limit+1、`u32::MAX`、伪造 size、高压缩比和 cleanup 全绿。

### Lane 4C — AR-05 path/image budget

- 删除 `/pages/from-paths` 和对应 generated caller；不建立 path-grant registry。
- Tauri dialog 选择后只用其动态临时 FS scope 调用 `readFile`，转换为 `File[]` 后复用 Web 的 multipart `POST /pages`。
- multipart 在 decode 前限制 256 files 和 512 MiB 编码总量；decode 后限制 1 GiB RGBA 总量，使用 2-permit semaphore。
- 任一读取、解码或预算失败发生在 blob/scene mutation 之前。

Checkpoint：后端无本机 path import route；总预算和混合失败无部分 blob/scene；正常 Web/Tauri multipart import 保持。

### Lane 4D — AR-06 jobs and retention

- 使用现有 Tokio semaphore：pipeline 每项目 1、AI 全局 2、bulk import 全局 1。
- permit 以 RAII 移入任务；success/failure/cancel/panic 全部释放。
- Running 永不淘汰；完成项固定保留 256 条。
- 满载返回 `429` 与 `Retry-After: 1`。
- HTTP/MCP pipeline 共用 admission。

Checkpoint：barrier/pending future 验证上限、释放、panic cleanup 和第 257 个完成项淘汰；不启动真实模型。

### Lane 4E — Remaining AR-13 boundaries

- Create display name 继续 slugify；open/delete 只接受精确 canonical ID。
- Mask/raw fetch 收敛到已生成 API；不得手改 generated 文件。
- Export filename header 若 Orval 无法保留，返回规格审阅，不牺牲用户可观察文件名。

### Wave gate C4

- Core/App/RPC full tests 和 workspace check/clippy 通过。
- 两条 Critical 利用链、history 原子性和资源预算矩阵全部绿色。
- Desktop/headless/Docker 本地 smoke 通过。

## 8. Wave 5 — Parallel Domain Lanes

### Lane 5A — AR-07/08 Tauri, CSP, FS and ZIP

顺序：先 ZIP 全量预验证，再移除宽 FS scope。

- ZIP 所有 entry 和总预算在第一次 mkdir/write 前验证；目标只能是所选根目录后代。
- Axum HTML 响应发出 CSP；Tauri config 同步不再是 `null`。
- navigation 仅允许本次服务 origin；dev origin 单独配置。
- 删除 `fs:scope "**"`，保留实际命令和 dialog 临时 scope。
- 官方 Tauri v2 dialog 实现会把选择的文件/目录动态加入 FS scope；是否递归仍由每个 picker 用例明确测试。

Checkpoint：POSIX/drive/UNC/backslash traversal 零写入；CSP/capability policy tests；三平台 dialog/open/save/ZIP smoke。

### Lane 5B — AR-09 native artifact integrity

- 一个最小 `NativeArtifact` 描述 `{url, sha256, archive_kind, selected_files}`。
- PyPI wheel 读取官方 `digests.sha256`，缺失即拒绝。
- digest 进入缓存身份和 install source id；缓存、下载、解压前共用验证。
- mismatch 不覆盖已验证安装、不 preload、清理 temp。

Checkpoint：正确/错误 digest、损坏缓存、中断下载、缺 PyPI digest 和 marker 变化确定性通过。

### Lane 5C — AR-11/12 UI ownership and cleanup

- Read/query latest-wins；lossless mutation 按 project/page 串行且执行时读取最新 scene。
- Mask 使用 generation，旧 bitmap 只 close 不 draw/upload。
- Auto-render 使用 project/page keyed Map；项目关闭清理 pending。
- Scene 5xx/网络错误保留旧数据；明确“无项目”才返回 null。
- 配置失败保留 draft并显示；旧响应不能覆盖新编辑。
- Query cache 保存 Blob/bytes；组件拥有 object URL。
- FontFace、ImageBitmap、Updater、timer、jobs/downloads retention 全部有 owner。
- 文本输入 undo/redo、字体收藏和删除按钮满足键盘/a11y。

Checkpoint：deferred promise、fake timers、MSW inverse response 和 cleanup spy 全绿；UI full suite/typecheck/build 通过。

### Lane 5D — AR-10 release/container provenance

- 所有非本地 Action 固定 40 字符 SHA；权限降到 job。
- 下载执行的签名/发布 CLI 固定 digest。
- 当前 `$GITHUB_SHA` 的 Linux artifact 和 digest由同一 run 传给 container job。
- Dockerfile 不联网获取 Koharu；OCI labels 带 source/revision/version。
- updater、Winget、container 全部指向 `nbjinkui1980-tech`。

Checkpoint：静态 policy test、无凭据 dry-run、容器内 binary digest 与 artifact 一致；不 push、不发 tag。

## 9. Wave 6 — AR-14 Convergence

- CI 的 check/clippy 使用 `--workspace --all-targets`。
- Rust/UI format、UI tests、生成物、policy tests、cargo audit、官方 registry bun audit 全部阻断。
- Allowlist 必须含 advisory、reachability、owner、expiry；过期自动失败。
- Action SHA、Docker provenance、Tauri CSP/FS policy 纳入 CI。
- 默认 Next build 在标准布局通过；外置 dependency layout 作为额外 smoke。
- AR-10 完成后才统一编辑 workflows，避免 writer 冲突。

### Checkpoint C6

按固定顺序串行执行规格中的完整验收命令。任何 registry、平台、凭据或默认构建缺口都保持未完成状态。

## 10. Wave 7 — Platform and Credential Evidence

### No-credential evidence

- macOS arm64、Windows x64、Ubuntu x64 workspace/UI/Tauri build。
- Ubuntu Docker auth/headless/SSE/MCP smoke。
- Windows drive/UNC 与 ZLUDA/CUDA digest。
- macOS dialog/CSP/mask race。

### Credential-gated evidence

- 真实 Release、GHCR push、updater 签名、Winget 和生产 Sentry 需要单独用户授权。
- 凭据阶段只发布已经验证的同 commit/digest artifact，不重新构建。
- 未授权时状态为 `PENDING-CREDENTIAL-QA`，禁止宣称发布就绪。

## 11. Verification Protocol

每个后续任务固定执行：

1. `RED-0`：测试 harness 可编译。
2. `RED-1`：只在目标断言失败；编译/环境失败不算 RED。
3. `GREEN-1`：同一测试字节通过。
4. `GREEN-2`：相邻模块回归通过。
5. `WAVE-GREEN`：本波完整门禁通过。

保存 AR 编号、命令、退出码、失败/成功摘要和 diff SHA。验收语义改变必须返回规格审阅。

## 12. Parallel Execution Boundaries

- 每条 lane 单一 writer；共享文件冲突时串行。
- Rust 并行任务使用独立 `CARGO_TARGET_DIR=/tmp/koharu-sdd-target-<ar>`。
- Full workspace tests 由唯一 verifier 使用独立 target 串行执行。
- 不并行运行 Cargo format/audit、依赖更新、lockfile、Orval、Next build 或生成物命令。
- UI fake timers、global mocks、MSW、stores 必须 restore；`scene.ts` 单例测试使用独立 Vitest 进程。
- `Cargo.lock`、`bun.lock`、Tauri config、workflow、Dockerfile 是独占写入区。
- 不清理其他 lane 的 target、`.next`、临时目录或未跟踪文件。

## 13. Stop and Rollback Conditions

立即停止当前波次：

- RED 意外通过或失败点不是目标断言。
- 只能通过弱化、skip、retry 或删除测试实现 GREEN。
- 拒绝路径仍有部分 scene/history/file/job 或 secret 泄漏。
- 需要改变项目/history/API 格式、批准预算、认证方案、发布主体或 G005。
- 合法既有项目出现非 64 位小写 BlobRef。
- MCP 客户端无法使用 Bearer，或 Desktop cookie 无法避免匿名 bootstrap。
- 单个任务无法拆到约 5 文件而又需要新通用抽象。
- Audit 需要无 owner/无期限 ignore 或 `continue-on-error`。

回滚以未发布 wave 为单位；不得通过恢复匿名 API、`CSP=null`、`FS="**"`、跳过 digest 或关闭 audit 回滚。

## 14. Phase 3 Entry Approvals

以下事项已由用户于 2026-08-10 批准：

1. 批准本计划的 W0-W7 顺序、checkpoint 和回滚边界。
2. 批准 AR-06 初始容量：pipeline/project `1`、AI/global `2`、bulk import/global `1`、completed retention `256`。
3. 批准增加以下**已有锁版本的直接依赖边**，不引入新包版本：
   - `getrandom 0.3.4`：生成 32-byte OS CSPRNG secret。
   - `base64 0.22`：URL-safe no-padding token transport encoding。
   - `sha2 0.10.9`：runtime artifact SHA-256 verification。
4. 批准在 Phase 3 前创建一个仅包含 approved spec + approved plan 的 checkpoint commit；如不批准提交，则以记录的文件 digest 继续，但不宣称规格已进入版本控制基线。

本批准只授权生成 Phase 3 TASKS 和创建 spec/plan checkpoint commit；不授权修改产品代码、依赖、CI 或运行配置。Phase 3 TASKS 仍需独立人工批准后才能进入 IMPLEMENT。

### 2026-08-10 Phase 3 规格补丁

用户已批准：

1. 删除 `/pages/from-paths`，统一使用 dialog scoped `readFile` + multipart，不实现一次性 path-grant registry。
2. 图片预算冻结为 256 files / 512 MiB encoded / 1 GiB decoded / 2 decoders；单图 512 MiB 保持。

该补丁只解除 TASKS 阻塞，不授权 IMPLEMENT 或新的 Git commit。
