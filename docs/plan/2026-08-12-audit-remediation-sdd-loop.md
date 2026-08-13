# 审计修复 SDD Phase 3 — Loop 执行驱动文档

**状态：ACTIVE/AUTO — Phase 3 一次批准已于 2026-08-13 授予；Loop 空闲，当前无在途 lane。**
**规格：** `docs/plan/2026-08-10-audit-remediation-sdd-spec.md`
**计划：** `docs/plan/2026-08-10-audit-remediation-sdd-plan.md`
**任务：** `docs/plan/2026-08-10-audit-remediation-sdd-tasks.md`
**基线盘点日期：** 2026-08-12(HEAD `fc737092`,main 领先 origin/main 1 提交)

本文档是 Phase 3 剩余 57 张任务卡的**唯一执行入口**。任何会话(人或 AI)按 §7 引导指令接管后,依 §1 循环协议逐 lane 推进,并在本文档 §3/§8 就地更新状态。**本文档不替代 TASKS 的验收标准;卡片验收以 TASKS 原文为准。**

---

## §0 常青约束(任何 loop 迭代不得违反)

1. **远端零同步**:无用户明确文字指示(如"推送""发布"),不得 `git push`、打 tag、触发 release、修改远端任何状态。所有提交只留本地。
2. **CI 远端暂停**:AR10 可在 Phase 3 一次授权下本地修改 `.github/workflows/`,但不得 push、触发或调试远端 GitHub Actions/release。
3. **Cargo 纪律**:一律 `bun cargo ...`;共享 `KOHARU_SHARED_TARGET_DIR`;不得覆盖 `CARGO_TARGET_DIR`;不得在 `/tmp`、`/private/tmp` 建 target。
4. **生成物纪律**:不得手改 `ui/lib/api/generated.ts` 与生成 schema;改 OpenAPI 源或 Orval 配置后跑 `bun run check:generated` 验证无漂移。
5. **执行协议**(承 TASKS §2):每卡固定 RED-0 → RED-1 → GREEN-1 → GREEN-2;不得弱化、skip、retry 或改快照接受旧行为;拒绝路径不得残留部分 scene/history/file/job、根外读取或 secret 泄漏。
6. **规模纪律**:单卡预计超 5 个文件 → 停下回 TASKS 拆分;需改认证方案/批准预算/格式/发布主体 → 回 SPEC;需批准范围外新依赖或通用框架 → 回 PLAN。
7. **串行点**:format/audit/lockfile/Orval/Next build 只允许单一 owner 串行;所有 Rust 卡共用受保护 target,不并行运行 Cargo 检查/测试。
8. **提交纪律**:conventional prefix(`feat:`/`fix:`/`refactor:`/`ci:`/`chore(deps):`);AI 协助提交须带真实身份 `Co-Authored-By`;一个 commit 一个目标,不混入无关改动。
9. **凭据门禁**:真实 release tag、GHCR、updater 签名、Winget、生产 Sentry 保持 `PENDING-CREDENTIAL-QA`,未获单独授权不得触碰。
10. **范围外**:AR01-T06(Docker auth smoke)按 AR01 执行合同 §6 出范围,除非用户重新授权并与 AR10-T03 排他调度。

---

## §1 循环协议(每 lane 一圈)

授权节奏:**Phase 3 一次批准**。用户已于 2026-08-13 批准全部 lane 按本文档自动串行执行;lane 完成后只在本地执行分支提交、更新台账并立即进入下一条 lane,不再逐 lane 请求批准。

```
LOOP-0  执行分支:从批准时的 main 基线创建或接管唯一本地分支
        audit-remediation-phase3。产品、测试、合同与台账均只提交到该分支;
        整个 loop 不合入 main,不与远端同步。
LOOP-1  选 lane:按 §2 队列取最优先且依赖就绪的 lane;读取 §3 矩阵确认其卡序。
LOOP-2  起草 lane 执行合同:按 §4 模板,写入
        docs/superpowers/plans/YYYY-MM-DD-arXX-<lane>-contract.md,
        每卡给出 RED 断言、目标文件、验收命令(从 TASKS 原文摘录)。
        合同在既定范围内时直接继续;只有触发 §0/§6 停止条件才暂停。
LOOP-3  本地认领:在 audit-remediation-phase3 上登记当前 lane,将对应卡标 🚧,
        并把合同+台账作为 docs-only 本地提交。唯一执行器同时只能认领一条 lane。
LOOP-4  逐卡执行(对 lane 内每张卡,按序):
        a. RED-0:测试 harness 编译/启动成功。
        b. RED-1:写目标断言测试;确认只因目标断言失败(编译/fixture/环境失败不算 RED)。
        c. GREEN-1:最小实现,同字节测试通过。
        d. GREEN-2:相邻模块 suite 通过。
        e. 卡级门禁:按触及域跑 §9 对应命令;全绿。
        f. 提交(单卡或合同中明确冻结的逻辑组一个 commit);逻辑组是不可拆回滚单元;
           在 lane 合同文件逐卡记录 RED/GREEN 证据。
LOOP-5  lane 收口:
        a. lane 级完整门禁(§9 全套适用命令)。
        b. 独立审查(对照 AR01 模式:scoped code-reviewer 零发现或修复至零发现)。
        c. 确认产品/测试 commit 均在本地执行分支;不合入 main,不 push。
        d. 若该 lane 收齐某一 wave,由唯一 verifier 在当前执行分支 SHA 追加 WAVE-GREEN。
        e. 仅当 a~d 全绿,才在执行分支把卡标 ✅、清除认领行、重估依赖并传播 READY,
           同步 §3 波次门禁表与 §8 日志,提交 docs(evidence)。失败则保持 🚧,不得解锁后继。
LOOP-6  自动回 LOOP-1,不等待用户批准。只在全部可执行 lane 完成或触发停止条件时汇报。
```

**跨 lane 串行**:为保持单一本地台账、依赖基线和共享 Cargo target 稳定,同一时刻只执行一条 lane;IDE 后台检查与定向测试也串行。

---

## §2 Lane 优先级队列

优先级依据:波次收齐 > 关键路径解锁面 > 修正案落地 > 独立 lane。同优先级按表中顺序串行。

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
| **P4** | **L-AR10** 发布 provenance | AR10-T01 → T02 → T03 | 解锁 AR14-T06;仅本地修改/验证 | 🟡 就绪 |
| **P5** | **L-FINAL** 收敛 | 全部非 OOS 卡已集成执行分支 + W1~W6 PASS → FINAL-T01 → FINAL-T02 | 全门禁 + 平台/凭据验收 | 🔴 阻塞 |

**选择规则**:同一时刻只激活 1 条 lane。每次从依赖已齐的 lane 中按 P0→P5 选最高优先级;同级按表中顺序。当前 lane 收口并提交台账后自动选下一条。

**分支规则**:全部 lane 在唯一本地 `audit-remediation-phase3` 分支上顺序累积,不为每条 lane 创建独立分支或 worktree。该分支内的本文档是执行主账;main 在整个 loop 期间保持不变。所有 Rust 命令共享 `KOHARU_SHARED_TARGET_DIR`,不得覆盖 `CARGO_TARGET_DIR`。全部收口后只报告分支 tip,等待用户另行下达合并 main 指令。TASKS 原文引用的 `codex/audit-remediation-sdd` 分支已不存在,以本规则为准。

**◐ 证据补录**:AR14-T07 的 Linux CI/macOS/Windows 标准布局验证为独立小任务,可随时插入执行,不占 lane。

---

## §3 卡片状态矩阵(loop 就地更新)

状态:✅DONE(实现和强制证据均已集成 `audit-remediation-phase3`) / ◐IMPLEMENTED-EVIDENCE-PENDING(不满足依赖) / 🟡READY(依赖齐) / 🚧IN_PROGRESS(已认领,见 §3 末登记表) / 🔴BLOCKED / ⛔OOS(出范围) / ⏸PAUSED

### W1/W2(证据待补卡不计 DONE)

| 卡 | 状态 | 证据 |
|---|---|---|
| AR14-T04 Rust advisory | ✅ | `45b090cf`+`51665c78`,quick-xml=0.41.0 |
| AR14-T05 Next/sharp | ✅ | `5f966821` |
| AR14-T01 Clippy 清零 | ✅ | `0618c39a`+`fbe90ea1` |
| AR14-T02 UI format 基线 | ✅ | `4efc4c4f` |
| AR14-T07 Next/Turbopack 布局 | ◐ | `ee94d0c1`;缺 Linux CI + macOS/Windows 标准布局验证证据,补录后转 ✅ |
| AR13-T01 5xx 脱敏/Sentry PII | ✅ | `88781c76` |
| AR02-T01~T05 BlobRef 全链 | ◐ | `64a026d2`+`f2e31a65`;五卡实现已集成,但缺逐卡 RED/GREEN/WAVE-GREEN 证据 |
| AR04-T01 Batch 原子 | ✅ | `fb7d5546` |

### W3

| 卡 | 状态 | 证据 | 备注 |
|---|---|---|---|
| AR01-T00~T03 | ✅ | 台账 10 product commits | 见 AR01 证据台账 |
| AR03-T01 Provider URL/authority 规范化 | 🚧 | — | L-AR03 在途,合同 `22565edc` |
| AR03-T02 Config authority 冲突 | 🚧 | — | ←T01;L-AR03 在途 |
| AR03-T03 Redirect/错误脱敏 | 🚧 | — | ←T01;L-AR03 在途 |
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
| AR10-T01 Actions 固定完整 SHA | 🟡 | — | 现状 `@v7`/`@master`;仅本地修改/验证 |
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
| AR14-T03A Rust format 分片 A | ◐ | `676603f4` | 五片实现已集成,缺逐片失败命令/GREEN 证据 |
| AR14-T03B Rust format 分片 B | ◐ | `676603f4` | 同上 |
| AR14-T03C Rust format 分片 C | ◐ | `676603f4` | 同上 |
| AR14-T03D Rust format 分片 D | ◐ | `676603f4` | 同上 |
| AR14-T03E Rust format 分片 E | ◐ | `676603f4` | 同上 |
| AR14-T06 CI 完整门禁 | 🔴 | — | ←AR10-T03 |
| FINAL-T01 单一 verifier 全门禁 | 🔴 | — | |
| FINAL-T02 平台与凭据状态 | 🔴 | — | 凭据项 PENDING-CREDENTIAL-QA |

**计数:✅14 / ◐11 / 🟡26 / 🔴20 / ⛔1 / ⏸0(共 72;待处理 57 = 72 − 14 ✅ − 1 ⛔)**

### 波次门禁登记表

只有某 wave 的全部非 OOS 卡均在 `audit-remediation-phase3` 上为 ✅,且唯一 verifier 在该执行分支 SHA 上执行 TASKS 对应 WAVE-GREEN 后,该 wave 才能标 PASS。◐、🚧、🟡、🔴、⏸ 均阻止 PASS。记录必须包含 verifier、执行分支 SHA、命令、退出码和结果摘要。

| Wave | 状态 | 执行分支 SHA | Verifier / 证据 |
|---|---|---|---|
| W1 | PENDING | — | AR14-T07 为 ◐,尚未闭环 |
| W2 | PENDING | — | AR02-T01~T05 为 ◐,尚未闭环 |
| W3 | PENDING | — | 尚有未完成卡 |
| W4 | PENDING | — | 尚有未完成卡 |
| W5 | PENDING | — | 尚有未完成卡 |
| W6 | PENDING | — | AR14-T03A~E 为 ◐,AR14-T06 未完成 |

FINAL-T01 只有在全部非 OOS 卡为执行分支上的 ✅ 且 W1~W6 全部 PASS 后才可转 🟡;FINAL-T02 依赖 FINAL-T01 完成。凭据门禁项仍按 §0.9 保持 `PENDING-CREDENTIAL-QA`。

### 在途 lane 登记表(lane 认领的唯一事实来源)

| Lane | Owner 会话 | 执行分支 | 合同 SHA | 登记时间 |
|---|---|---|---|---|
| L-AR03 | ses_0097e4b0(Sisyphus/ulw) | `audit-remediation-phase3` | `22565edc71c01579` | 2026-08-13 |

规则:唯一执行器在 LOOP-3 写代码前,于 `audit-remediation-phase3` 主账登记当前 lane 并标 🚧。新会话接管时必须从该分支读取此表;同时只允许一行。仅 LOOP-5e 可在门禁全绿后清除登记并标 ✅。

---

## §4 Lane 执行合同模板

每 lane 一份,命名 `docs/superpowers/plans/YYYY-MM-DD-arXX-<lane>-contract.md`:

```markdown
# AR-XX <lane 名> 执行合同

- 来源:docs/plan/2026-08-12-audit-remediation-sdd-loop.md §2 队列
- 认领基线:Phase 3 起点 main SHA + 当前执行分支 SHA
- 卡序:ARxx-T01 → ...(含每卡依赖确认)
- 范围文件域:(列出允许触碰的目录/文件;域外改动禁止)
- 串行点声明:(本 lane 是否占用 lockfile/format/Orval/Next build)
- 提交/回滚单元:(单卡 commit;或列出合同冻结的逻辑组,组内卡不可分拆回滚)

## 卡:ARxx-T0N <标题>
- 验收标准:(从 TASKS 原文摘录)
- RED 断言:(具体测试名 + 预期失败断言)
- 目标文件:(≤5;超限先回 TASKS 拆分)
- 验收命令:(TASKS 原文命令)
- 证据记录:RED-0 / RED-1 / GREEN-1 / GREEN-2 的精确命令、退出码和结果摘要;本卡 diff SHA / commit SHA
```

---

## §5 授权模型

| 动作 | 授权要求 |
|---|---|
| 读取文档、选 lane、起草合同 | Phase 3 一次批准已覆盖 |
| 起动任意 lane(写产品/测试/本地 workflows) | Phase 3 一次批准已覆盖;不逐 lane 请求 |
| lane 内逐卡 RED→GREEN、门禁、本地提交、台账更新、进入下一 lane | 自动连续执行 |
| 合并 main | **本 loop 禁止**;全部收口后等用户另行指令 |
| push / tag / release / 远端任何变更 | **单独明确许可**(§0.1) |
| 凭据门禁项(GHCR/签名/Winget/生产 Sentry) | **单独明确许可**(§0.9) |
| 回 SPEC/PLAN/TASKS 修订 | 向用户提出,等批准 |

---

## §6 停止与回滚

任一条件成立立即停 loop 并报告:

- 同一卡连续 3 次 RED/GREEN 循环未收敛 → 停止并报告根因分析。未提交变更只允许逆转已确认归属当前卡的精确 patch(先 `git diff` 存证),**禁止 `git checkout -- <files>` 整文件覆盖**(会误伤其他未提交改动)。
- 发现 TASKS 验收标准与现状矛盾、或依赖卡实际未完成 → 冻结该 lane,更新 §3 矩阵,报告。
- 门禁出现与本 lane 无关的既有失败 → 不修复、不弱化,记录并继续;与本 lane 相关的失败必须修复至绿。
- 任何 §0 约束将被违反 → 停止,等用户指示。

回滚单元仅限合同声明的单卡 commit 或冻结逻辑组 commit;逻辑组不得拆分。所有回滚均在 `audit-remediation-phase3` 上用 `git revert <commit>`。回滚前必须列出该分支上已提交的反向依赖;如存在依赖者则停止并报告,获得用户指示后才能按反向拓扑序回滚依赖闭包。回滚后重跑受影响的卡级、lane 级及 wave 门禁,并确认未恢复 TASKS 明禁的不安全状态。禁止 force-push。

---

## §7 新会话引导指令(直接可执行)

新会话接管 loop 时,把以下指令作为首条消息粘贴:

```
阅读 docs/plan/2026-08-12-audit-remediation-sdd-loop.md 并接管 loop:
1. 检查并接管本地 `audit-remediation-phase3` 分支;以该分支中 §3 矩阵与 §8 日志为当前状态,不重新全盘盘点。
2. 查 §3"在途 lane 登记表",有在途 lane 则继续;否则按 §2 选下一条依赖就绪的 lane,按 §4 写合同后直接执行。
3. 每条 lane 收口后只做本地提交、更新台账,然后自动进入下一条;不逐 lane 请求批准。
4. 不合并 main,不 push/tag/release,不触发远端 workflows;触发 §0/§6 停止条件才暂停报告。
```

---

## §8 Loop 日志(每次迭代追加一行)

| 日期 | Lane | 动作 | 结果/证据 |
|---|---|---|---|
| 2026-08-12 | — | 基线盘点 + 本文档创建 | HEAD `fc737092`;✅24/◐1/🟡25/🔴20/⛔1/⏸1(共 72,经两轮审查核定) |
| 2026-08-12 | — | 五轴自审修复(4 必修 + 1 Nit) | C1 计数基线、I2 选择规则域优先、I3 AR11/AR12 拆行、I4 LOOP-5c 就绪传播、N5 AR05-T01 措辞 |
| 2026-08-12 | — | 用户审查修复(3 HIGH + 2 MEDIUM) | HIGH-1 状态转 DRAFT(待 checkpoint)、HIGH-2 lane 认领登记+分支规则、HIGH-3 禁止整文件 checkout、MEDIUM-1 T03A~E 展开五卡、MEDIUM-2 T07 降级 ◐;计数 → ✅24/◐1/🟡25/🔴20/⛔1/⏸1(共 72) |
| 2026-08-13 | L-AR03 | 合同经 Phase 3 一次批准覆盖,LOOP-3 本地认领登记 | 合同 `22565edc71c01579`;认领基线 main@`b68f123e`;计数 → ✅14/◐11/🟡23/🚧3/🔴20/⛔1/⏸0 |
| 2026-08-13 | — | Ralplan 收口 blocker 修正 | 主账原子认领+集成后 DONE、10 卡降为 ◐、W1~W6 门禁闭环、回滚单元+反向依赖闭包;计数 → ✅14/◐11/🟡25/🔴20/⛔1/⏸1(共 72,待处理 57) |
| 2026-08-13 | — | 授权模型改为 Phase 3 一次批准 | 唯一本地 `audit-remediation-phase3` 分支串行全部 lane;每 lane 本地提交+台账更新后自动继续;不合并 main,不远端同步;AR10-T01 转 🟡;计数 → ✅14/◐11/🟡26/🔴20/⛔1/⏸0 |

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
