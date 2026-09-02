import { useEffect, useState } from 'react'
import { Minus, Plus } from 'lucide-react'
import { useShallow } from 'zustand/react/shallow'
import { createInitialPresentationState } from '../../../shared/presentation'
import { mergeNotesForSlide, SPEAKER_NOTE_SNIPPET } from '@/lib/presentation-notes'
import { usePresentationStore } from '@/stores/presentation-store'
import { SlideStage } from './slide-stage'
import { MONO_LABEL, PButton, SectionLabel } from './presentation-ui'

const NOTES_SIZE_KEY = 'typsmthng.presentation.notesFontSize'
const NOTES_SIZE_MIN = 12
const NOTES_SIZE_MAX = 34
const NEUTRAL_STATE = createInitialPresentationState()

function loadNotesSize(): number {
  try {
    const stored = Number(window.localStorage?.getItem(NOTES_SIZE_KEY))
    if (Number.isFinite(stored) && stored >= NOTES_SIZE_MIN && stored <= NOTES_SIZE_MAX) return stored
  } catch {}
  return 17
}

export function NotesPanel({ slide, compact = false }: { slide: number; compact?: boolean }) {
  const { slideDeck, sidecarNotes, inlineNotes, setSidecarNote, sidecarPath } = usePresentationStore(
    useShallow((s) => ({
      slideDeck: s.slideDeck,
      sidecarNotes: s.sidecarNotes,
      inlineNotes: s.inlineNotes,
      setSidecarNote: s.setSidecarNote,
      sidecarPath: s.sidecarPath,
    })),
  )
  const [fontSize, setFontSize] = useState(loadNotesSize)
  const merged = mergeNotesForSlide(slide, sidecarNotes, inlineNotes)
  const draft = merged.sidecar

  useEffect(() => {
    try {
      window.localStorage?.setItem(NOTES_SIZE_KEY, String(fontSize))
    } catch {}
  }, [fontSize])

  const hasRenderedNotes = slideDeck?.hasRenderedNotes ?? false
  const hasAnything = hasRenderedNotes || merged.inline.length > 0 || draft.trim().length > 0

  return (
    <div className="flex flex-col min-h-0" style={{ flex: 1, gap: '10px', ['--p-notes-size' as string]: `${fontSize}px` }}>
      <div className="flex items-center justify-between">
        <SectionLabel>Notes</SectionLabel>
        <div className="flex items-center" style={{ gap: '4px' }}>
          <PButton compact onClick={() => setFontSize((v) => Math.max(NOTES_SIZE_MIN, v - 2))} title="Smaller notes">
            <Minus size={11} />
          </PButton>
          <span style={{ ...MONO_LABEL, color: 'var(--p-text-faint)', minWidth: '28px', textAlign: 'center' }}>{fontSize}</span>
          <PButton compact onClick={() => setFontSize((v) => Math.min(NOTES_SIZE_MAX, v + 2))} title="Larger notes">
            <Plus size={11} />
          </PButton>
        </div>
      </div>

      {hasRenderedNotes && slideDeck && (
        <div
          style={{
            flexShrink: 0,
            aspectRatio: String(slideDeck.aspectRatio(slide, 'notes')),
            maxHeight: compact ? '30%' : '40%',
            border: '1px solid var(--p-border)',
            borderRadius: '2px',
            overflow: 'hidden',
          }}
          title="Notes rendered by the document (right half of the page)"
        >
          <SlideStage
            deck={slideDeck}
            slide={slide}
            region="notes"
            state={NEUTRAL_STATE}
            showAnnotations={false}
            showLaser={false}
            showBlackout={false}
            background="var(--p-surface)"
          />
        </div>
      )}

      {merged.inline.length > 0 && (
        <div
          className="overflow-auto"
          style={{
            flexShrink: 0,
            maxHeight: '45%',
            padding: '12px 14px',
            border: '1px solid var(--p-border)',
            borderLeft: '3px solid var(--accent)',
            borderRadius: '2px',
            background: 'var(--p-surface)',
            fontFamily: 'var(--font-sans)',
            fontSize: `${fontSize}px`,
            lineHeight: 1.55,
            color: 'var(--p-text)',
            whiteSpace: 'pre-wrap',
          }}
        >
          {merged.inline.map((text, index) => (
            <p key={index} style={{ marginBottom: index < merged.inline.length - 1 ? '10px' : 0 }}>{text}</p>
          ))}
        </div>
      )}

      <textarea
        className="presentation-notes-editor"
        value={draft}
        spellCheck
        placeholder={hasAnything ? 'Add your own notes for this slide…' : 'Speaker notes for this slide. They are saved next to your deck.'}
        onChange={(event) => setSidecarNote(slide, event.target.value)}
      />

      {!hasAnything && (
        <div style={{ color: 'var(--p-text-faint)', fontSize: '10.5px', lineHeight: 1.6, flexShrink: 0 }}>
          {sidecarPath && (
            <div>
              Saved to <span style={{ color: 'var(--p-text-dim)' }}>{sidecarPath}</span>.
            </div>
          )}
          <div style={{ marginTop: '6px' }}>Or write notes in Typst:</div>
          <code
            style={{
              display: 'block',
              marginTop: '4px',
              padding: '6px 8px',
              background: 'var(--p-surface)',
              border: '1px solid var(--p-border)',
              borderRadius: '2px',
              fontSize: '10px',
              color: 'var(--p-text-dim)',
              whiteSpace: 'pre-wrap',
              wordBreak: 'break-word',
            }}
          >
            {SPEAKER_NOTE_SNIPPET}
            {'\n'}
            {'#note("Mention the deadline.")'}
          </code>
        </div>
      )}
    </div>
  )
}
