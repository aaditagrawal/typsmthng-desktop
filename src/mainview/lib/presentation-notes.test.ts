import { describe, expect, it } from 'vitest'
import {
  mergeNotesForSlide,
  normalizeInlineNotes,
  notesSidecarPath,
  parseSidecarNotes,
  serializeSidecarNotes,
} from './presentation-notes'

describe('notesSidecarPath', () => {
  it('replaces the extension with .notes.md', () => {
    expect(notesSidecarPath('deck.typ')).toBe('deck.notes.md')
    expect(notesSidecarPath('talks/2026/keynote.typ')).toBe('talks/2026/keynote.notes.md')
  })

  it('handles dotted directories and extensionless files', () => {
    expect(notesSidecarPath('v1.2/slides')).toBe('v1.2/slides.notes.md')
    expect(notesSidecarPath('a\\b\\main.typ')).toBe('a/b/main.notes.md')
  })
})

describe('parseSidecarNotes', () => {
  it('reads "## Slide N" sections into zero-based indices', () => {
    const { notes } = parseSidecarNotes([
      '# Speaker notes — deck',
      '',
      '## Slide 1',
      'Welcome everyone.',
      '',
      '## Slide 3',
      'Skip slide 2.',
      'Second paragraph.',
    ].join('\n'))

    expect(notes.get(0)).toBe('Welcome everyone.')
    expect(notes.has(1)).toBe(false)
    expect(notes.get(2)).toBe('Skip slide 2.\nSecond paragraph.')
  })

  it('accepts bare numeric headings, CRLF, and ignores preamble text', () => {
    const { notes } = parseSidecarNotes('intro text\r\n## 2\r\nhello\r\n')
    expect(notes.get(1)).toBe('hello')
    expect(notes.size).toBe(1)
  })

  it('drops sections that are only whitespace', () => {
    const { notes } = parseSidecarNotes('## Slide 1\n\n   \n## Slide 2\nx')
    expect(notes.has(0)).toBe(false)
    expect(notes.get(1)).toBe('x')
  })
})

describe('serializeSidecarNotes', () => {
  it('round-trips through parse in slide order', () => {
    const notes = new Map<number, string>([[4, 'five'], [0, 'one'], [2, '']])
    const markdown = serializeSidecarNotes(notes, 'deck.typ')

    expect(markdown.startsWith('# Speaker notes — deck.typ\n')).toBe(true)
    expect(markdown.indexOf('## Slide 1')).toBeLessThan(markdown.indexOf('## Slide 5'))
    expect(markdown).not.toContain('## Slide 3')

    const parsed = parseSidecarNotes(markdown).notes
    expect(parsed.get(0)).toBe('one')
    expect(parsed.get(4)).toBe('five')
    expect(parsed.size).toBe(2)
  })
})

describe('normalizeInlineNotes', () => {
  it('accepts page/text pairs and common aliases', () => {
    expect(normalizeInlineNotes([
      { page: 2, text: 'a' },
      { page: '3', note: 'b' },
      { page: 4, body: 'c' },
    ])).toEqual([
      { page: 2, text: 'a' },
      { page: 3, text: 'b' },
      { page: 4, text: 'c' },
    ])
  })

  it('ignores malformed entries', () => {
    expect(normalizeInlineNotes([null, 'x', { page: 0, text: 'bad' }, { text: 'no page' }, { page: 1 }])).toEqual([])
    expect(normalizeInlineNotes('not an array')).toEqual([])
  })
})

describe('mergeNotesForSlide', () => {
  it('pairs inline notes by page with the editable sidecar text', () => {
    const inline = [{ page: 2, text: 'from typst' }, { page: 2, text: 'second' }, { page: 5, text: 'later' }]
    const sidecar = new Map([[1, 'mine']])

    expect(mergeNotesForSlide(1, sidecar, inline)).toEqual({ inline: ['from typst', 'second'], sidecar: 'mine' })
    expect(mergeNotesForSlide(0, sidecar, inline)).toEqual({ inline: [], sidecar: '' })
  })
})
