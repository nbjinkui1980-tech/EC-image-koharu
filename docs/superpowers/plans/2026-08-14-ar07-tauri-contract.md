# Lane 执行合同:L-AR07 — Tauri 攻击面(CSP + navigation 同源 + 删全盘 FS scope)

- 状态:**Phase 3 一次批准已覆盖(2026-08-13 授予)**;LOOP-3 本地认领已登记
- 认领基线:Phase 3 起点 main `b68f123e`;执行分支 `audit-remediation-phase3`(认领时 tip `83e6371d`)
- 提交/回滚单元:T01/T02/T03 各一个单卡 commit(可独立 revert);lane 收口 docs(evidence) 单独一个 commit
- 前置依赖:AR14-T04 ✅(`45b090cf`);T03 ← AR05-T05A ✅(`d13f9b6e`)+ AR08-T02 ✅(`4a2249e7`,2026-08-14 解锁)
- 执行环境偏差(继承):子代理模型映射故障未修(14 次失败),codegraph + 直接工具单执行器;lane 收口独立 review 重试,失败则对抗性自审并落档

## 范围文件域(域外改动禁止)

| 卡 | 允许文件 |
|---|---|
| AR07-T01 | `crates/koharu-rpc/src/server.rs`、`crates/koharu/tauri.conf.json`、新 `scripts/tauri-security-config.test.ts`(≤3) |
| AR07-T02 | `crates/koharu/src/app.rs`(≤1) |
| AR07-T03 | `crates/koharu/capabilities/default.json`、`scripts/tauri-security-config.test.ts`(≤2) |

## 卡:AR07-T01 — Axum HTML CSP 与 Tauri CSP

- **验收标准(TASKS 原文)**:RED:HTML 无 CSP,Tauri `csp=null`。GREEN:规格冻结指令由实际 HTML 响应发出,配置同步非 null。
- **SPEC 冻结基线(AR-07)**:`default-src 'self'`、`object-src 'none'`、`base-uri 'none'`、`frame-ancestors 'none'`、`form-action 'none'`;正常打开/导入/保存/ZIP/字体/图片/SSE/Sentry/updater 不被误拦。
- **现状(RED-0 源码实证)**:`server.rs serve_asset` 仅设 Content-Type,无 CSP 头;`tauri.conf.json:12` `"csp": null`。
- **设计**:`serve_asset` 对 `text/html` 响应注入 `Content-Security-Policy` 头;tauri.conf.json `csp` 同值非 null。指令集 = SPEC 基线 + 功能最小放行:`script-src 'self' 'unsafe-inline'`(Next 静态导出内联引导脚本)、`style-src 'self' 'unsafe-inline'`(React inline style)、`img-src 'self' data: blob:`、`font-src 'self' data:`、`connect-src 'self' https://*.sentry.io https://*.ingest.sentry.io`(Sentry 出站)。**让步点记录**:unsafe-inline 双项为 Next/React 现状所需,SPEC 基线未禁。
- **RED 断言**(`bun cargo test -p koharu-rpc csp` + policy test):
  1. `html_response_carries_csp` — 真 server 取 `/` → 响应含 CSP 头且含五条冻结指令;当前无 → FAIL
  2. `non_html_response_has_no_csp_requirement` — 锁:asset(CSS/JS)响应正常(不强制 CSP,或同样带头无害)
  3. policy test:`scripts/tauri-security-config.test.ts` 断言 tauri.conf.json `csp` 非 null 且含五条冻结指令 → 当前 null → FAIL
- **目标文件**:上表 T01 行(≤3)
- **验收命令**:`bun cargo test -p koharu-rpc csp`、`bun test scripts/tauri-security-config.test.ts`、`bun run --cwd ui build`(确认 UI 在 CSP 下可构建)
- **证据记录**:RED / GREEN / 响应头样例 / commit SHA

## 卡:AR07-T02 — Webview navigation 同源限制

- **验收标准(TASKS 原文)**:RED:外部 origin 可替换主 Webview。GREEN:只允许本次 service origin;外链交 opener;dev origin 单独冻结。
- **现状(RED-0 源码实证)**:`crates/koharu/src/app.rs` WebviewWindowBuilder 只 `.navigate(url)`,无 `on_navigation` 限制——webview 内任意链接可导航主窗口到外部 origin。
- **设计**:`on_navigation` 回调:release 模式仅允许 `http://127.0.0.1:{port}`(本次 service origin);debug 模式允许 dev_url origin;其余返回 false(外链由 UI 侧 opener 处理——现状 opener:default 权限在)。on_navigation 在 build 前注册(navigate 之后每次跳转被闸)。
- **RED 断言**(`bun cargo test -p koharu navigation_`):
  1. `navigation_allows_service_origin` — 同源 URL → true
  2. `navigation_rejects_external_origin` — `https://evil.example` → false;当前无闸 → FAIL(编译期:闸函数不存在 → RED-0 先落纯判定函数 `fn navigation_allowed(url, service_origin) -> bool` 未接线,测试直调)
  3. `navigation_dev_origin_allowed_in_debug` — debug 下 dev origin → true(cfg!(debug_assertions) 分支)
- **目标文件**:`crates/koharu/src/app.rs`(≤1)
- **验收命令**:`bun cargo test -p koharu navigation_`
- **证据记录**:RED / GREEN / commit SHA

## 卡:AR07-T03 — 删除全盘 FS scope

- **验收标准(TASKS 原文)**:RED:capability 含 `fs:scope "**"`。GREEN:只保留 dialog 动态临时 scope 与实际命令。验证:policy test、`bun run build`、三平台 open/save/ZIP smoke;重启后旧授权失效。
- **现状(RED-0 源码实证)**:`capabilities/default.json` 含 `{ "identifier": "fs:scope", "allow": [{ "path": "**" }] }`。
- **dialog 临时授权机制(侦查取证)**:tauri 核心源码(manager/window.rs)证实用户手势路径(drag-drop)自动 `allow_file` 进 fs runtime scope;dialog-open 同机制(runtime 内存态,重启即失效——正合"重启后旧授权失效"语义)。删除 `**` 后保留 `fs:allow-read-file/write-file/read-dir/mkdir/exists` + dialog 权限,dialog 选定路径经 runtime scope 临时授权可读。
- **设计**:删 `fs:scope` 块;policy test 断言:无 `**` scope、fs 读写命令在位、dialog 权限在位。
- **RED 断言**(policy test):
  1. `capabilities 无 fs scope ** 通配` → 当前有 → FAIL
  2. 锁:fs/dialog/opener 权限条目保留
- **目标文件**:上表 T03 行(≤2)
- **验收命令**:`bun test scripts/tauri-security-config.test.ts`、`bun run build`(桌面构建验证 capability 编译)
- **证据记录**:RED / GREEN / commit SHA
- **手动验证遗留项**:三平台 open/save/ZIP 真机 smoke 无法在本机执行——落档为后续手动项;机制依据(tauri auto-allow 源码证据)+ policy test 为本卡验证面

---

## Lane 收口门禁(Wave 5 gate 对齐)

- `bun cargo test -p koharu-rpc`、`-p koharu`(desktop crate)、`-p koharu-app` 全绿
- `bun cargo clippy --workspace --all-targets -- -D warnings`、`bun cargo fmt --all -- --check`
- `bun cargo check --workspace --all-targets`
- `bun run build`(T03 桌面构建;**前置:无 tauri dev 会话占租约**)
- `bun test scripts/tauri-security-config.test.ts`
- 独立 scoped code-review 零发现(重试子代理;故障则对抗性自审并落档偏差)
- CSP 响应头/navigation 闸/scope 删除可重复演示

## 风险与决策点(批准时一并确认)

- CSP 的 `unsafe-inline`(script/style)是 Next 静态导出 + React inline style 现状所需让步,SPEC 基线未禁;若后续要收严需 UI 侧改造,回 SPEC
- navigation 闸只挡 webview 主框导航;window.open/外链 UI 已有 opener:default
- T03 真机 dialog 授权 smoke 为手动遗留项(本机无法驱动 OS dialog)
- `bun run build` 桌面构建耗时长,仅 lane 收口跑
