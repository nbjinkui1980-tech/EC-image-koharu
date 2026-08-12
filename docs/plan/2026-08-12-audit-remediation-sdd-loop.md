# 审计修复 SDD Phase 3 — Loop 执行驱动文档

**状态：DRAFT/REVIEW — 2026-08-12 创建；Loop 空闲，当前无在途 lane。docs-only checkpoint 提交（本地）后转 ACTIVE。**
**规格：** `docs/plan/2026-08-10-audit-remediation-sdd-spec.md`
**计划：** `docs/plan/2026-08-10-audit-remediation-sdd-plan.md`
**任务：** `docs/plan/2026-08-10-audit-remediation-sdd-tasks.md`
**基线盘点日期：** 2026-08-12(HEAD `fc737092`,main 领先 origin/main 1 提交)

本文档是 Phase 3 剩余 47 张任务卡的**唯一执行入口**(DRAFT 期间不得起动任何 lane)。任何会话(人或 AI)按 §7 引导指令接管后,依 §1 循环协议逐 lane 推进,并在本文档 §3/§8 就地更新状态。**本文档不替代 TASKS 的验收标准;卡片验收以 TASKS 原文为准。**

---

## §0 常青约束(任何 loop 迭代不得违反)

1. **远端零同步**:无用户明确文字指示(如"推送""发布"),不得 `git push`、打 tag、触发 release、修改远端任何状态。所有提交只留本地。
2. **CI 线暂停**:GitHub Actions 调试/release 工作由用户于 2026-08-12 明确暂停。AR10 整条 lane(触碰 `.github/workflows/`)起动前必须先获得用户对该 lane 的明确许可。
3. **Cargo 纪律**:一律 `bun cargo ...`;共享 `KOHARU_SHARED_TARGET_DIR`;不得覆盖 `CARGO_TARGET_DIR`;不得在 `/tmp`、`/private/tmp` 建 target。
4. **生成物纪律**:不得手改 `ui/lib/api/generated.ts` 与生成 schema;改 OpenAPI 源或 Orval 配置后跑 `bun run check:generated` 验证无漂移。
5. **执行协议**(承 TASKS §2):每卡固定 RED-0 → RED-1 → GREEN-1 → GREEN-2;不得弱化、skip、retry 或改快照接受旧行为;拒绝路径不得残留部分 scene/history/file/job、根外读取或 secret 泄漏。
6. **规模纪律**:单卡预计超 5 个文件 → 停下回 TASKS 拆分;需改认证方案/批准预算/格式/发布主体 → 回 SPEC;需批准范围外新依赖或通用框架 → 回 PLAN。
7. **串行点**:format/audit/lockfile/Orval/Next build 只允许单一 owner 串行;并行 Rust 卡共用受保护 target。
8. **提交纪律**:conventional prefix(`feat:`/`fix:`/`refactor:`/`ci:`/`chore(deps):`);AI 协助提交须带真实身份 `Co-Authored-By`;一个 commit 一个目标,不混入无关改动。
9. **凭据门禁**:真实 release tag、GHCR、updater 签名、Winget、生产 Sentry 保持 `PENDING-CREDENTIAL-QA`,未获单独授权不得触碰。
10. **范围外**:AR01-T06(Docker auth smoke)按 AR01 执行合同 §6 出范围,除非用户重新授权并与 AR10-T03 排他调度。

---

## §1 循环协议(每 lane 一圈)

授权节奏:**每 lane 一批准**。lane 内所有卡连续自主执行;lane 间必须停下等批准。

```
LOOP-1  选 lane:按 §2 队列取最优先且依赖就绪的 lane;读取 §3 矩阵确认其卡序。
LOOP-2  起草 lane 执行合同:按 §4 模板,写入
        docs/superpowers/plans/YYYY-MM-DD-arXX-<lane>-contract.md,
        每卡给出 RED 断言、目标文件、验收命令(从 TASKS 原文摘录)。
LOOP-3  ⏸ 向用户提交合同,等明确"批准"。未批准不得写产品代码。
LOOP-3B 登记:批准后、写代码前,立即在 §3"在途 lane 登记表"登记
        (lane/owner 会话/branch/合同 SHA),并把 lane 内卡在矩阵标 🚧。
        同 lane 已有登记行 → 撞车,停止并报告。
LOOP-4  逐卡执行(对 lane 内每张卡,按序):
        a. RED-0:测试 harness 编译/启动成功。
        b. RED-1:写目标断言测试;确认只因目标断言失败(编译/fixture/环境失败不算 RED)。
        c. GREEN-1:最小实现,同字节测试通过。
        d. GREEN-2:相邻模块 suite 通过。
        e. 卡级门禁:按触及域跑 §9 对应命令;全绿。
        f. 提交(单卡或逻辑组一个 commit);在 lane 合同文件记录 RED/GREEN 证据。
LOOP-5  lane 收口:
        a. lane 级完整门禁(§9 全套适用命令)。
        b. 独立审查(对照 AR01 模式:scoped code-reviewer 零发现或修复至零发现)。
        c. 更新本文档:§3 矩阵(完成卡标 ✅,清除在途登记行,重估全部 🔴 卡依赖、传播就绪状态) + §8 日志(证据 commit SHA)。
        d. docs(evidence) 提交。
LOOP-6  ⏸ 向用户汇报 lane 结果,回 LOOP-1。
```

**跨 lane 并行**:文件域不相交的 lane 可在不同会话并行(如 L-AR03 在 `koharu-ai` 与 L-AR11 在 `ui/`),但 §0.7 串行点必须互斥,且每个 lane 独立走 LOOP-3 批准。

---

## §2 Lane 优先级队列

优先级依据:波次收齐 > 关键路径解锁面 > 修正案落地 > 独立 lane。同优先级内文件域不相交者可并行。

| 优先级 | Lane | 卡序 | 解锁价值 | 状态 |
|---|---|---|---|---|
| **P0** | **L-AR03** Provider authority | AR03-T01 → T02 → T03 | W3 收齐 | 🟡 就绪 |
| **P0** | **L-AR04** Durable history | AR04-T02 → T03 | W3 收齐;解锁 AR05-T03/T04 | 🟡 就绪 |
| **P1** | **L-AR05-LIMIT** 体积/批量预算 | AR05-T01 → T06 | 落地 AMEND-02 | 🟡 就绪 |
| **P1** | **L-AR05-PICKER** 导入路径收口 | AR05-T05A → T05B | 落地 AMEND-01(删 `/pages/from-paths`) | 🟡 就绪 |
| **P1** | **L-AR06** Job 生命周期 | AR06-T01 → T02 → T03 ∥ T04 → (T05 等 L-AR05-ARCHIVE) | 任务槽/有界注册表 | 🟡 就绪(T05 🔴) |
| **P1** | **L-AR13B** 边界余量 | AR13-T02 ∥ T03 ∥ T04 | 三张小卡,依赖已齐 | 🟡 就绪 |
| **P2** | **L-AR05-ARCHIVE** 归档预算 | AR05-T02 →(等 L-AR04)→ T03 → T04 | history/archive 预算线 | 🟡 半就绪(T03/T04 🔴 等 AR04-T03) |
| **P2** | **L-AR07** Tauri 攻击面 | AR07-T01 ∥ T02 →(T03 等 T05A+AR08-T02) | CSP/导航/FS scope | 🟡 半就绪(T03 🔴) |
| **P2** | **L-AR08** ZIP 安全 | AR08-T01 → T02 | 解锁 AR07-T03 | 🟡 就绪 |
| **P2** | **L-AR09** 产物完整性 | AR09-T01 → T02 ∥ T03 | 下载 digest 链 | 🟡 就绪 |
| **P3** | **L-AR11** UI 所有权 | AR11-T01~T06(相互独立) | UI 数据一致性 | 🟡 就绪 |
| **P3** | **L-AR12** UI 资源 | AR12-T01~T06(相互独立) | UI 资源生命周期 | 🟡 就绪 |
| **P4** | **L-AR10** 发布 provenance | AR10-T01 → T02 → T03 | 解锁 AR14-T06;⚠️ 须先解除 §0.2 暂停 | 🔴 暂停 |
| **P5** | **L-FINAL** 收敛 | AR14-T06 → FINAL-T01/T02 | 全门禁 + 平台/凭据验收 | 🔴 阻塞 |

**选择规则**:同一时刻最多激活 2 条 lane(一条 Rust 域 + 一条 UI 域)。P 序只约束 Rust 域 lane:P0 未清空不开 P2 及以后(除非用户明确指示跳过);UI 域 lane(L-AR11/L-AR12)不受 P 序限制,可随时与任一 Rust 域 lane 并行。

**分支规则**:每条 lane 从起动时的 main HEAD 创建 `arXX-<lane>` 分支;并行 lane 使用独立 worktree,共享 `KOHARU_SHARED_TARGET_DIR`,不得覆盖 `CARGO_TARGET_DIR`。lane 收口后按用户指示合回 main。TASKS 原文引用的 `codex/audit-remediation-sdd` 分支已不存在,以本规则为准。

**◐ 证据补录**:AR14-T07 的 Linux CI/macOS/Windows 标准布局验证为独立小任务,可随时插入执行,不占 lane。

---

## §3 卡片状态矩阵(loop 就地更新)

状态:✅DONE / ◐DONE 但证据未记录 / 🟡READY(依赖齐) / 🚧IN_PROGRESS(已认领,见 §3 末登记表) / 🔴BLOCKED / ⛔OOS(出范围) / ⏸PAUSED

### W1/W2(除 T07 证据待补外已收齐)

| 卡 | 状态 | 证据 |
|---|---|---|
| AR14-T04 Rust advisory | ✅ | `45b090cf`+`51665c78`,quick-xml=0.41.0 |
| AR14-T05 Next/sharp | ✅ | `5f966821` |
| AR14-T01 Clippy 清零 | ✅ | `0618c39a`+`fbe90ea1` |
| AR14-T02 UI format 基线 | ✅ | `4efc4c4f` |
| AR14-T07 Next/Turbopack 布局 | ◐ | `ee94d0c1`;缺 Linux CI + macOS/Windows 标准布局验证证据,补录后转 ✅ |
| AR13-T01 5xx 脱敏/Sentry PII | ✅ | `88781c76` |
| AR02-T01~T05 BlobRef 全链 | ✅ | `64a026d2`+`f2e31a65` ⚠️ 单提交覆盖五卡,未逐卡 RED/GREEN |
| AR04-T01 Batch 原子 | ✅ | `fb7d5546` |

### W3

| 卡 | 状态 | 证据 | 备注 |
|---|---|---|---|
| AR01-T00~T03 | ✅ | 台账 10 product commits | 见 AR01 证据台账 |
| AR03-T01 Provider URL/authority 规范化 | 🟡 | — | koharu-ai 无任何实现证据 |
| AR03-T02 Config authority 冲突 | 🔴 | — | ←T01 |
| AR03-T03 Redirect/错误脱敏 | 🔴 | — | ←T01 |
| AR04-T02 Apply/Undo/Redo durable commit | 🟡 | — | history.rs 自 08-10 零提交 |
| AR04-T03 损坏尾回滚 fail-stop | 🔴 | — | ←T02 |

### W4

| 卡 | 状态 | 证据 | 备注 |
|---|---|---|---|
| AR01-T04/T04B/T04C/T05 | ✅ | 台账 | |
| AR01-T06 Docker auth smoke | ⛔ | — | 合同 §6 出范围 |
| AR05-T01 Route body limits | 🟡 | — | 现状仅全局 1GiB DefaultBodyLimit(早于本计划,无路由级差异) |
| AR05-T02 Archive 读取预算 | 🟡 | — | 无依赖 |
| AR05-T03 Import 原子发布 | 🔴 | — | ←AR04-T03+T02 |
| AR05-T04 History frame 预算 | 🔴 | — | ←AR04-T03 |
| AR05-T05A Tauri picker File | 🟡 | — | 仅依赖已批准 AMEND-01 |
| AR05-T05B 删除 from-paths API | 🔴 | — | ←T05A;现状 `/pages/from-paths` 仍在 pages.rs |
| AR05-T06 批量预算/decode admission | 🔴 | — | ←T01 |
| AR06-T01 有界 JobRegistry | 🟡 | — | 现状无有界注册表/槽位 |
| AR06-T02 统一 registry | 🔴 | — | ←T01 |
| AR06-T03 Pipeline 单槽 | 🔴 | — | ←T01,T02 |
| AR06-T04 AI 双槽 | 🔴 | — | ←T01,T02 |
| AR06-T05 Bulk import 单槽 | 🔴 | — | ←AR05-T03,T01 |
| AR13-T02 破坏性 Project ID 精确匹配 | 🟡 | — | |
| AR13-T03 Mask 复用 generated API | 🟡 | — | |
| AR13-T04 Export 保留 filename | 🟡 | — | psd_export.rs 无 filename 处理 |

### W5

| 卡 | 状态 | 证据 | 备注 |
|---|---|---|---|
| AR07-T01 Axum+Tauri CSP | 🟡 | — | 现状无任何 CSP 头 |
| AR07-T02 Webview navigation 同源 | 🟡 | — | AR01 台账单独跟踪项 |
| AR07-T03 删除全盘 FS scope | 🔴 | — | ←T05A,AR08-T02;现状 fs:scope=`**` |
| AR08-T01 ZIP entry 路径验证 | 🟡 | — | 现有 zip 代码属 G00x d0 管线,不覆盖本卡 |
| AR08-T02 ZIP 全量预验证/预算 | 🔴 | — | ←T01 |
| AR09-T01 SHA-256 下载/缓存不变量 | 🟡 | — | 模型下载无 digest 校验 |
| AR09-T02 llama/ZLUDA 描述 | 🔴 | — | ←T01 |
| AR09-T03 CUDA PyPI digest | 🔴 | — | ←T01 |
| AR10-T01 Actions 固定完整 SHA | ⏸ | — | 现状 `@v7`/`@master`;须先解除 §0.2 |
| AR10-T02 Release 最小权限/签名 digest | 🔴 | — | ←T01 |
| AR10-T03 同 run artifact/Docker provenance | 🔴 | — | ←T02;独占 Dockerfile,与 AR01-T06 互斥 |
| AR11-T01 Mask bitmap 页面代次 | 🟡 | — | lane 内相互独立 |
| AR11-T02 Config 保存失败与乱序 | 🟡 | — | lane 内相互独立 |
| AR11-T03 Style mutation lossless queue | 🟡 | — | lane 内相互独立 |
| AR11-T04 Auto-render project/page 隔离 | 🟡 | — | lane 内相互独立 |
| AR11-T05 Scene 临时错误保留旧数据 | 🟡 | — | lane 内相互独立 |
| AR11-T06 Verification URL allowlist | 🟡 | — | lane 内相互独立 |
| AR12-T01 Query 缓存 bytes,组件拥有 URL | 🟡 | — | lane 内相互独立 |
| AR12-T02 FontFace owner | 🟡 | — | lane 内相互独立 |
| AR12-T03 UI jobs/downloads retention | 🟡 | — | lane 内相互独立 |
| AR12-T04 Updater cleanup | 🟡 | — | lane 内相互独立 |
| AR12-T05 文本输入原生 undo/redo | 🟡 | — | lane 内相互独立 |
| AR12-T06 字体收藏与删除按钮 a11y | 🟡 | — | lane 内相互独立 |

*AR11/AR12 十二卡在各自 lane 内相互独立,可任意排序执行;自 08-10 起 UI 域零修复提交。*

### W6/FINAL

| 卡 | 状态 | 证据 | 备注 |
|---|---|---|---|
| AR14-T03A Rust format 分片 A | ✅ | `676603f4` | ⚠️ 五片单提交覆盖,未逐片 RED/GREEN |
| AR14-T03B Rust format 分片 B | ✅ | `676603f4` | ⚠️ 同上 |
| AR14-T03C Rust format 分片 C | ✅ | `676603f4` | ⚠️ 同上 |
| AR14-T03D Rust format 分片 D | ✅ | `676603f4` | ⚠️ 同上 |
| AR14-T03E Rust format 分片 E | ✅ | `676603f4` | ⚠️ 同上 |
| AR14-T06 CI 完整门禁 | 🔴 | — | ←AR10-T03 |
| FINAL-T01 单一 verifier 全门禁 | 🔴 | — | |
| FINAL-T02 平台与凭据状态 | 🔴 | — | 凭据项 PENDING-CREDENTIAL-QA |

**计数:✅24 / ◐1 / 🟡25 / 🔴20 / ⛔1 / ⏸1(共 72;待处理 47 = 72 − 24 ✅ − 1 ⛔)**

### 在途 lane 登记表(lane 认领的唯一事实来源)

| Lane | Owner 会话 | Branch/Worktree | 合同 SHA | 登记时间 |
|---|---|---|---|---|

规则:LOOP-3 合同批准后、LOOP-4 写代码前必须先登记,并把 lane 内卡标 🚧;未登记视为未认领。登记即互斥:同 lane 出现第二行登记 = 撞车,后来者停止并报告。lane 收口(LOOP-5c)时清除登记行。

---

## §4 Lane 执行合同模板

每 lane 一份,命名 `docs/superpowers/plans/YYYY-MM-DD-arXX-<lane>-contract.md`:

```markdown
# AR-XX <lane 名> 执行合同

- 来源:docs/plan/2026-08-12-audit-remediation-sdd-loop.md §2 队列
- 卡序:ARxx-T01 → ...(含每卡依赖确认)
- 范围文件域:(列出允许触碰的目录/文件;域外改动禁止)
- 串行点声明:(本 lane 是否占用 lockfile/format/Orval/Next build)

## 卡:ARxx-T0N <标题>
- 验收标准:(从 TASKS 原文摘录)
- RED 断言:(具体测试名 + 预期失败断言)
- 目标文件:(≤5;超限先回 TASKS 拆分)
- 验收命令:(TASKS 原文命令)
- 证据记录:RED 输出摘要 / GREEN 输出摘要 / commit SHA
```

---

## §5 授权模型

| 动作 | 授权要求 |
|---|---|
| 读取文档、选 lane、起草合同 | 无需授权 |
| 起动一条 lane(写产品/测试代码) | **用户明确批准该 lane 合同** |
| lane 内逐卡 RED→GREEN、门禁、本地提交 | 合同批准后自主 |
| 起动 P4 L-AR10(触 workflows) | **单独明确许可**(解除 §0.2) |
| push / tag / release / 远端任何变更 | **单独明确许可**(§0.1) |
| 凭据门禁项(GHCR/签名/Winget/生产 Sentry) | **单独明确许可**(§0.9) |
| 回 SPEC/PLAN/TASKS 修订 | 向用户提出,等批准 |

---

## §6 停止与回滚

任一条件成立立即停 loop 并报告:

- 同一卡连续 3 次 RED/GREEN 循环未收敛 → 停止并报告根因分析。回滚方式:已提交卡用 `git revert <card-commit>`;未提交卡只允许逆转已确认归属当前卡的精确 patch(先 `git diff` 存证),**禁止 `git checkout -- <files>` 整文件覆盖**(会误伤用户或并行 lane 的未提交改动)。
- 发现 TASKS 验收标准与现状矛盾、或依赖卡实际未完成 → 冻结该 lane,更新 §3 矩阵,报告。
- 门禁出现与本 lane 无关的既有失败 → 不修复、不弱化,记录并继续;与本 lane 相关的失败必须修复至绿。
- 任何 §0 约束将被违反 → 停止,等用户指示。

回滚粒度:单卡 = 该卡 commit 范围;lane = 该 lane 全部 commit(均只在本地,revert 而非 force-push)。

---

## §7 新会话引导指令(直接可执行)

新会话接管 loop 时,把以下指令作为首条消息粘贴:

```
阅读 docs/plan/2026-08-12-audit-remediation-sdd-loop.md 并接管 loop:
1. 以 §3 矩阵与 §8 日志为当前状态,不要重新全盘盘点。
2. 先查 §3"在途 lane 登记表"排除已认领 lane,再按 §2 队列与"选择规则"确定下一条 lane,按 §4 模板起草 lane 执行合同交我批准。
3. 严格遵循 §0 常青约束与 §1 循环协议;lane 内自主,lane 间等我批准。
4. 未经我明确许可:不起新 lane、不动 workflows、不 push/tag/release。
```

---

## §8 Loop 日志(每次迭代追加一行)

| 日期 | Lane | 动作 | 结果/证据 |
|---|---|---|---|
| 2026-08-12 | — | 基线盘点 + 本文档创建 | HEAD `fc737092`;✅24/◐1/🟡25/🔴20/⛔1/⏸1(共 72,经两轮审查核定) |
| 2026-08-12 | — | 五轴自审修复(4 必修 + 1 Nit) | C1 计数基线、I2 选择规则域优先、I3 AR11/AR12 拆行、I4 LOOP-5c 就绪传播、N5 AR05-T01 措辞 |
| 2026-08-12 | — | 用户审查修复(3 HIGH + 2 MEDIUM) | HIGH-1 状态转 DRAFT(待 checkpoint)、HIGH-2 lane 认领登记+分支规则、HIGH-3 禁止整文件 checkout、MEDIUM-1 T03A~E 展开五卡、MEDIUM-2 T07 降级 ◐;计数 → ✅24/◐1/🟡25/🔴20/⛔1/⏸1(共 72) |

---

## §9 门禁命令速查

| 域 | 命令 |
|---|---|
| Rust 检查 | `bun cargo check --workspace --all-targets` |
| Rust lint | `bun cargo clippy --workspace --all-targets -- -D warnings` |
| Rust 格式 | `bun cargo fmt --all -- --check` |
| Rust 定向测试 | `bun cargo test -p <crate> <filter>` |
| Rust 全量测试 | `bun cargo test --workspace --tests` |
| Rust audit | `bun cargo audit` |
| UI 测试 | `bun run test:ui` |
| UI lint | `bun run lint:ui` |
| UI 格式 | `bun run format:check` |
| UI build | `bun run --cwd ui build` |
| 生成物漂移 | `bun run check:generated` |
| Desktop build | `KOHARU_CARGO_GUARD_ACTIVE=1 bun run build` |
| 文件级格式化 | `bunx oxfmt <file>` |
