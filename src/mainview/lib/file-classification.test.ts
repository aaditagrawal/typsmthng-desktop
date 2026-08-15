import { describe, expect, it } from 'vitest'
import { isKnownTextPath, isLatexPath, LATEX_TEXT_EXTENSIONS } from './file-classification'

describe('file-classification', () => {
  it('treats Overleaf-style LaTeX support files as text', () => {
    expect(LATEX_TEXT_EXTENSIONS).toEqual(expect.arrayContaining(['.sty', '.cls', '.bst']))
    expect(isKnownTextPath('ieee.cls')).toBe(true)
    expect(isKnownTextPath('macros.sty')).toBe(true)
    expect(isKnownTextPath('plain.bst')).toBe(true)
    expect(isLatexPath('main.tex')).toBe(true)
    expect(isLatexPath('extra.ltx')).toBe(true)
    expect(isKnownTextPath('extra.ltx')).toBe(true)
    expect(isLatexPath('main.TYP')).toBe(false)
  })
})
