# AR-03 Provider Authority 执行合同(L-AR03)

- 来源:`docs/plan/2026-08-12-audit-remediation-sdd-loop.md` §2 队列 P0 首条
- 卡序:**AR03-T01 → AR03-T02 → AR03-T03**(T02/T03 均复用 T01 的 authority 比较,串行)
- 依赖确认:T01 依赖 AR13-T01(✅ `88781c76`);T03 依赖 AR03-T01 + AR13-T01;全部就绪
- 状态:**Phase 3 一次批准已覆盖(2026-08-13 授予)**;LOOP-3 本地认领已登记,合同 SHA-256 `22565edc71c01579e133eef60aa58890`
- 认领基线:Phase 3 起点 main `b68f123e`;执行分支 `audit-remediation-phase3`(tip 见 loop 文档 §8 日志)
- 提交/回滚单元:T01/T02/T03 各一个单卡 commit(可独立 revert);lane 收口 docs(evidence) 单独一个 commit

## 范围文件域(域外改动禁止)

| 卡 | 允许文件 |
|---|---|
| T01 | `crates/koharu-llm/src/providers/mod.rs`、`crates/koharu-llm/src/providers/openai_compatible.rs`,允许新增 `providers/authority.rs` |
| T02 | `crates/koharu-app/src/config.rs`、`crates/koharu-rpc/src/routes/config.rs` |
| T03 | `crates/koharu-llm/src/providers/openai_compatible.rs`、`crates/koharu-app/src/llm.rs`、`crates/koharu-rpc/src/routes/llm.rs` |

**申请扩展(随合同一并批准,否则回 PLAN)**:
1. `crates/koharu-runtime/src/runtime.rs` — 仅当 T03 RED-0 实测证明 reqwest 默认 redirect 行为不满足时,新增 provider 专用 client 构建(见 T03 设计点)。
2. T03 的 `ensure_provider_success` 位于 `providers/mod.rs`(已在 T01 文件域内,不额外扩展)。

**明确出范围**:UI 任何文件(409 的前端处理属 AR11/12 域);`koharu-ai`(Codex provider 自带 client);`google_fonts.rs` 自建 client(无 secret);downloads client(HF 跨 authority 下载重定向保持现状)。

## 串行点声明

- **Orval/OpenAPI**:T02 给 config 端点新增 409 响应会改 utoipa 注解 → `bun run check:generated` 可能漂移,lane 收口时重生成。本 lane 占用该串行点。
- **lockfile**:不占用。零新依赖 — mock HTTP server 用 `tokio::net::TcpListener` 手写(koharu-llm 无 dev-deps,不加 axum/wiremock)。
- format/audit/Next build:不占用。

## 现状基线(侦察事实,RED 设计依据)

- `openai_compatible.rs:31-37` `normalized_base_url`:仅 trim + 去尾斜杠。`ftp://`、userinfo、fragment、任意端口组合均可通过并联网。
- `providers/mod.rs:99` `ensure_provider_success`:`bail!("{provider} API request failed ({status}): {body}")` — 全量响应体进入错误文本。
- `runtime.rs:44-58` `build_client`:UA + 超时 + retry 中间件,**未设 redirect policy**(reqwest 默认 `limited(10)`);providers 经 `llm.rs:422/496` 共享此 downloads client。
- `config.rs:~272` `apply_patch`:base_url 以 `.or_else(existing)` 静默合并,无 authority 冲突检测;`hydrate_provider_secrets` 从 keyring 注入 secret。
- `routes/config.rs`:`patch_config` / `set_provider_secret`(PUT) / `clear_provider_secret` / `upsert_provider_secret`,无 409 路径。
- `url = 2.5` 已是 koharu-llm 直接依赖(SPEC"复用已安装 url"满足)。

---

## 卡:AR03-T01 — Provider URL 与 authority 规范化

- **验收标准(TASKS 原文)**:RED:非 HTTP(S)、userinfo、fragment 可联网;effective port/scheme/host 比较错误。GREEN:复用 `url` 的唯一 authority 比较;同 authority path 变化相等。
- **设计**:新增 `providers/authority.rs`,提供唯一入口 `provider_authority(raw: &str) -> Result<Authority>`(基于 `url::Url`;scheme+host+effective port;非 http(s)/userinfo/fragment 拒绝)与 `Authority::eq`。**stub 先行**(`Ok` 直通)以满足 RED-0 编译要求。`normalized_base_url` 改为复用该解析(保留 path,供 URL 拼接)。
- **RED 断言**(`bun cargo test -p koharu-llm authority`):
  1. `authority_rejects_non_http_scheme` — `ftp://example.com` 应 Err;stub 返回 Ok → FAIL
  2. `authority_rejects_userinfo` — `http://user:pass@host/v1` 应 Err → FAIL
  3. `authority_rejects_fragment` — `http://host/v1#frag` 应 Err → FAIL
  4. `authority_effective_port` — `https://h` == `https://h:443`;`http://h:80` == `http://h`;`http://h` != `https://h` → 比较不存在/stub 错误 → FAIL
  5. `authority_same_path_insensitive` — `http://h:8080/v1` == `http://h:8080/v2/api`(path 不参与 authority)→ FAIL
- **目标文件**:`providers/authority.rs`(新)、`providers/openai_compatible.rs`、`providers/mod.rs`(≤3)
- **验收命令**:`bun cargo test -p koharu-llm authority`
- **证据记录**:RED 输出(5 断言失败)/ GREEN 输出 / commit SHA

## 卡:AR03-T02 — Config authority 冲突(409)

- **验收标准(TASKS 原文)**:RED:已有 secret 的 provider 改 authority 且未提供新 secret 时旧 secret 被复用或配置部分更新。GREEN:mutation 前返回 409;显式新 secret 后才提交;同 authority path 保留 secret。
- **设计**:`config.rs` 在 `apply_patch`/保存路径加 authority 变更检测(复用 T01 比较):已有 secret + authority 变化 + 未显式提交新 secret → 返回结构化冲突(不部分更新)。`routes/config.rs` 映射为 `409` + 稳定有界 body。utoipa 注解同步(触发 Orval 串行点)。
- **RED 断言**(`bun cargo test -p koharu-app provider_authority` / `bun cargo test -p koharu-rpc config_conflict`):
  1. `patch_authority_change_without_secret_conflicts` — 有 secret 的 provider,base_url `http://h:8080/v1` → `http://h:9090/v1`,不带新 secret → 期望冲突错误;当前静默合并 → FAIL
  2. `patch_same_authority_path_change_keeps_secret` — `http://h:8080/v1` → `http://h:8080/api` → 允许且 secret 保留;当前行为凑巧通过,作为锁定测试
  3. `rpc_patch_config_returns_409` — PATCH 改 authority 无新 secret → 409 + 稳定 body;当前 2xx → FAIL
  4. `rpc_set_secret_then_authority_change_commits` — 显式提交新 secret 后变更 → 2xx 完整提交
- **目标文件**:`config.rs`、`routes/config.rs`(≤2)
- **验收命令**:`bun cargo test -p koharu-app provider_authority`、`bun cargo test -p koharu-rpc config_conflict`
- **证据记录**:RED / GREEN / 409 body 样例 / commit SHA

## 卡:AR03-T03 — Redirect 与 provider 错误脱敏

- **验收标准(TASKS 原文)**:RED:mock A 跨 authority redirect 到 B,B 收到 Authorization;大 body/secret 出现在公开错误。GREEN:provider 专用 redirect policy 去除敏感 header;只保留有界摘要。
- **RED-0 实测前置(强制)**:手写双 `TcpListener` server(A 要求 Bearer 并 302 → B;B 记录所收 headers),分别测:host 变化、**port 变化(同 host)**、path 变化。记录 reqwest 默认行为实测表(预期:reqwest 跨 host 默认剥离,跨 port/scheme 泄漏)。实测表写入本合同证据区;若默认全满足,redirect 部分降级为锁定测试并回 PLAN 确认范围缩减。
- **设计(预期路径)**:provider 专用 client:`redirect::Policy::none()` + 自写 redirect 中间件/跟随逻辑,跨 authority(复用 T01 比较)剥离 `Authorization`/`Cookie`,同 authority 保留;providers 改收专用 client(不动 downloads client — HF 场景保持默认)。`ensure_provider_success` 错误体改为有界(≤256 字符)且剔除疑似 secret 的摘要。
- **RED 断言**(`bun cargo test -p koharu-llm redirect` + provider error 测试):
  1. `redirect_cross_port_strips_authorization` — A(带 Bearer)302 → 同 host 不同 port 的 B,B 断言无 `Authorization`;默认 client 下 B 收到 → FAIL
  2. `redirect_same_authority_keeps_authorization` — A 302 → 同 authority 另一 path → `Authorization` 保留
  3. `redirect_cross_host_strips_authorization` — host 变化场景锁定(默认已剥离则直接 GREEN,仍作为回归锁)
  4. `provider_error_bounded_and_redacted` — 5KB 含 `sk-live-secret` 的错误 body → 错误文本 ≤ 有界长度且不含 secret 子串;当前全量透传 → FAIL
- **目标文件**:`openai_compatible.rs`、`providers/mod.rs`(错误摘要)、`llm.rs`、`routes/llm.rs`;条件扩展 `runtime.rs`(≤5)
- **验收命令**:`bun cargo test -p koharu-llm redirect`、App/RPC provider error tests
- **证据记录**:RED-0 实测表 / RED / GREEN / B 端 header 捕获样例 / commit SHA

---

## Lane 收口门禁(Wave 3 gate 对齐)

- `bun cargo test -p koharu-llm`、`-p koharu-app`、`-p koharu-rpc` 全绿
- `bun cargo clippy --workspace --all-targets -- -D warnings`、`bun cargo fmt --all -- --check`
- `bun cargo check --workspace --all-targets`
- `bun run check:generated`(Orval 漂移已重生成且审查 diff)
- 独立 scoped code-review 零发现(对照 AR01 模式)
- provider redirect 与 config 409 可重复演示(实测表 + header 捕获)

## 风险与决策点(批准时一并确认)

1. **runtime.rs 条件扩展**:仅当 RED-0 实测默认不足才动;动则只加 provider 专用构建,不改 downloads 行为。
2. **409 对前端的影响**:当前 UI 不认识 409,会落入通用错误路径(fail-closed,可接受);正式 UX 属 AR11/12。
3. **零新依赖**:mock server 手写 `TcpListener`;若执行中发现手写不可靠,回 PLAN 申请 dev-dep。
4. **T02 的 secret 语义**:authority 变化后旧 secret 保留在 keyring 但不发送(不主动删除),与"显式新 secret 后才提交"一致。
