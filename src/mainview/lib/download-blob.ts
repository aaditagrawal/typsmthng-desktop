import { desktopRpc } from '@/lib/desktop-rpc'

function triggerAnchorDownload(filename: string, blob: Blob, revokeDelayMs: number): void {
  const url = URL.createObjectURL(blob)
  const anchor = document.createElement('a')
  anchor.href = url
  anchor.download = filename
  anchor.style.display = 'none'
  document.body.appendChild(anchor)
  anchor.click()
  anchor.remove()
  setTimeout(() => URL.revokeObjectURL(url), revokeDelayMs)
}

/** Save a blob to Downloads via the desktop shell, falling back to an anchor click. */
export async function downloadBlob(filename: string, blob: Blob, revokeDelayMs = 10_000): Promise<void> {
  try {
    const data = new Uint8Array(await blob.arrayBuffer())
    const result = await desktopRpc.request.saveDownload({ filename, data })
    if (result?.ok) return
  } catch {
    // Tests and webviews without the RPC fall back to a download anchor.
  }

  triggerAnchorDownload(filename, blob, revokeDelayMs)
}
