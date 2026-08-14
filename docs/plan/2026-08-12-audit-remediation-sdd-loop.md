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
| **P2** | **L-AR05-ARCHIVE** 归档预算 | AR05-T02 → T03 → T04 | history/archive 预算线 | ✅ 收口(2026-08-15) |
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
| AR03-T01 Provider URL/authority 规范化 | ✅ | commit `4a71facf` | L-AR03 收口证据见合同 |
| AR03-T02 Config authority 冲突 | ✅ | commit `664d4071` | ←T01;review-fix `a1fad0e3` |
| AR03-T03 Redirect/错误脱敏 | ✅ | commit `d6bd1034` | ←T01;redirect 降级回归锁(实测默认安全) |
| AR04-T02 Apply/Undo/Redo durable commit | ✅ | commit `141f19ed` | L-AR04 收口证据见合同 |
| AR04-T03 损坏尾回滚 fail-stop | ✅ | commit `eb8b9ccc` | ←T02 |

### W4

| 卡 | 状态 | 证据 | 备注 |
|---|---|---|---|
| AR01-T04/T04B/T04C/T05 | ✅ | 台账 | |
| AR01-T06 Docker auth smoke | ⛔ | — | 合同 §6 出范围 |
| AR05-T01 Route body limits | ✅ | commit `85bc85c1` | L-AR05-LIMIT 收口证据见合同 |
| AR05-T02 Archive 读取预算 | ✅ | commit `23a9f98a` | L-AR05-ARCHIVE 收口证据见合同 |
| AR05-T03 Import 原子发布 | ✅ | commit `6af405c7` | ←T02;纯锁定卡(范围缩减预案) |
| AR05-T04 History frame 预算 | ✅ | commit `2b1a394c` | ←AR04-T03 |
| AR05-T05A Tauri picker File | ✅ | commit `d13f9b6e` | L-AR05-PICKER 收口证据见合同 |
| AR05-T05B 删除 from-paths API | ✅ | commit `c1fd37d2` | ←T05A;AMEND-01 落地 |
| AR05-T06 批量预算/decode admission | ✅ | commit `2d74327a` | ←T01 |
| AR06-T01 有界 JobRegistry | ✅ | commit `2024de62` | L-AR06 收口证据见合同 |
| AR06-T02 统一 registry | ✅ | commit `533d116d` | ←T01;纯锁定卡 |
| AR06-T03 Pipeline 单槽 | ✅ | commit `eda1b3f3` | ←T01,T02;429+Retry-After |
| AR06-T04 AI 双槽 | ✅ | commit `cfc88385` | ←T01,T02;panic 清理 |
| AR06-T05 Bulk import 单槽 | 🟡 | — | ←AR05-T03(✅`6af405c7`),T01(✅`85bc85c1`) |
| AR13-T02 破坏性 Project ID 精确匹配 | ✅ | commit `5ee962f2` | L-AR13B 收口证据见合同 |
| AR13-T03 Mask 复用 generated API | ✅ | commit `59e53e9f` | barrel 修复 `3c991482` |
| AR13-T04 Export 保留 filename | ✅ | commit `1e4ec083` | orval per-op mutator |

### W5

| 卡 | 状态 | 证据 | 备注 |
|---|---|---|---|
| AR07-T01 Axum+Tauri CSP | ✅ | commit `8634c8ca` | L-AR07 收口证据见合同 |
| AR07-T02 Webview navigation 同源 | ✅ | commit `75b51085` | on_navigation 闸 |
| AR07-T03 删除全盘 FS scope | ✅ | commit `8db96c6e` | dialog 临时授权;build 0 |
| AR08-T01 ZIP entry 路径验证 | ✅ | commit `615f8eee` | L-AR08 收口证据见合同 |
| AR08-T02 ZIP 全量预验证/预算 | ✅ | commit `4a2249e7` | ←T01;解锁 AR07-T03 |
| AR09-T01 SHA-256 下载/缓存不变量 | ✅ | commit `25d28f97` | L-AR09 收口证据见合同 |
| AR09-T02 llama/ZLUDA 描述 | ✅ | commit `6e5f622d` | ←T01;7 artifact 钉值 |
| AR09-T03 CUDA PyPI digest | ✅ | commit `34191ca3` | ←T01;fail closed;10 钉值 |
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
| 2026-08-13 | L-AR03 | T01 ✅(provider URL authority 规范化) | commit `4a71facf`;5 断言 RED(exit 101)→GREEN(exit 0);suite 36P/0F,clippy/fmt 净;计数 → ✅15/◐11/🟡23/🚧2/🔴20/⛔1/⏸0 |
| 2026-08-13 | L-AR03 | T02 ✅(config authority 冲突 409) | commit `664d4071`;4 断言 RED→GREEN;app 444P/rpc 22P,clippy/fmt 净;Orval 重生成(apiError schema);计数 → ✅16/◐11/🟡23/🚧1/🔴20/⛔1/⏸0 |
| 2026-08-13 | L-AR03 | T03 ✅(provider 错误脱敏;redirect 降级回归锁) | commit `d6bd1034`;RED-0 实测 reqwest 默认全满足(host/port 剥离 Authorization+Cookie,同 authority 保留)→合同范围缩减预案激活,文件域收缩为 providers/mod.rs 单文件;RED 1F/3P→GREEN 4/4;llm suite 40P/0F,clippy/fmt 净;计数 → ✅17/◐11/🟡23/🚧0/🔴20/⛔1/⏸0 |
| 2026-08-13 | L-AR03 | lane 收口 ✅(4 commit) | T01 `4a71facf` / T02 `664d4071` / T03 `d6bd1034` / review-fix `a1fad0e3`;门禁全绿(llm 40P/app 444P 二轮/rpc 33P,workspace clippy/fmt/check,check:generated);typography flake 复跑确认(既有,无关);独立 review 因 provider 模型故障(4 次启动失败)降级为对抗性自审——1 minor(409 message 字节截断 panic 路径)已修并验证;docs 证据单独提交;计数不变 → ✅17/◐11/🟡23/🚧0/🔴20/⛔1/⏸0 |
| 2026-08-14 | — | LOOP-5e 台账补漏(L-AR03) | commit `b0437c98`;§3 矩阵 AR03-T01/T02/T03 🚧→✅、在途登记表清除 L-AR03 行;无依赖传播(无卡等 L-AR03);计数不变 |
| 2026-08-14 | L-AR04 | 合同经 Phase 3 一次批准覆盖,LOOP-3 本地认领登记 | 合同 `e93680f4d38aa499`;认领基线 main@`b68f123e`,分支 tip `b0437c98`;前置 AR04-T01 ✅`fb7d5546`;子代理基础设施仍故障(子代理模型 ID 均带错误 `siliconflow/` 前缀),侦查经 codegraph 单执行器完成;计数 → ✅17/◐11/🟡22/🚧2/🔴19/⛔1/⏸0 |
| 2026-08-14 | L-AR04 | T02 ✅(Apply/Undo/Redo durable commit) | commit `141f19ed`;RED 3F/6P(均死于 scene must not change)→GREEN history 9P/0F、session 14P/0F;app suite/clippy/fmt 净;session.rs 未动(发布语义全在 History 内);计数 → ✅18/◐11/🟡22/🚧1/🔴19/⛔1/⏸0 |
| 2026-08-14 | L-AR04 | T03 ✅(损坏尾回滚 fail-stop) | commit `eb8b9ccc`;RED 3F/15P(坏尾未截断/未 fail-stop)→GREEN session 18P/0F、history 9P/0F;app suite/clippy/fmt 净;计数 → ✅19/◐11/🟡22/🚧0/🔴19/⛔1/⏸0 |
| 2026-08-14 | L-AR04 | lane 收口 ✅(2 commit) | T02 `141f19ed` / T03 `eb8b9ccc`;门禁全绿(app/rpc/llm suites exit 0,workspace clippy/fmt/check,check:generated 零漂移);CHECK/GEN 曾被 tauri dev 会话租约阻塞,用户关闭后完成(环境事件);独立 review 因 oracle 模型映射仍故障(第 8 次失败)降级对抗性自审——零 blocker/major/minor,1 informational(真实 mid-write 失败留部分尾由 T03 回滚接管,by design);依赖传播:AR05-T04 🔴→🟡(AR05-T03 仍等 AR05-T02);计数 → ✅19/◐11/🟡23/🚧0/🔴18/⛔1/⏸0 |
| 2026-08-14 | L-AR05-LIMIT | 合同经 Phase 3 一次批准覆盖,LOOP-3 本地认领登记 | 合同 `6a8eb8e7f2172480`;认领基线 main@`b68f123e`,分支 tip `b6b60c28`;前置 AR01-T01 ✅、AMEND-02 已批准;plan agent 探针第 9 次失败(子代理映射仍故障),继续 codegraph+单执行器;计数 → ✅19/◐11/🟡22/🚧2/🔴17/⛔1/⏸0 |
| 2026-08-14 | L-AR05-LIMIT | T01 ✅(Route body limits 分层) | commit `85bc85c1`;RED 3F/1P(400/400/500 到达 handler)→GREEN 4/4;关键取证:逐请求限值只能经 `DefaultBodyLimit::apply`(extension 键为 crate 私有 Kind);rpc suite/clippy/fmt 净;计数 → ✅20/◐11/🟡22/🚧1/🔴17/⛔1/⏸0 |
| 2026-08-14 | L-AR05-LIMIT | T06 ✅(批量预算/decode admission) | commit `2d74327a`;RED-0 脚手架(常量+覆盖缝+gauge)→RED 4F/2P→GREEN 6/6(gauge ≤2);rpc suite/clippy/fmt 净;计数 → ✅21/◐11/🟡22/🚧0/🔴17/⛔1/⏸0 |
| 2026-08-14 | L-AR05-LIMIT | lane 收口 ✅(2 commit) | T01 `85bc85c1` / T06 `2d74327a`;门禁全绿(rpc/app/llm suites exit 0,workspace clippy/fmt/check,check:generated 零漂移);独立 review 第 10 次启动失败降级对抗性自审——零 blocker/major,1 minor(multipart 单字段在编码总量拒绝前完整缓冲,TASKS"mutation 前累计"语义已满足,流式中段拒绝超范围);无依赖传播(无卡等 T06);计数不变 → ✅21/◐11/🟡22/🚧0/🔴17/⛔1/⏸0 |
| 2026-08-14 | L-AR05-PICKER | 合同经 Phase 3 一次批准覆盖,LOOP-3 本地认领登记 | 合同 `cdd7492a5a3f8ea8`;认领基线 main@`b68f123e`,分支 tip `363c9c37`;前置 AMEND-01 已批准;唯一 UI 卡 lane(T05A),子代理仍故障单执行器继续;计数 → ✅21/◐11/🟡21/🚧2/🔴16/⛔1/⏸0 |
| 2026-08-14 | L-AR05-PICKER | T05A ✅(picker 统一 File)+T05B ✅(删 from-paths)+lane 收口 | T05A `d13f9b6e`(RED 8F/5P→GREEN 13/13,UI 235 净)/ T05B `c1fd37d2`(rg 零命中,rpc 32P/0F,生成物纯删 79 行,快照更新);门禁全绿(3-crate、workspace clippy/fmt/check、check:generated 零漂移、UI 235P、lint:ui 0);oracle 第 11 次失败→自审零发现;无依赖传播(AR07-T03 仍等 AR08-T02);计数 → ✅23/◐11/🟡21/🚧0/🔴16/⛔1/⏸0 |
| 2026-08-14 | L-AR06 | 合同经 Phase 3 一次批准覆盖,LOOP-3 本地认领登记 | 合同 `bebe8dc7e311128d`;认领基线 main@`b68f123e`,分支 tip `d46facce`;前置 AR13-T01 ✅`88781c76`;T05 不在本 lane(维持 🔴 等 L-AR05-ARCHIVE);计数 → ✅23/◐11/🟡20/🚧4/🔴13/⛔1/⏸0 |
| 2026-08-14 | L-AR06 | T01 ✅(有界 JobRegistry) | commit `2024de62`;RED 3F(len 257/301/257)→GREEN 3/3 + mcp 锁 2/2;app suite 454P/0F,clippy/fmt 净;域外最小牵连 bootstrap/mcp 签名各一行(类型换型强制,证据注明);计数 → ✅24/◐11/🟡20/🚧3/🔴13/⛔1/⏸0 |
| 2026-08-14 | L-AR06 | T02 ✅+T03 ✅+T04 ✅+lane 收口 | T02 `533d116d`(纯锁定卡,三入口一致性)/T03 `eda1b3f3`(429+Retry-After,RED 200→GREEN)/T04 `cfc88385`(AI 双槽+catch_unwind panic 清理);门禁全绿(app 454P/rpc 38P/llm 40P、workspace clippy/fmt/check、check:generated 零漂移、UI 235P);oracle 第 12 次失败(入队成功但运行时仍模型 404)→对抗性自审零 blocker/major,1 informational(cancelled 字符串匹配为既有模式);无依赖传播(AR06-T05 仍等 AR05-T03);计数 → ✅27/◐11/🟡20/🚧0/🔴13/⛔1/⏸0 |
| 2026-08-14 | L-AR13B | 合同经 Phase 3 一次批准覆盖,LOOP-3 本地认领登记 | 合同 `6266066571e96275`;认领基线 main@`b68f123e`,分支 tip `64614e01`;前置 AR13-T01/AR01-T03 ✅;T04 停止条件裁决:orval per-operation mutator 可行(context7 取证),不回 SPEC;计数 → ✅27/◐11/🟡17/🚧3/🔴13/⛔1/⏸0 |
| 2026-08-14 | L-AR13B | T02 ✅+T03 ✅+T04 ✅+lane 收口(4 commit) | T02 `5ee962f2`(精确匹配,RED 1F→GREEN 4/4)/T03 `59e53e9f`(putMask 复用)+barrel 修复 `3c991482`(build 捕获的 T03 漏出)/T04 `1e4ec083`(orval mutator 保 headers);门禁全绿(3-crate、workspace clippy/fmt/check、check:generated、UI 236P、lint 0、format 净、**ui build exit 0**);oracle 第 13 次失败→自审零 blocker/major;教训落档:UI 卡门禁必含 build(mock 不验证真实导出);无依赖传播;计数 → ✅30/◐11/🟡17/🚧0/🔴13/⛔1/⏸0 |
| 2026-08-14 | L-AR08 | 合同经 Phase 3 一次批准覆盖,LOOP-3 本地认领登记 | 合同 `08a66a1821d353db`;认领基线 main@`b68f123e`,分支 tip `5ffd1bfb`;T02 停止条件裁决:fflate 0.8.3 流式 Unzip(onfile 预验+ondata 累计+terminate)分配前有界,不回 PLAN;预算值自定记合同决策点;计数 → ✅30/◐11/🟡16/🚧2/🔴12/⛔1/⏸0 |
| 2026-08-14 | L-AR08 | T01 ✅+T02 ✅+lane 收口 | T01 `615f8eee`(sanitizeZipEntryName 纯验证边界,RED 4F→GREEN 11/11)/T02 `4a2249e7`(流式 Unzip 两阶段预算,RED 3F→GREEN 15/15);门禁:UI 245P、lint/format 净、ui build 0、workspace check 0(纯 UI lane);oracle 第 14 次失败→自审零发现;**依赖传播:AR07-T03 🔴→🟡**(T05A+AR08-T02 均 ✅);计数 → ✅32/◐11/🟡17/🚧0/🔴11/⛔1/⏸0 |
| 2026-08-14 | L-AR07 | 合同经 Phase 3 一次批准覆盖,LOOP-3 本地认领登记 | 合同 `0de2241f3324c3ae`;认领基线 main@`b68f123e`,分支 tip `83e6371d`;前置 AR14-T04 ✅、T05A+AR08-T02 ✅;dialog 临时授权机制经 tauri 核心源码(auto-allow)取证;CSP 基线取 SPEC AR-07 冻结五条;计数 → ✅32/◐11/🟡16/🚧3/🔴10/⛔1/⏸0 |
| 2026-08-14 | L-AR07 | T01 ✅+T02 ✅+T03 ✅+lane 收口 | T01 `8634c8ca`(CSP 冻结五条+UI 让步项,rpc csp 2/2+policy)/T02 `75b51085`(on_navigation 同源闸,3/3)/T03 `8db96c6e`(删 fs scope **,policy 2/2,桌面 release build 0);门禁全绿;oracle 第 15 次未试(基础设施故障持续,沿用自审偏差);依赖传播:无新(AR07-T03 是本 lane 末卡);计数 → ✅35/◐11/🟡16/🚧0/🔴10/⛔1/⏸0 |
| 2026-08-14 | L-AR09 | 合同经 Phase 3 一次批准覆盖,LOOP-3 本地认领登记 | 合同 `a0933b09afcb0d05`;认领基线 main@`b68f123e`,分支 tip `515a2c0e`;无外部依赖;T02 digest 钉值策略:本机下载全部 artifact 算 sha256(决策点);oracle 第 15 次失败确认(AR07 收口时);计数 → ✅35/◐11/🟡15/🚧3/🔴8/⛔1/⏸0 |
| 2026-08-14 | L-AR09 | T01 ✅+T02 ✅+T03 ✅+lane 收口 | T01 `25d28f97`(sha2+验证通道,RED 2F→GREEN 5/5)/T02 `6e5f622d`(NativeArtifact+7 钉值+先验后装)/T03 `34191ca3`(PyPI digest fail-closed+10 钉值,孤儿 cached_download 移除);门禁全绿(runtime 33P 含真网络、3-crate、workspace clippy/fmt/check、check:generated 零漂移);oracle 第 16 次失败→自审零 blocker/major,2 informational;环境事件:G 卷掉线致 suite 卡死,重挂恢复;无依赖传播;计数 → ✅38/◐11/🟡15/🚧0/🔴8/⛔1/⏸0 |
| 2026-08-13 | — | Ralplan 收口 blocker 修正 | 主账原子认领+集成后 DONE、10 卡降为 ◐、W1~W6 门禁闭环、回滚单元+反向依赖闭包;计数 → ✅14/◐11/🟡25/🔴20/⛔1/⏸1(共 72,待处理 57) |
| 2026-08-13 | — | 授权模型改为 Phase 3 一次批准 | 唯一本地 `audit-remediation-phase3` 分支串行全部 lane;每 lane 本地提交+台账更新后自动继续;不合并 main,不远端同步;AR10-T01 转 🟡;计数 → ✅14/◐11/🟡26/🔴20/⛔1/⏸0 |
| 2026-08-15 | — | 接管前遗留工作区分治(非 loop 卡)+环境清理 | 4 commits:`7371b8f2`(dev.ts 进程树终止+dev.test.ts;修复其 bind flake——内核 socket 拆除瞬态窗口,lsof 无监听者/+50ms 可绑,改 waitForBind 轮询,6/6 稳定)/`94705d20`(UI 死代码:Kbd 内联/isDesktop→isTauri/RetryableSseError/useMemo 简化)/`bf7c9dcd`(hanonly 孤儿门禁+Rust 测试占位删除)/`db58bce3`(/.omo/ ignore);门禁:UI 245P(1 次 flake 三跑不复现,既有 typography 模式)、lint/format 净、dev.test 2/2、bus:: 5/5、clippy 双 crate -D warnings、rustfmt 净、ledger py_compile;终止 agent 遗留 tmux `koharu-dev`(持 cargo 租约 1h+,环境事件,同 AR09 先例);计数不变 → ✅38/◐11/🟡15/🚧0/🔴8/⛔1/⏸0 |
| 2026-08-15 | L-AR05-ARCHIVE | 合同经 Phase 3 一次批准覆盖,LOOP-3 本地认领登记 | 合同 `e554301c1af58e1b`;认领基线 main@`b68f123e`,分支 tip `24bef959`;前置 AR04-T02/T03 ✅、AR02-T04 ◐(实现已集成,证据待补录);T03 🔴→🚧、T02/T04 🟡→🚧;预算常量值留合同决策点(T02)/TASKS 定 16 MiB(T04);计数 → ✅38/◐11/🟡13/🚧3/🔴7/⛔1/⏸0 |
| 2026-08-15 | L-AR05-ARCHIVE | T02 ✅(Archive 读取预算) | commit `23a9f98a`;RED 5F/2P(entry 数/伪造声明 size/单项/总量/100:1 均无检查)→GREEN 7/7;app suite 461P/0F(hanonly_pre_greenc_red_t3 flake 单跑+复跑确认,既有,无关);clippy/fmt 净;预算定值 10_000 entries/256 MiB 单项/4 GiB 总量/100:1(1 MiB 下限);无依赖传播(T03 为本 lane 内下一卡);计数 → ✅39/◐11/🟡13/🚧2/🔴7/⛔1/⏸0 |
| 2026-08-15 | L-AR05-ARCHIVE | T03 ✅(Import 原子发布,纯锁定卡) | commit `6af405c7`;RED-0 实测现状原子性已满足(allocate→extract→open→rename+错误清理;既有 4 失败模式锁)→范围缩减预案激活(AR03-T03 先例),产品零改动;新增超预算锁(伪造声明 size→per-entry budget Err+无 staging/final 残留,补既有测试未断言 .staging 缺口);rpc lib 41P/0F(早前 15 失败=github 不可达瞬态环境,恢复后全绿);clippy/fmt 净;依赖传播:AR06-T05 仍等 lane 收口后统一评估;计数 → ✅40/◐11/🟡13/🚧1/🔴7/⛔1/⏸0 |
| 2026-08-15 | L-AR05-ARCHIVE | T04 ✅(History frame 分配前上限) | commit `2b1a394c`;RED 2F/0P(16 MiB+1/u32::MAX 头均被当截断尾静默容忍)→GREEN 2/2;history 11P/0F;app suite 463P/0F;clippy/fmt 净;16 MiB 上限两模式同 corruption,未超限截断尾保持 AR04-T03 语义;写侧对称上限记合同备查(超卡范围);计数 → ✅41/◐11/🟡13/🚧0/🔴7/⛔1/⏸0 |
| 2026-08-15 | L-AR05-ARCHIVE | lane 收口 ✅(4 commit) | T02 `23a9f98a` / T03 `6af405c7`(纯锁定) / T04 `2b1a394c` / review-fix `6f8fb8e8`;门禁全绿(app 465P/rpc 41P/llm 40P,workspace clippy/fmt/check,check:generated 零漂移);独立 review 经 oracle 完成(基础设施第 17 次恢复):零 blocker,1 major(比率检查信可伪造 compressed_size→归档长度钳制)+2 minor(消息断言脆弱裁决记录/staging 断言收紧)+3 informational,修复至零;依赖传播:AR06-T05 🔴→🟡(AR05-T03+T01 均 ✅);无 wave 收齐(AR06-T05 未竟);计数 → ✅41/◐11/🟡14/🚧0/🔴6/⛔1/⏸0 |

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
