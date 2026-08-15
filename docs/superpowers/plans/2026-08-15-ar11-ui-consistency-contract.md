# Lane 执行合同:L-AR11 — UI 数据一致性(6 张相互独立卡)

- 状态:**Phase 3 一次批准已覆盖(2026-08-13 授予)**;LOOP-3 本地认领已登记
- 认领基线:Phase 3 起点 main `b68f123e`;执行分支 `audit-remediation-phase3`(认领时 tip `3a13538d`)
- 提交/回滚单元:T01~T06 各一个单卡 commit(可独立 revert);lane 收口 docs(evidence) 单独一个 commit
- 前置依赖:无(卡间相互独立);T03/T04 共享 `ui/lib/io/scene.ts`、T02/T06 共享 `SettingsDialog.tsx`,串行执行天然隔离
- 执行环境偏差(继承):子代理基础设施已恢复(oracle 近两 lane 收口均成功);lane 收口独立 review 沿用 oracle,失败则对抗性自审并落档
- 串行点声明:UI lane——门禁含 `test:ui`/`lint:ui`/`format:check`/`ui build`(L-AR13B 教训:mock 不验证真实导出,UI 卡门禁必含 build);不占用 Cargo/lockfile/Orval

## 范围文件域(域外改动禁止)

| 卡 | 允许文件 |
|---|---|
| AR11-T01 | `ui/hooks/useMaskDrawing.ts`、`ui/tests/hooks/useMaskDrawing.test.tsx`(≤2) |
| AR11-T02 | `ui/components/SettingsDialog.tsx`、`ui/tests/components/SettingsDialog 新测试文件`(≤2) |
| AR11-T03 | `ui/components/panels/RenderControlsPanel.tsx`、`ui/lib/io/scene.ts`、既有两个 tests(≤4) |
| AR11-T04 | `ui/lib/io/scene.ts`、`ui/tests/lib/io/autoRender.test.ts`(≤2) |
| AR11-T05 | `ui/hooks/useScene.ts`、必要时 `ui/lib/api/index.ts`、对应 test(≤3) |
| AR11-T06 | `ui/lib/backend.ts`、`ui/components/SettingsDialog.tsx`、新 backend test(≤3) |

新依赖:无。

## 卡:AR11-T01 — Mask bitmap 页面代次

- **验收标准(TASKS 原文)**:文件:`ui/hooks/useMaskDrawing.ts`、对应 test。RED:旧页面 late bitmap 绘制/上传到新页且未 close。GREEN:generation guard;所有 stale bitmap `close()`。验证:useMaskDrawing Vitest、快速切页 smoke。
- **现状(RED-0 源码实证)**:`convertBytesToBitmap` await 后(useMaskDrawing.ts:65/123)无页面代次检查——切页后 late bitmap 仍 `drawImage` 进新页 canvas;close() 只在既定路径(70/125/132),stale 路径不 close。
- **设计**:hook 内 generation 计数(每次页面/会话切换自增);await 返回后比对代次,不一致则 `bitmap.close()` 并放弃绘制/上传;所有弃用路径保证 close。
- **RED 断言**(useMaskDrawing Vitest):
  1. `stale_bitmap_is_closed_and_not_drawn_after_page_switch` — 绘制 await 期间切页 → 旧 bitmap 不 drawImage、`close()` 被调 → 现状 FAIL
- **目标文件**:上表 T01 行(≤2)
- **验收命令**:`bun run --cwd ui test -- tests/hooks/useMaskDrawing.test.tsx`
- **证据**:RED 1F/1P(stale bitmap 被绘制且未 close)→GREEN 2/2;commit `5999a1e1`

## 卡:AR11-T02 — Config 保存失败与乱序

- **验收标准(TASKS 原文)**:文件:`ui/components/SettingsDialog.tsx`、新对应 test。RED:失败吞掉且 key draft 丢失;较旧响应覆盖较新编辑。GREEN:显式 error + draft 保留;latest mutation guard。验证:SettingsDialog Vitest。
- **现状(RED-0 源码实证)**:保存经 `updateConfig`(scene.ts:224);SettingsDialog 存在空 `catch {}`(218/230/279/340 区域),保存失败无显式 error、draft 被清空;并发/连发保存无 latest-only 守卫。
- **设计**:保存失败 → 显式错误提示且保留 draft;连续保存以递增序号守卫,仅最后一次 mutation 落盘生效。
- **RED 断言**(新 SettingsDialog 测试):
  1. `config_save_failure_keeps_draft_and_shows_error` — PATCH 失败 → draft 保留 + 错误可见 → 现状 FAIL
  2. `config_save_older_response_does_not_overwrite_newer_edit` — 旧响应后到 → 不覆盖新 draft → 现状 FAIL
- **目标文件**:上表 T02 行(≤2)
- **验收命令**:`bun run --cwd ui test -- tests/components/SettingsDialog`
- **证据**:RED 2F/0P(失败吞错+丢 draft;旧响应覆盖新值)→GREEN 2/2;commit `e8c6cbf1`;类型注解修复 `4429d1dd`(ui build 门禁捕获,L-AR13B 教训再证)

## 卡:AR11-T03 — Style mutation lossless queue

- **验收标准(TASKS 原文)**:文件:`RenderControlsPanel.tsx`、`ui/lib/io/scene.ts`、现有两个 tests。RED:连续字号/颜色/描边更新互相覆盖。GREEN:按 project/page 串行;执行时读取最新 scene。验证:RenderControlsPanel + scene Vitest。
- **现状(RED-0 源码实证)**:样式更新经 `applyOp`(scene.ts:65)直发,无按页串行队列;连续快速更新可交错,后到 op 基于旧 scene 覆盖先到的。
- **设计**:按 project/page 键的 promise 链串行;每个 op 执行时读取最新 scene 构造 patch。
- **RED 断言**(RenderControlsPanel + scene Vitest):
  1. `rapid_style_updates_apply_in_order_without_overwrite` — 连续 3 个样式 op 全部按序生效 → 现状 FAIL
- **目标文件**:上表 T03 行(≤4)
- **验收命令**:`bun run --cwd ui test -- tests/components/RenderControlsPanel tests/lib/io/scene`
- **证据**:RED 2F/40P(scene 集成+panel builder 均FAIL)→GREEN 42/42;commit `9a722c57`;设计:applyOp 重载 builder 在队列轮次内对最新缓存构造;panel resolveOpArg 读缓存断言模式

## 卡:AR11-T04 — Auto-render project/page 隔离

- **验收标准(TASKS 原文)**:文件:`ui/lib/io/scene.ts`、`ui/tests/lib/io/autoRender.test.ts`。RED:不同 page 互相取消;关闭项目后旧 timer 仍执行。GREEN:keyed Map debounce;按项目取消。验证:autoRender fake-timer test。
- **现状(RED-0 源码实证)**:scene.ts:102-116 全局单 `autoRenderTimer` + `autoRenderPendingPageId`——`queueAutoRender(A)` 后 `queueAutoRender(B)` 直接 `clearTimeout` 掉 A;项目关闭无取消入口。
- **设计**:`Map<projectId:pageId, timer>` keyed debounce;`closeProject`/切项目时取消该项目全部 pending timer。
- **RED 断言**(autoRender fake-timer):
  1. `auto_render_timers_are_isolated_per_page` — A、B 页各自触发,互不取消 → 现状 FAIL
  2. `auto_render_timer_cancelled_on_project_close` — 关闭项目后 timer 不执行 → 现状 FAIL
- **目标文件**:上表 T04 行(≤2)
- **验收命令**:`bun run --cwd ui test -- tests/lib/io/autoRender`
- **证据**:RED 2F/7P(跨页互消;关闭后仍触发)→GREEN 9/9;commit `0fa9b34f`

## 卡:AR11-T05 — Scene 临时错误保留旧数据

- **验收标准(TASKS 原文)**:文件:`ui/hooks/useScene.ts`、如必要 `ui/lib/api/index.ts`、对应 test。RED:5xx/network 被转换成无项目并清空旧 scene。GREEN:只对明确 no-project 清空;其他错误保留 data 并可重试。验证:useScene Vitest。
- **现状(RED-0 源码实证)**:useScene.ts:26-28 `if (isError) return { scene: null, epoch: 0 }`——任何错误(含 5xx/network)都把已有 scene 清空为 null。
- **设计**:仅对明确的 no-project(400 "no project open")清空;其余错误保留上一次 data 并允许重试(React Query keep previous data 语义)。
- **RED 断言**(useScene Vitest):
  1. `transient_fetch_error_keeps_previous_scene` — 有旧 data 时 5xx/network → scene 保留 → 现状 FAIL
  2. `explicit_no_project_clears_scene` — 400 no-project → 清空(锁)
- **目标文件**:上表 T05 行(≤3)
- **验收命令**:`bun run --cwd ui test -- tests/hooks/useScene`
- **证据**:RED 1F/3P(500 清空旧 scene)→GREEN 4/4;commit `4aa36a36`;既有 400 清场测试为锁

## 卡:AR11-T06 — Verification URL allowlist

- **验收标准(TASKS 原文)**:文件:`ui/lib/backend.ts`、`SettingsDialog.tsx`、新 backend test。RED:非 HTTPS、userinfo、未批准 host 可被打开。GREEN:固定认证 host + HTTPS;调用者不能扩展 allowlist。验证:backend Vitest、Codex login smoke。
- **现状(RED-0 源码实证)**:backend.ts:6-14 `openExternalUrl(url)` 直接 Tauri `openUrl`/`window.open`,无 scheme/host/userinfo 校验;verification URI 来源(Codex device login)与打开同一通道。
- **设计**:backend.ts 内部固定 allowlist(认证 host)+ 强制 HTTPS + 拒绝 userinfo/query 凭据;校验失败报错不打开;调用者无法传入自定义 allowlist。
- **RED 断言**(新 backend 测试):
  1. `verification_url_rejects_http_userinfo_and_unknown_hosts` — http://、user@host、陌生 host 均拒绝 → 现状 FAIL
  2. `verification_url_allows_fixed_auth_host` — 批准的认证 host HTTPS → 打开(锁)
- **目标文件**:上表 T06 行(≤3)
- **验收命令**:`bun run --cwd ui test -- tests/lib/backend`
- **证据**:RED 1F/1P(三类 URL 未拒绝)→GREEN 6/6(backend+Codex polling+config);commit `944915d6`

## Lane 收口(2026-08-15)

- 门禁:UI suite 259P/0F(38 文件)、lint:ui exit 0、format:check 净、ui build exit 0、workspace check/clippy/fmt 净、check:generated 零漂移
- 独立 review(oracle):零 blocker;**2 major**——superseded 配置失败仍报错(persistConfig 改 saved/failed/superseded outcome 联合,旧失败静默)/样式 builder 闭包持有点击时 page(执行时与渲染前双重 page 货币性检查);**2 minor**——switchProject 不清 auto-render 定时器(修)/URL :443 与大小写主张不实(URL 归一化,实证裁决;trailing-dot/punycode/编码 userinfo 拒绝补锁);**4 informational**——onBaseUrlBlur 失败静默(修)/理论 unhandled rejection(记录)/color 显式化既有行为(记录)/测试边角(补)
- review-fix commit:`b72c4c89`(7 文件,+130/-16)
- 提交/回滚单元:T01 `5999a1e1`、T02 `e8c6cbf1`+`4429d1dd`、T03 `9a722c57`、T04 `0fa9b34f`、T05 `4aa36a36`、T06 `944915d6`、review-fix `b72c4c89`,均可独立 revert
- 依赖传播:无下游;W5 未收齐(L-AR12 未竟)
