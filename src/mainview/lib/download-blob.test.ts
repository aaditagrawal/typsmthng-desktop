import { afterEach, describe, expect, it, vi } from 'vitest'
import { downloadBlob } from './download-blob'

describe('downloadBlob', () => {
  afterEach(() => {
    vi.useRealTimers()
    vi.restoreAllMocks()
  })

  it('appends a temporary download anchor, clicks it, then removes it', () => {
    vi.useFakeTimers()

    const createObjectURL = vi.fn(() => 'blob:mock-url')
    const revokeObjectURL = vi.fn()
    vi.stubGlobal('URL', { createObjectURL, revokeObjectURL })

    const click = vi.fn()
    const remove = vi.fn()
    const appendChild = vi.spyOn(document.body, 'appendChild').mockImplementation((node) => node)
    const createElement = vi.spyOn(document, 'createElement').mockReturnValue({
      href: '',
      download: '',
      style: { display: '' },
      click,
      remove,
    } as unknown as HTMLAnchorElement)

    const blob = new Blob(['hello'], { type: 'text/plain' })
    downloadBlob('note.txt', blob, 1_000)

    expect(createObjectURL).toHaveBeenCalledWith(blob)
    expect(createElement).toHaveBeenCalledWith('a')
    expect(appendChild).toHaveBeenCalled()
    expect(click).toHaveBeenCalledOnce()
    expect(remove).toHaveBeenCalledOnce()

    expect(revokeObjectURL).not.toHaveBeenCalled()
    vi.advanceTimersByTime(1_000)
    expect(revokeObjectURL).toHaveBeenCalledWith('blob:mock-url')
  })
})
