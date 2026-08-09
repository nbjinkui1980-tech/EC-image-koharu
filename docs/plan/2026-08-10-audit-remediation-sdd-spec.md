# 全项目审查修复 SDD 规格

**状态：APPROVED (AMENDED) — 2026-08-10；允许进入 TASKS 审阅，不直接授权实现。**

**日期：** 2026-08-10
**基线：** `codex/g005-typography-checkpoint` / `a035d113`
**执行分支：** `codex/audit-remediation-sdd`
**范围来源：** 2026-08-10 全项目代码、架构、安全、性能、UI、CI 与供应链审查。

本文件是本轮修复的唯一规格来源。严格遵守 `SPECIFY → PLAN → TASKS → IMPLEMENT`：本规格获批前，不修改产品代码、CI、依赖或运行配置。

## 1. Objective

修复审查确认的发布阻断项，同时保持 Koharu 现有图像翻译、渲染、项目格式、OpenAPI/Orval、SSE 与 pipeline 行为不变。

完成后的系统必须满足：

1. HTTP、SSE、下载、blob 与 MCP 控制面不再匿名暴露用户权限。
2. 所有 BlobRef 在共同边界上验证，无法逃逸 blob 根目录。
3. Scene、history、undo/redo 和 Batch 在失败时保持原子。
4. archive、图片、请求体、任务和前端缓存都有明确资源预算和生命周期所有者。
5. Tauri 渲染器、运行时原生制品、GitHub Actions 和容器发布链满足最小权限与内容完整性要求。
6. 已确认的 UI 数据竞态、静默失败、资源泄漏和键盘可访问性问题得到回归测试覆盖。
7. 完整 workspace、UI、格式、静态分析、生成物、依赖审计和支持平台构建成为可重复门禁。

## 2. Assumptions

以下假设和决策已由用户于 2026-08-10 批准：

1. Koharu 继续是单进程、单所有者应用；不增加账号、RBAC、多租户或内建 TLS。
2. 合法 BlobRef 是现有 BLAKE3 写入路径生成的 64 位小写 ASCII 十六进制值。
3. LM Studio 等经用户显式配置的 loopback/RFC1918 provider 仍是合法功能，不能一刀切禁止私网 URL。
4. 合法 `.khr`、Scene、Op JSON、epoch、undo/redo 和成功 API 响应保持兼容。
5. 修复控制面认证允许一次明确记录的客户端兼容变更；同时删除不安全且重复的 `/pages/from-paths` API；不保留匿名兼容 shim。
6. 当前进行中的 G005 不属于本规格；安全修复不得改写 `.omx/ultragoal` 的 G005-G009 目标、证据或视觉行为。
7. 本规格在获批后从当前基线创建独立修复分支；不把实现混入 G005 产品切片。

## 3. Tech Stack

- Rust 2024 workspace、Axum 0.8、Tokio、Tauri 2、postcard、zip。
- Next.js 16、React 19、TypeScript、TanStack Query、Zustand、Vitest、MSW。
- Bun、Cargo、GitHub Actions、Docker/OCI。
- 不新增依赖，除非 PLAN 阶段证明标准库和已安装依赖无法满足并获得单独批准。

## 4. Commands

### 快速定向验证

```bash
bun cargo test -p koharu-core
bun cargo test -p koharu-app
bun cargo test -p koharu-rpc
bun run --cwd ui test
```

### 完整验收

```bash
bun cargo fmt --all -- --check
bun cargo check --workspace --all-targets
bun cargo clippy --workspace --all-targets -- -D warnings
bun cargo test --workspace --tests
bun cargo audit
bun audit --registry https://registry.npmjs.org
bun run format:check
bun run lint:ui
bun run test:ui
bun run check:generated
bun run --cwd ui build
```

默认 Next/Turbopack 构建必须在受支持的标准布局通过；当前机器外置 `node_modules` 布局需要单独 smoke。webpack 通过不能永久替代声明的生产构建门禁，除非 PLAN 明确冻结该选择。

## 5. Project Structure

- `crates/koharu-core/`：BlobRef、Scene、Op 不变量。
- `crates/koharu-app/`：BlobStore、history/session、archive、provider、资源生命周期。
- `crates/koharu-rpc/`：认证、Host/Origin、路由预算、HTTP/MCP/SSE 适配。
- `crates/koharu/`：CLI、Tauri 启动、桌面 bootstrap、CSP/navigation。
- `crates/koharu-runtime/`：原生制品下载、digest、缓存与 preload。
- `ui/`：统一 API/SSE 边界、并发所有权、资源释放、可访问性。
- `.github/workflows/`、`Dockerfile`：CI 与发布 provenance。
- `scripts/`：只放仓库策略静态检查，不复制产品逻辑。
- 测试优先放在已有相邻 test 模块或 `ui/tests/`；仅在跨 crate/router 行为需要时新增集成测试文件。

## 6. Code Style

共享不变量只实现一次，并在存储层做纵深防御；禁止在每个调用者零散过滤。

```rust
let blob_ref = BlobRef::parse(input)?;
let path = store.path_for(&blob_ref)?;
```

```typescript
const generation = currentGeneration
const bitmap = await createImageBitmap(blob)
if (generation !== currentGeneration) {
  bitmap.close()
  return
}
```

规则：

- 先复用现有类型、queue、AbortSignal、React Query key 和测试 fixture。
- 安全输入拒绝必须 fail closed，不能 silent normalize。
- 失败不得产生部分 scene、history、staging、文件写入或 UI 乐观状态。
- 不增加通用 service layer、状态机、队列框架或“以后可能用”的抽象。

## 7. Functional and Security Requirements

### AR-01：控制面认证、Origin、Host 与远程暴露

- `/api/v1/**`、SSE、blob、下载和 `/mcp` 使用同一认证边界。
- 缺失/错误凭据返回 `401` 且无副作用；策略拒绝才返回 `403`。
- token 不出现在 URL、查询参数、响应体、普通日志或遥测中。
- Desktop 启动不要求用户手动输入 token。
- Headless/Docker 必须显式提供 secret；非 loopback 缺少 secret 或 Host allowlist 时拒绝启动。
- CORS 禁止 wildcard、任意 origin 回显和 `null`；生产同源，开发仅允许冻结的 localhost origin。
- 静态资源和最小 liveness 可公开；所有用户状态和操作受保护。
- 不实现用户账号、OAuth、RBAC 或内建 TLS。

### AR-02：BlobRef 不变量和目录包含性

- 只接受精确 64 位 `[0-9a-f]`。
- 拒绝空、短、长、大小写混合、Unicode、斜杠、反斜杠、点段、绝对路径和 percent-decoded 分隔符。
- HTTP、MCP、Scene、history 和 `.khr` 使用同一反序列化不变量。
- BlobStore 即使收到绕过反序列化的非法值，也必须拒绝根目录外路径。
- 合法但不存在的 hash 保持 `404`；非法格式返回稳定客户端错误。
- 不可信项目含非法引用时，导入失败并清理 staging，不成为当前项目。

### AR-03：Provider authority、secret 与 SSRF

- base URL 必须是绝对 `http`/`https` URL，禁止 user-info、fragment 和其他 scheme。
- scheme、host 或有效端口变化视为 authority 变化；旧 secret 不得发送到新 authority。
- 同 authority 的 base path 变化可以保留 secret。
- 跨 authority redirect 不携带 Authorization。
- 认证用户显式配置并重新提交 secret 后，loopback/RFC1918 provider 继续可用。
- Provider 错误、API 错误、JobSummary、日志和 Sentry 只保留有界、脱敏摘要；禁用默认 PII。

### AR-04：Scene/History 原子性

- 任意单 Op、混合/嵌套 Batch、undo 和 redo 全部成功或完全不生效。
- 失败时 scene、epoch、undo/redo 栈、事件和 durable log 保持调用前状态。
- 成功 Batch 仍只增加一个 epoch、一个 frame、一个 undo 单元。
- 合法顺序依赖保持有效。
- write/flush/sync 失败时不得发布内存状态；无法安全回滚日志尾部时 session fail-stop。
- 不改变 Op JSON、成功 epoch 语义或 history 格式。

### AR-05：Archive、history、图片、路径和请求预算

- 路由使用各自 body limit，JSON/control 不再继承 bulk 上限。
- ZIP 按实际读取量限制 entry 数、单项、总展开字节和压缩比；不按不可信声明 size 直接分配。
- history frame 在分配前验证长度；超限完整帧不得当作可忽略截断尾帧。
- 删除 `/pages/from-paths`；Tauri dialog 选择后只通过其临时 FS scope 使用 `readFile` 读取 bytes，再复用 multipart `POST /pages`。Axum 不再接收或读取客户端本机路径。
- 页面导入最多 256 个文件、总编码 bytes 512 MiB、总解码 RGBA bytes 1 GiB、同时 decode 2 个；现有单图解码上限 512 MiB 保持。
- 所有失败清理 staging，且不修改当前项目、scene 或 blob。

建议初始预算，批准前不得写入产品代码：

| 项目 | 建议值 |
| --- | ---: |
| JSON/control body | 1 MiB |
| 单 mask body | 64 MiB |
| archive compressed body | 512 MiB |
| ZIP entries | 20,000 |
| 单 entry/blob | 512 MiB |
| 总展开字节 | 4 GiB |
| `scene.bin` | 64 MiB |
| `history.log` | 256 MiB |
| 单 history frame | 16 MiB |
| 最大压缩比 | 100:1 |
| 单次图片文件数 | 256 |
| 单次图片总编码 bytes | 512 MiB |
| 单次图片总解码 RGBA bytes | 1 GiB |
| 同时 decode 数 | 2 |

这些值必须在 PLAN 阶段用不读取业务内容的真实项目 size/entry 统计校准。

### AR-06：Job 并发与保留

- Pipeline、AI 和 bulk import 使用小型有界 semaphore；满载返回 `429 + Retry-After`。
- 每项目最多一个运行中的 pipeline；AI 使用独立全局限流。
- 成功、失败、取消和 panic 都释放 slot。
- 完成记录使用固定容量或 TTL；不得永久增长。
- 不引入外部队列、分布式调度器或真实模型测试。

### AR-07：Tauri CSP、navigation 和 FS scope

- Axum HTML 响应发送实际生效的 CSP；Tauri 配置不再为 `null`。
- 基线至少包含 `default-src 'self'`、`object-src 'none'`、`base-uri 'none'`、`frame-ancestors 'none'`、`form-action 'none'`。
- navigation 只允许当前服务 origin；开发 origin 单独配置。
- 删除 FS scope `"**"`，保留 dialog 与实际使用的最小读写命令。
- 首轮使用 dialog 临时授权；不增加跨重启 persisted scope。
- 正常打开、导入、保存、ZIP 导出、字体、图片、SSE、Sentry 和 updater 不被 CSP/权限误拦。

### AR-08：ZIP 导出边界

- 拒绝 `..`、`.`、空段、POSIX 绝对路径、Windows drive/UNC 和反斜杠 traversal。
- 规范化目标必须是用户选择目录的后代。
- 在任何写入前验证所有 entry 和总预算。
- 不新增 ZIP 依赖；优先保留 ZIP 文件直接保存或使用后端受限流式提取。
- 当前后端安全文件名不是省略信任边界校验的理由。

### AR-09：运行时原生制品完整性

- llama/ZLUDA/CUDA artifact 都有 `{url, sha256, archive_kind, selected_files}`。
- PyPI wheel 使用官方元数据 `digests.sha256`；缺 digest 即拒绝。
- 缓存命中、下载完成和解压前均验证 digest；缓存身份包含 digest。
- mismatch 清理临时文件，不覆盖已验证安装，不 preload。
- 安装 marker/source id 纳入完整 digest 集合。
- 首轮不引入 Sigstore/TUF，也不扩展到普通模型权重 pinning。

### AR-10：Actions、权限与容器 provenance

- 所有非本地 `uses:` 固定 40 字符 commit SHA，并保留版本注释。
- Release/Publish 权限下放到 job；Winget、container、签名分别获得最小权限。
- 下载后执行的签名/发布 CLI 固定版本和 digest。
- Linux 二进制由当前 `$GITHUB_SHA` 构建；container job 只使用同一 run 的 immutable artifact 并验证 digest。
- Dockerfile 不再通过 `releases/latest` 获取 Koharu 二进制。
- OCI labels 包含 source、revision 和 version；updater、桌面、Winget、容器使用同一发布主体。

### AR-11：UI 数据一致性

- 旧页面异步 bitmap 不得绘制或上传到新页面；所有失效 bitmap 都 `close()`。
- 配置保存失败显式显示错误并保留 API key draft；乱序响应中较旧结果不能覆盖较新编辑。
- 连续样式编辑基于队列执行时的最新 scene，保留所有字段修改。
- Auto-render 按冻结的 project/page 语义隔离；切换或关闭项目时取消旧任务。
- Scene 的临时 5xx/网络错误保留现有数据并显示可重试错误，不能伪装成“无项目”。
- 外部 verification URL 仅允许 `https:` 和批准的认证 host。

### AR-12：UI 资源与可访问性

- React Query 缓存 Blob/bytes，不缓存 object URL；组件在 replacement、error 和 unmount 时 revoke。
- FontFace、ImageBitmap、Updater 对象和 timer 有明确 owner 与 cleanup。
- jobs/downloads UI store 有固定 retention bound。
- 文本输入中的 Ctrl/Cmd+Z/Y 保留浏览器行为。
- 字体收藏与字体选择使用独立键盘目标；破坏性按钮有可见 focus 和可访问名称。

### AR-13：错误、项目 ID 与传输层边界

- 5xx 响应不返回 anyhow chain、绝对路径、provider body 或内部实现细节。
- 删除/打开项目只接受规范 ID；不得 slugify 一个破坏性请求后作用于另一个项目。
- RPC route 不继续扩张应用用例编排；只把已经出现重复或安全不变量分叉的单个用例移到 `koharu-app`。
- generated API 已存在的调用不得继续用 raw fetch 重复实现。

### AR-14：依赖、格式、CI 与构建

- 清零当前 5 个 Clippy `-D warnings` 和 Rust/UI format 失败。
- CI 使用 `--workspace --all-targets`，运行 Rust/UI format、Clippy、生成物检查和 UI tests。
- `cargo audit` 与官方 registry `bun audit` 成为阻断门禁。
- 可达 runtime Critical/High 必须为零；不可达传递告警只有在记录 advisory、可达性、owner 和到期日后才能临时 allowlist。
- 升级 Tauri/plist 链直到 `quick-xml >= 0.41`，并处理 Next 16.2.9/sharp 的直接告警。
- 不批量升级全部依赖，不使用永久 `continue-on-error`。
- 默认生产构建在受支持 CI 和标准本地布局可重复通过。

## 8. Testing Strategy

每个任务严格执行：

1. 在实现前加入一个最小失败测试并记录 RED 原因。
2. 只实现使该测试通过的最小共享边界修复。
3. 运行定向测试；失败则不进入下一任务。
4. 每个规格组完成后运行相关 crate/UI suite。
5. 最后运行本文件“完整验收”全部命令。

必须覆盖的测试形状：

- Router：无/错/正确 token、Origin、Host、MCP、SSE、encoded slash。
- Core/App：非法 BlobRef 反序列化、BlobStore containment、失败 Batch、日志 write/flush/sync 注入。
- Archive：limit-1/limit/limit+1、伪造 size、高压缩比、超 entry、清理 staging。
- Jobs：barrier/pending future 验证 admission、释放和 retention；不启动真实模型。
- UI：deferred promise、fake timers、两个 page/project key、逆序响应、unmount cleanup。
- Policy：静态读取 Tauri JSON、workflow YAML、Dockerfile，检查 CSP、FS scope、Action SHA、权限与 provenance。

平台手工验收：

- macOS/Windows/Linux Tauri：打开、导入、保存、ZIP、CSP、临时 FS scope。
- Desktop/headless/Docker：cookie/bearer、SSE、MCP、非 loopback fail-closed。
- 快速切页/画 mask/连续样式编辑/切项目。
- 错误 digest、超预算请求和满并发错误可理解且不残留状态。

## 9. Boundaries

### Always

- 先 RED 测试，后最小实现，再 GREEN。
- 每个任务约不超过 5 个文件；超过则回到 PLAN 重新拆分。
- 保留无关 dirty/untracked 文件；不修改 `.omc/`、`.omo/` 和 G005 证据。
- 安全和数据丢失边界必须 fail closed。
- 每个提交只对应一个已批准任务；是否提交仍需当前用户授权。

### Ask first

- 改变本规格、认证兼容、资源预算、发布主体或分支策略。
- 新依赖、公开 API/格式迁移、数据库/项目格式变更。
- 需要凭据的真实 release、Winget、容器推送或外部系统变更。

### Never

- 先实现后补规格或测试。
- 为旧匿名控制面或非法 BlobRef 增加静默兼容 shim。
- 删除/跳过失败测试以获得绿色。
- 在日志、URL、错误、测试 fixture 或仓库中写入真实 secret。
- 顺带重写 pipeline、React Query、OpenAPI/Orval 或全部 RPC 架构。

## 10. Success Criteria

1. AR-01 至 AR-14 每项都有先失败、后通过的回归测试和对应最小实现。
2. 两条 Critical 利用链在 router/serialization 层被自动测试阻断。
3. 故障注入证明 scene/history/undo/redo 无部分状态。
4. 预算边界在 limit、limit+1 和清理路径全部可重复。
5. Desktop、headless、MCP 和 Docker 的认证迁移有文档和 smoke evidence。
6. Native artifact、Action、签名 CLI、container 均可追溯到固定 digest/commit。
7. UI 竞态、资源 cleanup 和键盘行为全部有确定性测试。
8. 完整验收命令全部通过；若平台/凭据阻止验证，必须明确记录为未完成，不能宣称发布就绪。
9. 最终 `git diff` 只包含批准的规格任务；G005 产品行为和证据不受影响。

## 11. Approved Decisions

1. **分支隔离：** 从 `a035d113` 创建 `codex/audit-remediation-sdd`；不修改当前 G005 goal ledger。
2. **认证：** Desktop 每次启动生成 256-bit 临时 secret；同源 UI 使用 HttpOnly/SameSite cookie，API/MCP 使用 Bearer；Headless/Docker 从环境变量或 secret file 注入，缺失即 fail closed。
3. **远程部署：** Koharu 不内建 TLS；非 loopback 必须 Bearer + Host allowlist，并要求受信反向代理终止 TLS；不保留匿名兼容窗口。
4. **Provider authority 变化：** PATCH 返回明确冲突，要求用户重新提交 secret；不自动静默清除，也不复用旧 secret。
5. **资源预算：** 以 AR-05 表格为 PLAN 初始值，用真实项目元数据校准；校准后的规格变化必须重新审阅。
6. **发布主体：** `nbjinkui1980-tech` fork 是 updater、Winget 和 container 的唯一 authority。
7. **Auto-render：** 按 project/page 独立、不可丢失 debounce；关闭/切换项目时取消该项目全部 pending render。
8. **FS 授权：** 每次 dialog 临时授权，不跨重启保存目录权限。
9. **依赖审计：** reachable runtime Critical/High 必须为零；不可达传递漏洞仅允许带 owner、理由和到期日的短期 allowlist。
10. **本机图片导入：** 删除 `/pages/from-paths`；统一使用 dialog 临时 scope `readFile` + multipart `POST /pages`。
11. **批量图片预算：** 每次 256 files / 512 MiB encoded / 1 GiB decoded / 2 decoders；单图 512 MiB 不变。

任何批准项发生变化时，必须先更新并重新审阅本文件；不得直接改变实现。
