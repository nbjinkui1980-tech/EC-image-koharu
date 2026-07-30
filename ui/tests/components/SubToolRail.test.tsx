import { screen } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'

import { SubToolRail } from '@/components/canvas/SubToolRail'
import { useEditorUiStore } from '@/lib/stores/editorUiStore'
import { usePreferencesStore } from '@/lib/stores/preferencesStore'

import { renderWithQuery } from '../helpers'

describe('SubToolRail', () => {
  beforeEach(() => {
    useEditorUiStore.setState({ mode: 'select' })
    usePreferencesStore.setState({
      brushConfig: {
        size: 36,
      },
    })
  })

  it('renders nothing when select tool is active', () => {
    const { container } = renderWithQuery(<SubToolRail />)
    expect(container.firstChild).toBeNull()
  })

  it('renders when eraser tool is active', () => {
    useEditorUiStore.setState({ mode: 'eraser' })
    renderWithQuery(<SubToolRail />)
    expect(screen.getByTestId('sub-tool-rail')).toBeInTheDocument()
    expect(screen.getByText('toolbar.brushSize')).toBeInTheDocument()
  })

  it('renders when repairBrush tool is active', () => {
    useEditorUiStore.setState({ mode: 'repairBrush' })
    renderWithQuery(<SubToolRail />)
    expect(screen.getByTestId('sub-tool-rail')).toBeInTheDocument()
    expect(screen.getByText('toolbar.brushSize')).toBeInTheDocument()
  })

  it('displays the correct brush size', () => {
    useEditorUiStore.setState({ mode: 'eraser' })
    usePreferencesStore.setState({ brushConfig: { size: 64 } })
    renderWithQuery(<SubToolRail />)
    expect(screen.getByDisplayValue('64')).toBeInTheDocument()
  })
})
