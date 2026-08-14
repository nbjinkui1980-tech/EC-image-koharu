# Lane 执行合同:L-AR13B — 边界余量(Project ID 精确匹配 ∥ Mask generated API ∥ Export 保留 filename)

- 状态:**Phase 3 一次批准已覆盖(2026-08-13 授予)**;LOOP-3 本地认领已登记
- 认领基线:Phase 3 起点 main `b68f123e`;执行分支 `audit-remediation-phase3`(认领时 tip `64614e01`)
- 提交/回滚单元:T02/T03/T04 各一个单卡 commit(可独立 revert);lane 收口 docs(evidence) 单独一个 commit
- 前置依赖:AR13-T01 ✅(`88781c76`);AR01-T03 ✅(含于 AR01-T00~T03)
- 执行环境偏差(继承):子代理模型映射故障未修(12 次失败),codegraph + 直接工具单执行器;lane 收口独立 review 重试,失败则对抗性自审并落档

## 范围文件域(域外改动禁止)

| 卡 | 允许文件 |
|---|---|
| AR13-T02 | `crates/koharu-app/src/projects.rs`、`crates/koharu-rpc/src/routes/projects.rs`(≤2) |
| AR13-T03 | `ui/hooks/useMaskDrawing.ts`、`ui/tests/hooks/useMaskDrawing.test.tsx`(≤2) |
| AR13-T04 | `ui/lib/io/scene.ts`、`ui/lib/api/fetch.ts`、`ui/orval.config.ts`、生成物(仅 `bun run generate:api`)、`ui/tests/lib/io/scene.test.ts`(≤4+生成物) |

## 卡:AR13-T02 — 破坏性 Project ID 精确匹配

- **验收标准(TASKS 原文)**:RED:`my-project!`、大小写、空格、encoded separator 可作用于 `my-project`。GREEN:create display name 仍 slugify;open/delete 只接受精确 canonical ID。
- **现状(RED-0 源码实证)**:`projects.rs:33` `project_path(config, id)` 对任意 id `slugify(id)` 后命中 `{slug}.khrproj`——`My Project!`/大小写/空格输入经规范化后作用于 `my-project` 项目(open/delete 同路)。
- **设计**:`project_path` 改为**精确匹配**:拒绝任何 `slugify(id) != id` 或空 id 的输入(400);`allocate_named`/`allocate_imported` 保留 slugify(create 面不变)。
- **RED 断言**(App projects + RPC project tests):
  1. `project_path_rejects_non_canonical_id` — `my-project!`、`My-Project`、`my project`、`my%2Fproject` → Err;当前 slugify 后 Ok → FAIL
  2. `project_path_accepts_exact_canonical_id` — 锁:`my-project` → Ok
  3. RPC 锁:open/delete 以非 canonical id → 4xx;canonical → 正常
- **目标文件**:上表 T02 行(≤2)
- **验收命令**:`bun cargo test -p koharu-app projects`、`bun cargo test -p koharu-rpc project`
- **证据记录**:RED / GREEN / commit SHA

## 卡:AR13-T03 — Mask 复用 generated API

- **验收标准(TASKS 原文)**:RED:mask 仍走重复 raw fetch。GREEN:复用 generated `putMask`;错误与 scene invalidation 保持。
- **现状(RED-0 源码实证)**:`useMaskDrawing.ts:98` `fetchWithAuth('/api/v1/pages/${page.id}/masks/segment?...')` raw fetch;generated `putMask(id, role, putMaskBody?, params?, options?)` 已存在(generated.ts:472-485)。
- **设计**:hook 改调 generated `putMask`;保持 ApiError 抛错形态与 invalidateScene 时序;测试断言走 generated 调用(mock/MSW 面不变,PUT /pages/{id}/masks/{role} 同一路径)。
- **RED 断言**(`ui/tests/hooks/useMaskDrawing.test.tsx`):
  1. mask 提交走 generated `putMask`(vi.mock `@/lib/api` 的 putMask 被调,raw fetchWithAuth 不被调)→ 当前 raw fetch → FAIL
  2. 锁:错误传播 + scene invalidation 断言(现有语义)
- **目标文件**:上表 T03 行(≤2)
- **验收命令**:`bun run --cwd ui test -- tests/hooks/useMaskDrawing.test.tsx`、`bun run lint:ui`
- **证据记录**:RED / GREEN / commit SHA

## 卡:AR13-T04 — Export generated API 保留 filename

- **验收标准(TASKS 原文)**:RED:generated 调用丢 `Content-Disposition` 或 blob type。GREEN:由生成器保留完整 Response;不手改 `generated.ts`。停止:Orval 无法保留 headers 时回 SPEC,不得静默丢 filename。
- **停止条件裁决(侦查取证,2026-08-14)**:orval 支持 per-operation mutator(`override.operations.<op>.mutator`),且 mutator 完全接管 body 反序列化(orval packages/fetch/src/index.ts 实证)→ **可行,不回 SPEC**。
- **现状(RED-0 源码实证)**:`orval.config.ts` `includeHttpResponseReturnType: false`(全局,改 true 会波及所有调用点,不可取);`scene.ts` `exportProject` 手写 `fetchWithAuth` 读 Content-Disposition/blob(在 generated 之外,属重复 raw fetch 同类问题)。
- **设计**:orval.config.ts 加 `override.operations.exportCurrentProject.mutator = { path: './lib/api/fetch.ts', name: 'fetchApiFullResponse' }`;fetch.ts 新增 `fetchApiFullResponse`(返回含 headers + blob 的完整结果,类型由生成处泛型对齐);`scene.ts` `exportProject` 改调 generated `exportCurrentProject`(mutator 产出),保留 Content-Disposition 文件名逻辑;`generate:api` 重生成,审查 generated.ts diff(仅 exportCurrentProject 实现变化)。
- **RED 断言**(`ui/tests/lib/io/scene.test.ts` + rg):
  1. `scene.ts` 中 `exportProject` 不再使用 `fetchWithAuth`/`getExportCurrentProjectUrl` 手写调用 → 当前仍用 → FAIL(rg 证据)
  2. export 行为锁:msw 返回带 Content-Disposition 的响应 → filename 保留 + blob type 正确(现有 scene.test.ts 用例语义,适配新调用面)
- **目标文件**:上表 T04 行
- **验收命令**:`bun run --cwd ui test -- tests/lib/io/scene.test.ts`、`bun run check:generated`、`bun run lint:ui`
- **证据记录**:RED / GREEN / generated diff 摘要 / commit SHA

---

## Lane 收口门禁(Wave 4 gate 对齐)

- `bun cargo test -p koharu-app`、`-p koharu-rpc` 全绿
- `bun cargo clippy --workspace --all-targets -- -D warnings`、`bun cargo fmt --all -- --check`
- `bun cargo check --workspace --all-targets`
- `bun run check:generated`(T04 重生成审查)
- `bun run test:ui`、`bun run lint:ui`、`bun run format:check`
- 独立 scoped code-review 零发现(重试子代理;故障则对抗性自审并落档偏差)
- 精确匹配拒绝/mask generated 调用/export filename 保留可重复演示

## 风险与决策点(批准时一并确认)

- T02 精确匹配边界:`slugify(id) == id` 即 canonical(小写、连字符、无空格/符号);既有 canonical id 全部满足(slugify 幂等),无迁移面
- T04 mutator 接管反序列化:exportCurrentProject 的 200 body 为二进制(orval 类型参数可能是 Blob/unknown)——mutator 内部自读 headers+blob,返回 `{ blob, headers }`;generated diff 审查确认唯一变化
- T03 putMask 的 params/Body 形态以 generated 签名为准;mask 端点行为不变(T01 的 64 MiB tier 不受影响)
- 三卡相互独立(∥),单执行器按 T02→T03→T04 串行
