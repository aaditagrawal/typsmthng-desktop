/**
 * Speaker notes live in a Markdown sidecar next to the presented file
 * (`deck.typ` -> `deck.notes.md`) so they survive recompiles, stay out of the
 * rendered document, and remain plain text for git. Notes can also come from
 * the document itself via `<typsmthng-note>` metadata; those are merged in at
 * display time and never written back.
 */

export const NOTES_SIDECAR_SUFFIX = '.notes.md'
export const SPEAKER_NOTE_LABEL = 'typsmthng-note'

/** Snippet users can paste into a deck to write notes inline. */
export const SPEAKER_NOTE_SNIPPET =
  `#let note(text) = context [#metadata((page: here().page(), text: text)) <${SPEAKER_NOTE_LABEL}>]`

const HEADING_RE = /^##\s+(?:slide\s+)?(\d+)\s*$/i

export interface SidecarNotes {
  /** Zero-based slide index -> note text. */
  notes: Map<number, string>
}

export function notesSidecarPath(sourcePath: string): string {
  const normalized = sourcePath.replace(/\\/g, '/')
  const dot = normalized.lastIndexOf('.')
  const slash = normalized.lastIndexOf('/')
  const base = dot > slash ? normalized.slice(0, dot) : normalized
  return `${base}${NOTES_SIDECAR_SUFFIX}`
}

export function parseSidecarNotes(markdown: string): SidecarNotes {
  const notes = new Map<number, string>()
  let current: number | null = null
  let buffer: string[] = []

  const flush = () => {
    if (current === null) return
    const text = buffer.join('\n').trim()
    if (text) notes.set(current, text)
    buffer = []
  }

  for (const rawLine of markdown.split(/\r?\n/)) {
    const match = HEADING_RE.exec(rawLine.trim())
    if (match) {
      flush()
      current = Math.max(0, Number.parseInt(match[1], 10) - 1)
      continue
    }
    if (current !== null) buffer.push(rawLine)
  }
  flush()

  return { notes }
}

export function serializeSidecarNotes(notes: Map<number, string>, title: string): string {
  const lines = [`# Speaker notes — ${title}`, '']
  const indices = Array.from(notes.keys()).sort((a, b) => a - b)
  for (const index of indices) {
    const text = notes.get(index)?.trim()
    if (!text) continue
    lines.push(`## Slide ${index + 1}`, '', text, '')
  }
  return lines.join('\n').replace(/\n+$/, '\n')
}

export interface InlineSpeakerNote {
  page: number
  text: string
}

/** Normalise raw `query(<typsmthng-note>, field: "value")` output. */
export function normalizeInlineNotes(raw: unknown): InlineSpeakerNote[] {
  if (!Array.isArray(raw)) return []
  const result: InlineSpeakerNote[] = []
  for (const entry of raw) {
    if (!entry || typeof entry !== 'object') continue
    const record = entry as Record<string, unknown>
    const page = typeof record.page === 'number' ? record.page : Number.parseInt(String(record.page ?? ''), 10)
    const text = typeof record.text === 'string'
      ? record.text
      : typeof record.note === 'string'
        ? record.note
        : typeof record.body === 'string'
          ? record.body
          : null
    if (!Number.isFinite(page) || page < 1 || text === null) continue
    result.push({ page, text })
  }
  return result
}

/**
 * Merge sidecar and inline notes for a slide. Inline notes render first
 * because they sit next to the slide's source; sidecar notes are the
 * presenter's editable layer on top.
 */
export function mergeNotesForSlide(
  index: number,
  sidecar: Map<number, string>,
  inline: InlineSpeakerNote[],
): { inline: string[]; sidecar: string } {
  return {
    inline: inline.filter((note) => note.page - 1 === index).map((note) => note.text),
    sidecar: sidecar.get(index) ?? '',
  }
}
