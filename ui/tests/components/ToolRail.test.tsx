import { screen } from '@testing-library/react'
import { beforeEach, describe, expect, it } from 'vitest'

import { ToolRail } from '@/components/canvas/ToolRail'
import { useEditorUiStore } from '@/lib/stores/editorUiStore'

import { renderWithQuery } from '../helpers'

describe('ToolRail', () => {
  beforeEach(() => {
    useEditorUiStore.setState({ mode: 'select' })
  })

  it('does_not_expose_color_brush_but_keeps_repair_and_eraser', () => {
    renderWithQuery(<ToolRail />)

    expect(screen.queryByTestId('tool-brush')).not.toBeInTheDocument()
    expect(screen.getByTestId('tool-eraser')).toBeInTheDocument()
    expect(screen.getByTestId('tool-repairBrush')).toBeInTheDocument()
  })
})
