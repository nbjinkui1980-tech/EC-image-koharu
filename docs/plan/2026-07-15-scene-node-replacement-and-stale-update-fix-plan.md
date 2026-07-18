# Scene 节点替换原子性与旧 ID 提交防护 Implementation Plan

> **For Claude:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task.

**Goal:** Detector 没有产生有效新文本框时保留现有 Text 节点；流水线替换节点期间，渲染设置 UI 不再向后端提交旧节点 ID，也不产生未处理的 `NodeNotFound` 异常。

**Architecture:** 修改现有 `clear_text_nodes_ops()`，让四个 Detector 通过同一入口表达“只有存在替换结果才删除旧节点”。UI 继续以后端 Scene 为事实来源：流水线运行时禁用渲染设置；字体异步操作完成后重新读取最新 Scene 并按当前节点 ID 组装 Op；任何样式提交失败都刷新 Scene、停止渲染并通过现有错误状态提示。

**Tech Stack:** Rust、koharu-core Scene/Op、React 19、TypeScript、Zustand、TanStack Query、Vitest、Testing Library、MSW、Bun。

---

## 已确认根因与不变量

- 当前错误不是 Next.js/Turbopack 故障。后端页面仍存在，但请求中的 Text Node 已不存在，`UpdateNode` 因此在 `koharu-core` 返回 `NodeNotFound`。
- `pp-doclayout-v3` 得到零个有效 block 时仍调用 `clear_text_nodes_ops()`，历史记录显示旧 Text 节点被清除，且没有新 Text 节点补回。
- `clear_text_nodes_ops()` 还有 CTD、Comic Text Bubble、Anime Text 三个调用方；修复必须落在这个共享入口，不能只给 PP-DocLayout 加特例。
- 字体选择可能先等待 Google Font 下载；回调恢复时，React 闭包中的 `selectedNodes` / `textNodes` 可能已经过期。
- Detector 有非空结果时仍按现有行为原子替换旧 Text 节点；零结果只保留 Text 节点，Detector 产生的独立 Mask 更新不受影响。
- 样式 Op 失败后不得更新 `defaultFont`，不得启动立即渲染或 debounce 渲染。
- 不修改 HTTP/OpenAPI、Scene DTO、Op JSON、HanOnly/AllText 语义、模型和权重；不新增依赖、epoch API、trait、store 或通用并发层。
- 自动化测试只使用内存 Scene 和 MSW，不加载或下载模型。

## Task 1：让所有 Detector 的零替换结果保持现有 Text 节点

**Files:**

- Modify: `crates/koharu-app/src/pipeline/engines/support.rs:887-910`
- Modify: `crates/koharu-app/src/pipeline/engines/pp_doclayout.rs:22-40`
- Modify: `crates/koharu-app/src/pipeline/engines/ctd_full.rs:24-57`
- Modify: `crates/koharu-app/src/pipeline/engines/comic_text_bubble.rs:43-78`
- Modify: `crates/koharu-app/src/pipeline/engines/anime_text.rs:24-45`
- Test: `crates/koharu-app/src/pipeline/engines/support.rs`

### Step 1：写共享入口失败测试

在现有 `support.rs` tests 中加入一个最小测试，复用本文件已有的 `translation_scene()` 和 `translated_node()`：

```rust
#[test]
fn text_replacement_cleanup_preserves_existing_nodes_when_new_count_is_zero() {
    let id = NodeId::new();
    let (scene, page) = translation_scene(vec![translated_node(id, "旧文本")]);

    assert!(clear_text_nodes_ops(&scene, page, 0).is_empty());

    let ops = clear_text_nodes_ops(&scene, page, 1);
    assert_eq!(ops.len(), 1);
    assert!(matches!(ops[0], Op::RemoveNode { id: removed, .. } if removed == id));
}
```

### Step 2：运行测试并确认 FAIL

```bash
bun cargo test -p koharu-app text_replacement_cleanup_preserves_existing_nodes_when_new_count_is_zero
```

预期：FAIL，原因是当前 `clear_text_nodes_ops()` 只接收两个参数。

### Step 3：修改现有共享 helper

不给项目增加新 helper；直接扩展现有函数：

```rust
pub fn clear_text_nodes_ops(
    scene: &Scene,
    page: PageId,
    replacement_count: usize,
) -> Vec<Op> {
    if replacement_count == 0 {
        return Vec::new();
    }
    let Some(page_ref) = scene.page(page) else {
        return Vec::new();
    };
    page_ref
        .nodes
        .iter()
        .enumerate()
        .filter(|(_, (_, node))| matches!(&node.kind, NodeKind::Text(_)))
        .map(|(idx, (id, node))| Op::RemoveNode {
            page,
            id: *id,
            prev_node: node.clone(),
            prev_index: idx,
        })
        .collect()
}
```

四个调用方必须先形成最终有效 `blocks` / `pairs`，再传入 `.len()`：

```rust
let mut ops = clear_text_nodes_ops(ctx.scene, ctx.page, blocks.len());
```

CTD 必须先构造并排序 `pairs`，再调用 helper；`pairs.is_empty()` 时只保留现有 Text 节点，Segment Mask 的 upsert 继续执行。

### Step 4：运行定向测试和 Detector 回归

```bash
bun cargo test -p koharu-app text_replacement_cleanup_preserves_existing_nodes_when_new_count_is_zero
bun cargo test -p koharu-app pipeline::engines::pp_doclayout::tests
bun cargo check -p koharu-app --all-targets
```

预期：PASS；四个 Detector 调用点全部通过编译，PP-DocLayout 既有过滤、去重和方向测试保持通过。

### Step 5：提交 Task 1

```bash
git add crates/koharu-app/src/pipeline/engines/support.rs \
  crates/koharu-app/src/pipeline/engines/pp_doclayout.rs \
  crates/koharu-app/src/pipeline/engines/ctd_full.rs \
  crates/koharu-app/src/pipeline/engines/comic_text_bubble.rs \
  crates/koharu-app/src/pipeline/engines/anime_text.rs
git commit -m "fix(pipeline): preserve text on empty detection"
```

## Task 2：阻止渲染设置提交旧 Text Node ID

**Files:**

- Modify: `ui/lib/api/index.ts:35-74`
- Modify: `ui/components/panels/RenderControlsPanel.tsx:30-64,136-172,340-426,486-end`
- Test: `ui/tests/components/RenderControlsPanel.test.tsx`

### Step 1：写三个失败测试

在现有 `RenderControlsPanel.test.tsx` 中：

1. 把 `invalidateScene` 加入 `@/lib/io/scene` mock。
2. `beforeEach` 调用 `useJobsStore.getState().clear()`。
3. 添加以下固定测试名：

```text
disables render controls while a pipeline job is running
refreshes the scene and skips stale font target ids
handles style apply failures without queueing a render
```

测试行为：

```typescript
it('disables render controls while a pipeline job is running', async () => {
  useJobsStore.getState().started('job-1', 'pipeline')
  renderWithQuery(<RenderControlsPanel />)

  expect(await screen.findByTestId('render-font-select')).toBeDisabled()
})

it('refreshes the scene and skips stale font target ids', async () => {
  const { client } = renderWithQuery(<RenderControlsPanel />)
  useSelectionStore.getState().select('t1', false)
  vi.mocked(sceneActions.invalidateScene).mockImplementationOnce(async () => {
    client.setQueryData(getGetSceneJsonQueryKey(), sceneWithTextNodes([]))
  })

  await userEvent.click(await screen.findByTestId('render-font-select'))
  await userEvent.click(await screen.findByText('Custom'))

  await waitFor(() => expect(sceneActions.invalidateScene).toHaveBeenCalled())
  expect(sceneActions.applyOp).not.toHaveBeenCalled()
  expect(sceneActions.runAutoRenderNow).not.toHaveBeenCalled()
})

it('handles style apply failures without queueing a render', async () => {
  vi.mocked(sceneActions.applyOp).mockRejectedValueOnce(new Error('node not found'))
  renderWithQuery(<RenderControlsPanel />)
  useSelectionStore.getState().select('t1', false)

  fireEvent.change(await screen.findByTestId('render-font-size'), {
    target: { value: '42' },
  })

  await waitFor(() =>
    expect(useEditorUiStore.getState().error?.message).toContain('node not found'),
  )
  expect(sceneActions.invalidateScene).toHaveBeenCalled()
  expect(sceneActions.queueAutoRender).not.toHaveBeenCalled()
  expect(sceneActions.runAutoRenderNow).not.toHaveBeenCalled()
})
```

### Step 2：运行测试并确认 FAIL

```bash
bun run --filter ui test -- tests/components/RenderControlsPanel.test.tsx -t "disables render controls|skips stale font target ids|handles style apply failures"
```

预期：FAIL。当前面板不知道流水线状态；字体操作继续使用闭包中的旧节点；非字体样式的 `void applyStyleToNodes(...)` 会留下未处理 rejection。

### Step 3：实现最小 UI 修复

1. 从现有 API facade 导出已经生成的 `getSceneJson`；不创建新 API wrapper。
2. 复用 `useJobsStore`，把面板最外层交互容器改成无边框 `fieldset`，流水线运行时设置 `disabled`。
3. 字体操作在提交前调用 `getSceneJson()`，把结果写入现有 Scene query key，并通过 `textNodesOf()` 和当前 `selectionStore.nodeIds` 重新解析目标。
4. 选中作用域中只要有一个 ID 已不存在，就取消整个 batch；不得部分更新、不得设置全局字体、不得渲染。
5. 在共享 `applyStyleToNodes()` 中捕获 `applyOp()` 错误，刷新 Scene、显示现有错误提示并立即返回。

字体目标重新解析使用现有类型和 store：

```typescript
const applyFontToCurrentScope = async (postScriptName: string): Promise<void> => {
  const pageId = useSelectionStore.getState().pageId
  if (!pageId) return

  let snapshot
  try {
    snapshot = await getSceneJson()
  } catch (error) {
    useEditorUiStore.getState().showError(String(error))
    return
  }
  queryClient.setQueryData(getGetSceneJsonQueryKey(), snapshot)

  const currentPage = snapshot.scene.pages[pageId]
  if (!currentPage) return
  const currentNodes = textNodesOf(currentPage)
  const selectedIds = useSelectionStore.getState().nodeIds

  if (selectedIds.size > 0) {
    const targets = currentNodes.filter((node) => selectedIds.has(node.id))
    if (targets.length !== selectedIds.size) return
    await applyStyleToNodes(targets, { fontFamilies: [postScriptName] }, 'Font family update', true)
    return
  }

  const setGlobalDefault = () =>
    usePreferencesStore.getState().setDefaultFont(postScriptName)
  if (currentNodes.length === 0) {
    setGlobalDefault()
    return
  }
  await applyStyleToNodes(
    currentNodes,
    { fontFamilies: [postScriptName] },
    'Font family update',
    true,
    setGlobalDefault,
  )
}
```

共享提交入口只增加错误边界：

```typescript
try {
  await applyOp(op)
} catch (error) {
  await invalidateScene().catch(() => undefined)
  useEditorUiStore.getState().showError(String(error))
  return
}
```

流水线锁复用已有 store，不新增状态：

```typescript
const isProcessing = useJobsStore((state) =>
  Object.values(state.jobs).some((job) => job.status === 'running'),
)

<fieldset disabled={isProcessing} className='m-0 flex w-full min-w-0 flex-col gap-2 border-0 p-0'>
  {/* existing controls unchanged */}
</fieldset>
```

### Step 4：运行 UI 定向测试和完整组件回归

```bash
bun run --filter ui test -- tests/components/RenderControlsPanel.test.tsx -t "disables render controls|skips stale font target ids|handles style apply failures"
bun run --filter ui test -- tests/components/RenderControlsPanel.test.tsx
```

预期：PASS。既有字体下载顺序、全局/选中作用域、样式字段保持和立即渲染测试不得回归。

### Step 5：提交 Task 2

```bash
git add ui/lib/api/index.ts \
  ui/components/panels/RenderControlsPanel.tsx \
  ui/tests/components/RenderControlsPanel.test.tsx
git commit -m "fix(ui): reject stale text style updates"
```

## Task 3：完整验证与问题项目人工复现

**Files:**

- No production file changes.

### Step 1：运行 Rust 门禁

```bash
bun cargo fmt --all -- --check
bun cargo check --workspace --all-targets
bun cargo clippy --workspace --all-targets -- -D warnings
bun cargo test --workspace --tests
```

预期：全部 PASS；测试不加载或下载 Detector、OCR、Inpainter 模型。

### Step 2：运行 UI 和生成物门禁

```bash
bun run format:check
bun run lint:ui
bun run test:ui
bun run check:generated
bun run build
```

预期：全部 PASS；`check:generated` 不留下 diff。

### Step 3：检查差异

```bash
git diff --check
git status --short
```

预期：只有 Task 1、Task 2 的计划内文件；不得修改 OpenAPI、生成 schema、模型文件或其他计划文档。

### Step 4：使用当前问题项目人工验收

打开 `test.khrproj` 并验证：

1. 页面已有 Text 节点时运行 PP-DocLayout；若有效结果为 0，旧 Text 节点和已有样式保持不变。
2. Detector 有非空结果时，旧 Text 节点仍被新结果完整替换，不发生重复叠加。
3. 流水线运行期间，渲染设置不可编辑。
4. 选择未缓存字体并让 Scene 在下载期间发生刷新；操作只允许应用到刷新后仍存在的节点，否则安全取消。
5. 不再出现 Next.js `Runtime ApiError: node not found` overlay；被后端拒绝的样式操作不启动 Renderer。

## 停止条件

- 零有效检测结果不会删除已有 Text 节点。
- 四个 Detector 都经过同一共享保护入口。
- 字体异步操作不使用下载前捕获的节点数组。
- 流水线运行时不能发起新的渲染样式操作。
- 任何样式 Op 失败后 Scene 会刷新，且不会产生未处理 rejection 或后续渲染。
- 所有定向测试、完整门禁和当前问题项目人工验收通过后结束；不继续引入 epoch 协议或新的并发框架。
