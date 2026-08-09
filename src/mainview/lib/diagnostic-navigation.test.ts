import { describe, expect, it } from 'vitest'
import {
  normalizeDiagnosticPath,
  waitForEditorPath,
} from '@/lib/diagnostic-navigation'

describe('normalizeDiagnosticPath', () => {
  it('strips leading slashes and normalizes separators', () => {
    expect(normalizeDiagnosticPath('/notes\\intro.typ')).toBe('notes/intro.typ')
    expect(normalizeDiagnosticPath('notes/intro.typ')).toBe('notes/intro.typ')
  })
})

describe('waitForEditorPath', () => {
  it('waits until the target path is current and loaded', async () => {
    let currentPath: string | null = 'main.typ'
    let loaded = false
    const view = { id: 'editor' } as never

    setTimeout(() => {
      currentPath = 'notes/intro.typ'
      loaded = true
    }, 30)

    const ready = await waitForEditorPath('notes/intro.typ', {
      timeoutMs: 500,
      isCurrentPath: () => currentPath,
      isFileLoaded: () => loaded,
      getEditorView: () => (loaded ? view : null),
    })

    expect(ready).toBe(view)
  })

  it('returns null when the file never becomes ready', async () => {
    await expect(
      waitForEditorPath('missing.typ', {
        timeoutMs: 40,
        isCurrentPath: () => 'main.typ',
        isFileLoaded: () => false,
        getEditorView: () => null,
      }),
    ).resolves.toBeNull()
  })
})
