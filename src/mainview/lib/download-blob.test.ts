import { afterEach, describe, expect, it, vi } from 'vitest'
import { downloadBlob } from './download-blob'

describe('downloadBlob', () => {
  afterEach(() => {
    vi.useRealTimers()
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
  })

  it('appends a temporary download anchor, clicks it, then removes it', () => {
    vi.useFakeTimers()

    const createObjectURL = vi.fn(() => 'blob:mock-url')
    const revokeObjectURL = vi.fn()
    vi.stubGlobal('URL', { createObjectURL, revokeObjectURL })

    const click = vi.fn()
    const remove = vi.fn()
    const appendChild = vi.fn((node: Node) => node)
    const anchor = {
      href: '',
      download: '',
      style: { display: '' },
      click,
      remove,
    }

    const createElement = vi.fn(() => anchor)
    vi.stubGlobal('document', {
      createElement,
      body: { appendChild },
    })

    const blob = new Blob(['hello'], { type: 'text/plain' })
    downloadBlob('note.txt', blob, 1_000)

    expect(createObjectURL).toHaveBeenCalledWith(blob)
    expect(createElement).toHaveBeenCalledWith('a')
    expect(anchor.download).toBe('note.txt')
    expect(anchor.href).toBe('blob:mock-url')
    expect(appendChild).toHaveBeenCalledWith(anchor)
    expect(click).toHaveBeenCalledOnce()
    expect(remove).toHaveBeenCalledOnce()

    expect(revokeObjectURL).not.toHaveBeenCalled()
    vi.advanceTimersByTime(1_000)
    expect(revokeObjectURL).toHaveBeenCalledWith('blob:mock-url')
  })
})
