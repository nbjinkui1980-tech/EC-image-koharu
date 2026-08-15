import { renderHook } from '@testing-library/react'
import { fireEvent } from '@testing-library/react'
import { beforeEach, describe, expect, it, vi } from 'vitest'

import { useKeyboardShortcuts } from '@/hooks/useKeyboardShortcuts'
import { getGetSceneJsonQueryKey } from '@/lib/api'
import type { Node, Page, SceneSnapshot } from '@/lib/api/schemas'
import { queryClient } from '@/lib/queryClient'
import { useEditorUiStore } from '@/lib/stores/editorUiStore'
import { usePreferencesStore } from '@/lib/stores/preferencesStore'
import { useSelectionStore } from '@/lib/stores/selectionStore'

const sceneOps = vi.hoisted(() => ({ undoOp: vi.fn(), redoOp: vi.fn() }))

vi.mock('@/lib/io/scene', async () => {
  const actual = await vi.importActual<typeof import('@/lib/io/scene')>('@/lib/io/scene')
  return { ...actual, undoOp: sceneOps.undoOp, redoOp: sceneOps.redoOp }
})

function textNode(id: string): Node {
  return {
    id,
    transform: { x: 0, y: 0, width: 10, height: 10, rotationDeg: 0 },
    visible: true,
    kind: { text: { raw: `t-${id}` } },
  } as unknown as Node
}

function seedScene(): SceneSnapshot {
  const page: Page = {
    id: 'p-1',
    name: 'P',
    width: 10,
    height: 10,
    nodes: { t1: textNode('t1'), t2: textNode('t2') },
  } as unknown as Page
  return {
    epoch: 1,
    scene: { pages: { 'p-1': page }, project: { name: 'P' } as never } as never,
  }
}

describe('useKeyboardShortcuts — Ctrl+A', () => {
  beforeEach(() => {
    useSelectionStore.getState().setPage(null)
    useEditorUiStore.setState({ mode: 'select' })
    usePreferencesStore.getState().resetPreferences()
    queryClient.clear()
  })

  it('Ctrl+A selects every text node on the active page', () => {
    queryClient.setQueryData(getGetSceneJsonQueryKey(), seedScene())
    useSelectionStore.getState().setPage('p-1')
    renderHook(() => useKeyboardShortcuts())

    fireEvent.keyDown(window, { key: 'a', ctrlKey: true })

    expect([...useSelectionStore.getState().nodeIds].sort()).toEqual(['t1', 't2'])
  })

  it('Ctrl+A is a no-op while typing inside a textarea', () => {
    queryClient.setQueryData(getGetSceneJsonQueryKey(), seedScene())
    useSelectionStore.getState().setPage('p-1')
    renderHook(() => useKeyboardShortcuts())

    const textarea = document.createElement('textarea')
    document.body.appendChild(textarea)
    textarea.focus()

    fireEvent.keyDown(textarea, { key: 'a', ctrlKey: true })

    expect(useSelectionStore.getState().nodeIds.size).toBe(0)

    document.body.removeChild(textarea)
  })

  it('removed_brush_shortcut_does_not_switch_tools', () => {
    renderHook(() => useKeyboardShortcuts())

    fireEvent.keyDown(window, { key: 'b' })

    expect(useEditorUiStore.getState().mode).toBe('select')
  })
})

describe('useKeyboardShortcuts — undo/redo in text fields', () => {
  beforeEach(() => {
    useSelectionStore.getState().setPage(null)
    useEditorUiStore.setState({ mode: 'select' })
    usePreferencesStore.getState().resetPreferences()
    queryClient.clear()
    sceneOps.undoOp.mockClear()
    sceneOps.redoOp.mockClear()
  })

  it('keeps native text undo/redo inside editable targets', () => {
    renderHook(() => useKeyboardShortcuts())
    const input = document.createElement('input')
    document.body.appendChild(input)
    input.focus()

    const event = new KeyboardEvent('keydown', {
      key: 'z',
      ctrlKey: true,
      bubbles: true,
      cancelable: true,
    })
    fireEvent(input, event)

    expect(sceneOps.undoOp).not.toHaveBeenCalled()
    expect(event.defaultPrevented).toBe(false)

    const redoEvent = new KeyboardEvent('keydown', {
      key: 'y',
      ctrlKey: true,
      bubbles: true,
      cancelable: true,
    })
    fireEvent(input, redoEvent)
    expect(sceneOps.redoOp).not.toHaveBeenCalled()
    expect(redoEvent.defaultPrevented).toBe(false)

    document.body.removeChild(input)
  })

  it('keeps native text undo inside contenteditable targets', () => {
    renderHook(() => useKeyboardShortcuts())
    const div = document.createElement('div')
    div.contentEditable = 'true'
    Object.defineProperty(div, 'isContentEditable', { value: true })
    document.body.appendChild(div)
    div.focus()

    const event = new KeyboardEvent('keydown', {
      key: 'z',
      ctrlKey: true,
      bubbles: true,
      cancelable: true,
    })
    fireEvent(div, event)

    expect(sceneOps.undoOp).not.toHaveBeenCalled()
    expect(event.defaultPrevented).toBe(false)

    document.body.removeChild(div)
  })

  it('routes undo to scene history outside editable targets', () => {
    renderHook(() => useKeyboardShortcuts())
    const event = new KeyboardEvent('keydown', {
      key: 'z',
      ctrlKey: true,
      bubbles: true,
      cancelable: true,
    })
    fireEvent(window, event)

    expect(sceneOps.undoOp).toHaveBeenCalledTimes(1)
    expect(event.defaultPrevented).toBe(true)
  })
})
