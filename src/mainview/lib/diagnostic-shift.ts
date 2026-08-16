import type { Diagnostic } from '@/stores/compile-store'

function shiftRangeLineNumber(line: number, delta: number): number {
  return Math.max(0, line + delta)
}

/** Shift 0-based compiler range line numbers, leaving columns unchanged. */
export function shiftDiagnosticRange(range: string, lineDelta: number): string {
  if (!range || lineDelta === 0) return range
  return range.replace(/(\d+)(?=:\d+)/g, (line) => String(shiftRangeLineNumber(parseInt(line, 10), lineDelta)))
}

export function shiftMainFileDiagnostics(
  diagnostics: Diagnostic[],
  mainPath: string,
  preambleLineCount: number,
): Diagnostic[] {
  if (preambleLineCount === 0) return diagnostics
  const normalizedMain = mainPath.replace(/\\/g, '/').replace(/^\/+/, '')
  return diagnostics.map((diag) => {
    const diagPath = (diag.path || '').replace(/\\/g, '/').replace(/^\/+/, '')
    if (diagPath && diagPath !== normalizedMain) return diag
    return {
      ...diag,
      range: shiftDiagnosticRange(diag.range, -preambleLineCount),
    }
  })
}
