export function normalizeDiagnosticPath(input: string): string {
  return input.replace(/\\/g, '/').replace(/^\/+/, '')
}

/**
 * After selectFile + optional hydrate, wait until the editor document matches the
 * diagnostic path before jumping. Fixed timeouts race unloaded companion files.
 */
export async function waitForEditorPath<TView>(
  targetPath: string | null | undefined,
  options?: {
    timeoutMs?: number
    isCurrentPath?: () => string | null
    isFileLoaded?: (path: string) => boolean
    getEditorView?: () => TView | null
  },
): Promise<TView | null> {
  const normalizedTarget = targetPath ? normalizeDiagnosticPath(targetPath) : null
  const timeoutMs = options?.timeoutMs ?? 2000
  const deadline = Date.now() + timeoutMs

  while (Date.now() <= deadline) {
    const currentPath = options?.isCurrentPath?.() ?? null
    const normalizedCurrent = currentPath ? normalizeDiagnosticPath(currentPath) : null
    const pathReady = !normalizedTarget || normalizedCurrent === normalizedTarget
    const loaded = !normalizedTarget || (options?.isFileLoaded?.(normalizedTarget) ?? true)
    const view = options?.getEditorView?.() ?? null

    if (pathReady && loaded && view) {
      return view
    }

    await new Promise<void>((resolve) => {
      setTimeout(resolve, 16)
    })
  }

  return options?.getEditorView?.() ?? null
}
