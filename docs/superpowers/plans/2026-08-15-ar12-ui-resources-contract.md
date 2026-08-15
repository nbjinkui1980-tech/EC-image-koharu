# Lane 执行合同:L-AR12 — UI 资源生命周期(6 张相互独立卡)

- 状态:**Phase 3 一次批准已覆盖(2026-08-13 授予)**;LOOP-3 本地认领已登记
- 认领基线:Phase 3 起点 main `b68f123e`;执行分支 `audit-remediation-phase3`(认领时 tip `7cad9a4f`)
- 提交/回滚单元:T01~T06 各一个单卡 commit(可独立 revert);lane 收口 docs(evidence) 单独一个 commit
- 前置依赖:无(卡间相互独立);T02/T06 共享 `font-select.tsx`,串行执行天然隔离
- 执行环境偏差(继承):oracle 已恢复;lane 收口独立 review 沿用 oracle,失败则对抗性自审并落档
- 串行点声明:UI lane——门禁含 `test:ui`/`lint:ui`/`format:check`/`ui build`;不占用 Cargo/lockfile/Orval

## 范围文件域(域外改动禁止)

| 卡 | 允许文件 |
|---|---|
| AR12-T01 | `ui/hooks/useBlobData.ts`、`ui/components/Image.tsx`、对应两个 tests(≤4) |
| AR12-T02 | `ui/components/ui/font-select.tsx`、现有 FontSelect test(≤2) |
| AR12-T03 | `ui/lib/stores/jobsStore.ts`、`ui/lib/stores/downloadsStore.ts`、`ui/tests/lib/events.dispatch.test.ts`(≤3) |
| AR12-T04 | `ui/components/Updater.tsx`、现有 Updater test(≤2) |
| AR12-T05 | `ui/hooks/useKeyboardShortcuts.ts`、现有对应 test(≤2) |
| AR12-T06 | `ui/components/ui/font-select.tsx`、`ui/components/Navigator.tsx`、两个现有 tests(≤4) |

新依赖:无。

## 卡:AR12-T01 — Query 缓存 bytes,组件拥有 URL

- **验收标准(TASKS 原文)**:文件:`ui/hooks/useBlobData.ts`、`ui/components/Image.tsx`、两个对应 tests。RED:query cache 持有 object URL;replacement/error/unmount 未 revoke。GREEN:cache 保存 Blob/bytes;组件 create/revoke。验证:URL spy tests。
- **现状(RED-0 源码实证)**:`blobImageQueryOptions.queryFn`(useBlobData.ts:33-43)把 `URL.createObjectURL(blob)` 的结果缓存在 query cache,全站无 revoke;`Image.tsx` 两处 createObjectURL(47/141)同样无 revoke。
- **设计**:queryFn 缓存 Blob(预加载用临时 URL 后立即 revoke);`useBlobImage` 在组件实例内 create/revoke(useEffect cleanup);Image.tsx 的 URL 生命周期补 revoke。
- **RED 断言**(URL spy):
  1. `query_cache_holds_blob_not_object_url` — 缓存条目为 Blob 而非 string URL → 现状 FAIL
  2. `object_url_revoked_when_component_unmounts_or_hash_changes` — unmount/hash 变更 → revokeObjectURL 被调 → 现状 FAIL
- **目标文件**:上表 T01 行(≤4)
- **验收命令**:`bun run --cwd ui test -- tests/hooks/useBlobData tests/components/Image`
- **证据记录**:RED / GREEN / commit SHA(执行时填)

## 卡:AR12-T02 — FontFace owner

- **验收标准(TASKS 原文)**:文件:`ui/components/ui/font-select.tsx`、现有 FontSelect test。RED:stale load 添加 face;unmount 不 delete。GREEN:组件 ownership + cancellation cleanup。验证:FontSelect Vitest。
- **现状(RED-0 源码实证)**:font-select.tsx:50-75 加载 effect 已有 `cancelled` 守卫(stale 不 add——该 RED 面按 AR03-T03 先例转锁);cleanup 只置 flag,**unmount 不 `document.fonts.delete(face)`**。
- **设计**:记录组件添加的 face,unmount/依赖变更时 `document.fonts.delete(face)`;stale 锁回归。
- **RED 断言**(FontSelect Vitest,document.fonts spy):
  1. `added_font_face_is_deleted_on_unmount` → 现状 FAIL
  2. `stale_load_never_adds_face`(锁,预期 PASS)
- **目标文件**:上表 T02 行(≤2)
- **验收命令**:`bun run --cwd ui test -- tests/components/FontSelect`
- **证据记录**:RED / GREEN / commit SHA(执行时填)

## 卡:AR12-T03 — UI jobs/downloads retention

- **验收标准(TASKS 原文)**:文件:`ui/lib/stores/jobsStore.ts`、`downloadsStore.ts`、`ui/tests/lib/events.dispatch.test.ts`。RED:completed 无限增长或 Running 被 trim。GREEN:固定 bound,保留 Running;与后端 256 completed 对齐。验证:events dispatch Vitest。
- **现状(RED-0 源码实证)**:jobsStore/downloadsStore 无任何 trim/bound 逻辑(grep 零命中)——completed 无限增长。
- **设计**:completed 条目封顶 256(最旧先出),Running/进行中状态永不被 trim;事件 dispatch 路径落地。
- **RED 断言**(events.dispatch Vitest):
  1. `completed_jobs_are_bounded_at_256_while_running_survive` → 现状 FAIL
- **目标文件**:上表 T03 行(≤3)
- **验收命令**:`bun run --cwd ui test -- tests/lib/events.dispatch`
- **证据记录**:RED / GREEN / commit SHA(执行时填)

## 卡:AR12-T04 — Updater cleanup

- **验收标准(TASKS 原文)**:文件:`ui/components/Updater.tsx`、现有 Updater test。RED:replacement/unmount 不 close 或重复 close。GREEN:明确 owner,只 close 一次。验证:Updater Vitest;真实 updater 保持凭据门禁。
- **现状(RED-0 源码实证)**:Updater.tsx:57-61 已有 `[update]` effect cleanup 关闭旧实例——结构疑似已满足;RED-0 需实证 replacement 路径与重复 close 面,若已安全按 AR03-T03 先例转锁。
- **设计**:每个 Update 实例恰好 close 一次的测试锁;发现缺口再最小修复。
- **RED 断言**(Updater Vitest):
  1. `each_update_handle_is_closed_exactly_once_across_replacement_and_unmount` → 现状待证
- **目标文件**:上表 T04 行(≤2)
- **验收命令**:`bun run --cwd ui test -- tests/components/Updater`
- **证据记录**:RED / GREEN / commit SHA(执行时填)

## 卡:AR12-T05 — 文本输入原生 undo/redo

- **验收标准(TASKS 原文)**:文件:`ui/hooks/useKeyboardShortcuts.ts`、现有 test。RED:input/textarea/contenteditable 的 Ctrl/Cmd+Z/Y 触发 scene history。GREEN:editable target 直接保留浏览器行为。验证:keyboard Vitest、三平台键盘 smoke。
- **现状(RED-0 源码实证)**:useKeyboardShortcuts.ts:29-55 计算了 `inTextField` 但 undo/redo/Y 分支无视它(注释明示"including from within text fields")——输入框内 Cmd+Z 触发 scene undo 且 preventDefault 阻断原生文本撤销。
- **设计**:editable target 时 undo/redo/Ctrl+Y 分支直接 return(不 preventDefault),浏览器原生文本撤销生效;非 editable 保持 scene history。
- **RED 断言**(keyboard Vitest):
  1. `editable_target_keeps_native_text_undo_redo` — input 内 Cmd+Z → undoOp 未调、未 preventDefault → 现状 FAIL
  2. `non_editable_target_routes_to_scene_history`(锁,预期 PASS)
- **目标文件**:上表 T05 行(≤2)
- **验收命令**:`bun run --cwd ui test -- tests/hooks/useKeyboardShortcuts`
- **证据记录**:RED / GREEN / commit SHA(执行时填)

## 卡:AR12-T06 — 字体收藏与删除按钮 a11y

- **验收标准(TASKS 原文)**:文件:`font-select.tsx`、`Navigator.tsx`、两个现有 tests。RED:收藏 Enter/Space 冒泡选择字体;删除按钮无 accessible name/focus-visible。GREEN:独立键盘 target、accessible name、可见焦点。验证:FontSelect + Navigator Vitest。
- **现状(RED-0 源码勘察)**:font-select 收藏按钮与 Navigator 删除按钮的键盘/ARIA 面待 RED-0 逐条核实;预期冒泡与缺 aria-label 成立。
- **设计**:收藏按钮 `stopPropagation` + 独立 keydown 处理;删除按钮加 accessible name 与 focus-visible 样式。
- **RED 断言**(FontSelect + Navigator Vitest):
  1. `favorite_button_enter_does_not_select_font` → 现状 FAIL
  2. `delete_button_has_accessible_name_and_visible_focus` → 现状 FAIL
- **目标文件**:上表 T06 行(≤4)
- **验收命令**:`bun run --cwd ui test -- tests/components/FontSelect tests/components/Navigator`
- **证据记录**:RED / GREEN / commit SHA(执行时填)
