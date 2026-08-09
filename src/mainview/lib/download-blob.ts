/** Trigger a browser download for a Blob via a temporary anchor element. */
export function downloadBlob(filename: string, blob: Blob, revokeDelayMs = 10_000): void {
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
