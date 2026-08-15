# Lane 执行合同:L-AR05-PICKER — 导入路径收口(Tauri picker 统一 File + 删除 /pages/from-paths)

- 状态:**Phase 3 一次批准已覆盖(2026-08-13 授予)**;LOOP-3 本地认领已登记
- 认领基线:Phase 3 起点 main `b68f123e`;执行分支 `audit-remediation-phase3`(认领时 tip `363c9c37`)
- 提交/回滚单元:T05A/T05B 各一个单卡 commit(可独立 revert);lane 收口 docs(evidence) 单独一个 commit
- 前置依赖:AMEND-01 已批准(2026-08-10,删 `/pages/from-paths`,复用 `readTauriFiles` + multipart);T05B ← T05A
- 执行环境偏差(继承):子代理模型映射故障未修(10 次启动失败),codegraph + 直接工具单执行器;lane 收口独立 review 重试,失败则对抗性自审并落档

## 范围文件域(域外改动禁止)

| 卡 | 允许文件 |
|---|---|
| AR05-T05A | `ui/lib/io/openFiles.ts`、`ui/lib/io/pagesIo.ts`、新 `ui/tests/lib/io/openFiles.test.ts`、`ui/tests/lib/io/pagesIo.test.ts`(≤4) |
| AR05-T05B | `crates/koharu-rpc/src/routes/pages.rs`、`ui/lib/io/scene.ts`、生成物:`ui/openapi.json`、`ui/lib/api/generated.ts`、`ui/lib/api/schemas/*`(仅经 `bun run generate:api` 重生成) |

注:scene.ts 入域因 TASKS 验证要求 `rg 'from-paths|uploadPagesByPaths'` 无运行时调用,`uploadPagesByPaths` 定义于 scene.ts;生成物单独生成命令,不手改。

## 卡:AR05-T05A — Tauri picker 统一返回 File

- **验收标准(TASKS 原文)**:RED:Tauri picker 返回路径并调用 raw path API。GREEN:dialog 后用已有 `readTauriFiles`,与 Web 一样返回 `File[]` 并走 multipart。
- **现状(RED-0 源码实证)**:`openFiles.ts` `openImageFiles/openImageFolder` 的 Tauri 支路返回 `{ kind: 'paths', paths }`(行 31-56、59-85);`pagesIo.importPages` 对 paths 支路调 `uploadPagesByPaths` → 后端 raw-path API;`readTauriFiles`(行 118)现成,仅 `.khr` 导入在用。`ImagePickerResult` 消费者唯 `pagesIo.importPages`。
- **设计**:两个图片 picker 的 Tauri 支路末尾经 `readTauriFiles(paths)` 转 `File[]`;`ImagePickerResult` 类型坍缩删除,`openImageFiles/openImageFolder` 直接返回 `Promise<File[]>`(取消 → `[]`);`importPages` 删 paths 分支,统一 `uploadPages(files, replace)`(multipart)。web 支路不动。
- **RED 断言**(`bun run --cwd ui test -- tests/lib/io/openFiles.test.ts tests/lib/io/pagesIo.test.ts`):
  1. 新 `openFiles.test.ts`(vitest,mock `@/lib/backend` isTauri→true、`@tauri-apps/plugin-dialog.open`、`@tauri-apps/plugin-fs.readFile`):`openImageFiles` Tauri 下返回 File 实例数组(name/mime 正确)→ 当前返回 paths 对象 → FAIL
  2. 同上:`openImageFolder` Tauri 下返回 File[](过滤非图片、排序保留)→ FAIL
  3. 同上:用户取消 → 返回 `[]` 且不调用 readFile → FAIL(当前返回 `{ kind:'paths', paths:[] }`,类型即不符)
  4. `pagesIo.test.ts`:picker 返回 File[] 时 multipart `POST /api/v1/pages` 被调用且 scene 失效(现有 files 用例适配新类型后作为锁);paths 专属用例删除
- **目标文件**:上表 T05A 行(≤4)
- **验收命令**:同上 + `bun run lint:ui`
- **证据记录**:RED / GREEN / 测试输出样例 / commit SHA
- **证据(T05A 收口,2026-08-14)**:
  - RED:`bun run --cwd ui test -- tests/lib/io/openFiles.test.ts tests/lib/io/pagesIo.test.ts` → 8 failed/5 passed(openFiles 5 用例返回形状不符;pagesIo 3 个 importPages 用例 TypeError——裸数组无 `.files`,证明当前走 paths 分流;khr/export 5 用例无关面 PASS)
  - GREEN:同命令 → `2 passed (files)` / **13 passed/0 failed**;全 UI 套件 235 passed;`lint:ui` exit 0(仅既有 RenderControlsPanel 警告);`format:check` 净
  - 设计落地:Tauri 支路末尾 `readTauriFiles(paths)`;`ImagePickerResult` 坍缩删除;`importPages` 统一 `uploadPages`(multipart);web 支路零改动
  - Commit:`d13f9b6e`(4 文件,+128/-59)

## 卡:AR05-T05B — 删除后端 from-paths API

- **验收标准(TASKS 原文)**:RED:policy/OpenAPI 仍暴露 `/pages/from-paths`。GREEN:删除 route、request schema 和 raw filesystem read;multipart 保持。验证含全仓 `rg 'from-paths|uploadPagesByPaths'` 无运行时调用。
- **现状(RED-0 源码实证)**:`pages.rs:45` 注册 `create_pages_from_paths`;行 359-385+ 定义 `CreatePagesFromPathsRequest` + handler(直接读任意绝对路径);行 975 附近有 from-paths 测试用例;`ui/openapi.json:572` 暴露 `/pages/from-paths`;生成物含 `createPagesFromPaths`/`CreatePagesFromPathsRequest`;`scene.ts:195` `uploadPagesByPaths` 为唯一 UI 调用点(T05A 后已死)。
- **设计**:删路由注册 + handler + request schema + pages.rs 内 from-paths 测试用例(Paths 支路的 ImportIngress::Paths 保留——它是 create_pages_from_paths 的测试驱动面?**不**:ImportIngress::Paths 驱动的是 from-paths handler;删除后 pages.rs 测试仅保留 Multipart 支路,Paths 相关 helper(`create_pages_from_paths` 直接调用)随之清理);删 scene.ts `uploadPagesByPaths`;`bun run generate:api` 重生成 openapi.json + orval 产物,审查 diff。
- **RED 断言**:
  1. `rg 'from-paths|fromPaths|createPagesFromPaths' crates/koharu-rpc` → 当前有路由/schema/测试命中(GREEN 后零命中)
  2. `rg '/pages/from-paths' ui/openapi.json` → 当前命中(GREEN 后零);`check:generated` 重生成审查
  3. `rg 'uploadPagesByPaths' ui/lib ui/components` → 当前 scene.ts 命中(GREEN 后零)
  4. 锁:`bun cargo test -p koharu-rpc page_import_budget` 及 multipart 导入用例 GREEN 后仍全过(multipart 路径不受影响)
- **目标文件**:上表 T05B 行
- **验收命令**:`bun cargo test -p koharu-rpc`、`bun run check:generated`、`rg 'from-paths|uploadPagesByPaths'`(全仓运行时零调用)
- **证据记录**:rg RED/GREEN 输出 / 重生成 diff 摘要 / commit SHA
- **证据(T05B 收口,2026-08-14)**:
  - RED(rg 命中):`pages.rs` 注册行/handler/schema/测试调用 5 处;`ui/openapi.json:572` 路径;`openapi_paths_snapshot.snap:150`;`scene.ts` `uploadPagesByPaths` + `createPagesFromPaths` 生成物引用
  - GREEN:`grep -rn 'from-paths|createPagesFromPaths|uploadPagesByPaths' crates/koharu-rpc ui/lib ui/openapi.json` → **零命中**;rpc suite 32P/0F;`page_import_budget` 6/6 锁 PASS;`check:generated` 提交后复跑零漂移;clippy/fmt 净
  - 重生成 diff:纯删除 79 行(openapi.json -44、generated.ts -24、schemas 索引 -1、createPagesFromPathsRequest.ts 删除);`ui/lib/api/index.ts` 手写 barrel 删 1 行
  - 连带清理:测试 harness `ImportIngress::Paths` 变体 + `run_import` Paths 分支 + 5 个 path-* 用例;`run_import` 死参 `project_dir` 删除
  - Commit:`c1fd37d2`(8 文件,+4/-347)

---

## Lane 收口门禁(Wave 4 gate 对齐)

- `bun cargo test -p koharu-rpc`、`-p koharu-app`、`-p koharu-llm` 全绿
- `bun cargo clippy --workspace --all-targets -- -D warnings`、`bun cargo fmt --all -- --check`
- `bun cargo check --workspace --all-targets`
- `bun run check:generated`(Orval 重生成且审查 diff——T05B 有 OpenAPI 面删除)
- `bun run test:ui`、`bun run lint:ui`、`bun run format:check`
- 独立 scoped code-review 零发现(重试子代理;故障则对抗性自审并落档偏差)
- Tauri picker→File→multipart 与 from-paths 消亡可重复演示(测试输出 + rg 证据)

**Lane 收口证据(2026-08-14)**:

- 门禁:`bun cargo test -p koharu-rpc -p koharu-app -p koharu-llm` → exit 0;`clippy --workspace --all-targets -D warnings` → exit 0;`fmt --all --check` → exit 0;`check --workspace --all-targets` → exit 0;`bun run check:generated` → 零漂移;`bun run --cwd ui test` → 235 passed;`lint:ui` → exit 0
- 独立 review(偏差记录):oracle 第 11 次启动失败 → 对抗性自审(`e1770ba3..c1fd37d2`):**零 blocker/major/minor**;关键核查——被删 handler 的 replace-empty clear 分支在 UI 侧无调用方(importPages 对空数组提前 return),死语义随死代码删除;`readTauriFiles` 路径分隔符/mime 映射为 `.khr` 导入既有验证代码;生成物全部经 `generate:api` 重生成无手改。**四条 lane 独立 review 拖欠,待子代理修复后一并补**
- 依赖传播:AR07-T03 需 T05A(✅)+AR08-T02(未做)→ 维持 🔴;无新 READY
- 可重复演示:openFiles/pagesIo 13 测试 + rg 零命中输出 + 重生成纯删除 diff

## 风险与决策点(批准时一并确认)

- `ImagePickerResult` 类型坍缩:消费者唯一(pagesIo.importPages),直接改签名返回 `File[]`,不留兼容壳(Phase 3 无兼容层原则)
- pages.rs 测试的 `ImportIngress::Paths` 支路随 handler 删除而清理;multipart 支路完整保留
- 三平台 dialog 临时 scope smoke(TASKS 提及)无法在本机三平台实测——以 `readTauriFiles` 复用既有 `.khr` 导入已验证的 `readFile` 路径为据,落档说明
- 生成物重生成单独成步,diff 审查后随 T05B 提交
