import { afterEach, describe, expect, it, vi } from 'vitest'

const saveDownloadMock = vi.fn()

vi.mock('@/lib/desktop-rpc', () => ({
  desktopRpc: {
    request: {
      saveDownload: (...args: unknown[]) => saveDownloadMock(...args),
    },
  },
}))

import { downloadBlob } from './download-blob'

describe('downloadBlob', () => {
  afterEach(() => {
    vi.useRealTimers()
    vi.restoreAllMocks()
    vi.unstubAllGlobals()
    saveDownloadMock.mockReset()
  })

  it('saves through the desktop RPC when available', async () => {
    saveDownloadMock.mockResolvedValue({ ok: true, path: '/tmp/note.txt' })
    const blob = new Blob(['hello'], { type: 'text/plain' })

    await downloadBlob('note.txt', blob)

    expect(saveDownloadMock).toHaveBeenCalledTimes(1)
    const payload = saveDownloadMock.mock.calls[0]?.[0] as { filename: string; data: Uint8Array }
    expect(payload.filename).toBe('note.txt')
    expect(payload.data).toEqual(new Uint8Array([104, 101, 108, 108, 111]))
  })

  it('falls back to a temporary download anchor when RPC is unavailable', async () => {
    vi.useFakeTimers()
    saveDownloadMock.mockRejectedValue(new Error('no rpc'))

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
    await downloadBlob('note.txt', blob, 1_000)

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
