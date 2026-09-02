import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { ChevronDown, HelpCircle, Maximize2, Minimize2, PanelRight, PanelRightClose, X } from 'lucide-react'
import { useShallow } from 'zustand/react/shallow'
import type { NotesLayout } from '../../../shared/presentation'
import { isMacOS } from '@/lib/platform'
import { resolvePresentationMouseButton } from '@/lib/presentation-input'
import { usePresentationStore } from '@/stores/presentation-store'
import { DisplayPicker } from './display-picker'
import { NotesPanel } from './notes-panel'
import { PresentationToolbar } from './presentation-toolbar'
import { ElapsedTimer, WallClock } from './presentation-timer'
import { Divider, Kbd, MONO_LABEL, PButton, SectionLabel } from './presentation-ui'
import { SlideGrid } from './slide-grid'
import { SlideStage } from './slide-stage'
import { usePresentationKeyboard, usePresentationPointerNavigation } from './use-presentation-input'

const NOTES_LAYOUT_OPTIONS: Array<{ value: NotesLayout; label: string; hint: string }> = [
  { value: 'auto', label: 'Auto-detect', hint: 'Split double-width pages (touying / polylux notes)' },
  { value: 'right', label: 'Right half', hint: 'Audience sees the left half; notes on the right' },
  { value: 'none', label: 'Whole page', hint: 'Show the full page to the audience' },
]

const SHORTCUTS: Array<[string, string]> = [
  ['→ Space PgDn', 'Next slide'],
  ['← PgUp Backspace', 'Previous slide'],
  ['Home / End', 'First / last slide'],
  ['12 ⏎', 'Jump to slide 12'],
  ['B / W', 'Black / white screen'],
  ['L', 'Laser pointer'],
  ['D / H / E', 'Pen / highlighter / eraser'],
  ['C', 'Clear annotations'],
  ['G', 'All slides'],
  ['S', 'Show / hide notes'],
  ['T / R', 'Pause / reset timer'],
  ['F', 'Toggle fullscreen'],
  ['Esc', 'End presentation'],
]

function NotesLayoutMenu() {
  const { notesLayout, setNotesLayout, hasRenderedNotes } = usePresentationStore(
    useShallow((s) => ({
      notesLayout: s.notesLayout,
      setNotesLayout: s.setNotesLayout,
      hasRenderedNotes: s.slideDeck?.hasRenderedNotes ?? false,
    })),
  )
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const handle = (event: MouseEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) setOpen(false)
    }
    document.addEventListener('mousedown', handle)
    return () => document.removeEventListener('mousedown', handle)
  }, [open])

  const current = NOTES_LAYOUT_OPTIONS.find((option) => option.value === notesLayout) ?? NOTES_LAYOUT_OPTIONS[0]

  return (
    <div ref={ref} style={{ position: 'relative' }}>
      <PButton onClick={() => setOpen((value) => !value)} title="How notes are laid out inside the compiled pages" active={hasRenderedNotes}>
        <span style={MONO_LABEL}>Page notes · {current.label}</span>
        <ChevronDown size={10} />
      </PButton>
      {open && (
        <div
          style={{
            position: 'absolute',
            top: '100%',
            right: 0,
            marginTop: '6px',
            minWidth: '280px',
            background: 'var(--p-elevated)',
            border: '1px solid var(--p-border-strong)',
            borderRadius: '3px',
            boxShadow: '0 12px 32px rgba(0,0,0,0.5)',
            zIndex: 60,
            padding: '6px 0',
          }}
        >
          {NOTES_LAYOUT_OPTIONS.map((option) => (
            <button
              key={option.value}
              type="button"
              className="presentation-menu-item"
              data-active={option.value === notesLayout ? 'true' : undefined}
              onClick={() => {
                setNotesLayout(option.value)
                setOpen(false)
              }}
              style={{ flexDirection: 'column', alignItems: 'flex-start', gap: '2px' }}
            >
              <span>{option.label}</span>
              <span style={{ fontSize: '10px', color: 'var(--p-text-faint)' }}>{option.hint}</span>
            </button>
          ))}
        </div>
      )}
    </div>
  )
}

function ShortcutHelp() {
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    const handle = (event: MouseEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) setOpen(false)
    }
    document.addEventListener('mousedown', handle)
    return () => document.removeEventListener('mousedown', handle)
  }, [open])

  return (
    <div ref={ref} style={{ position: 'relative' }}>
      <PButton onClick={() => setOpen((value) => !value)} title="Keyboard shortcuts" active={open}>
        <HelpCircle size={13} />
      </PButton>
      {open && (
        <div
          style={{
            position: 'absolute',
            top: '100%',
            right: 0,
            marginTop: '6px',
            width: '300px',
            background: 'var(--p-elevated)',
            border: '1px solid var(--p-border-strong)',
            borderRadius: '3px',
            boxShadow: '0 12px 32px rgba(0,0,0,0.5)',
            zIndex: 60,
            padding: '10px 12px',
            display: 'grid',
            gridTemplateColumns: 'auto 1fr',
            rowGap: '7px',
            columnGap: '12px',
            fontSize: '11px',
            color: 'var(--p-text-dim)',
          }}
        >
          {SHORTCUTS.map(([keys, label]) => (
            <div key={keys} style={{ display: 'contents' }}>
              <div className="flex" style={{ gap: '3px', flexWrap: 'wrap' }}>
                {keys.split(' ').map((key) => <Kbd key={key}>{key}</Kbd>)}
              </div>
              <span style={{ alignSelf: 'center' }}>{label}</span>
            </div>
          ))}
          <div style={{ gridColumn: '1 / -1', marginTop: '4px', color: 'var(--p-text-faint)', fontSize: '10px', lineHeight: 1.5 }}>
            Presentation remotes send these same keys. Click or right-click the slide to move forward or back.
          </div>
        </div>
      )}
    </div>
  )
}

export default function PresenterShell() {
  const {
    slideDeck, state, deckTitle, gridOpen, setGridOpen, sidebarOpen, setSidebarOpen, goto, performAction,
    upsertStroke, eraseStroke, setLaser, end, audienceOpen, lastError, mainFullScreen, setMainFullScreen,
  } = usePresentationStore(
    useShallow((s) => ({
      slideDeck: s.slideDeck,
      state: s.state,
      deckTitle: s.deck?.title ?? 'Presentation',
      gridOpen: s.gridOpen,
      setGridOpen: s.setGridOpen,
      sidebarOpen: s.sidebarOpen,
      setSidebarOpen: s.setSidebarOpen,
      goto: s.goto,
      performAction: s.performAction,
      upsertStroke: s.upsertStroke,
      eraseStroke: s.eraseStroke,
      setLaser: s.setLaser,
      end: s.end,
      audienceOpen: s.audienceOpen,
      lastError: s.lastError,
      mainFullScreen: s.mainFullScreen,
      setMainFullScreen: s.setMainFullScreen,
    })),
  )
  const gotoBuffer = usePresentationKeyboard({ onAction: performAction, onGoto: goto })
  const wheelRef = usePresentationPointerNavigation<HTMLDivElement>(performAction, !gridOpen)

  const slideCount = slideDeck?.slideCount ?? 0
  const current = state.slide
  const hasNext = current + 1 < slideCount
  const progress = slideCount > 0 ? (current + 1) / slideCount : 0

  const annotatedSlides = useMemo(() => {
    const set = new Set<number>()
    for (const [key, strokes] of Object.entries(state.annotations)) {
      if (strokes.length > 0) set.add(Number(key))
    }
    return set
  }, [state.annotations])

  const handleSurfaceClick = useCallback((event: React.MouseEvent<HTMLDivElement>) => {
    if (state.tool !== 'pointer') return
    const action = resolvePresentationMouseButton(event.button)
    if (action) performAction(action)
  }, [state.tool, performAction])

  const handleContextMenu = useCallback((event: React.MouseEvent<HTMLDivElement>) => {
    event.preventDefault()
    if (state.tool !== 'pointer') return
    performAction('prev')
  }, [state.tool, performAction])

  return (
    <div className="presentation-root flex flex-col h-full w-full select-none" style={{ position: 'relative' }}>
      {/* Header */}
      <header
        className="flex items-center shrink-0 electrobun-webkit-app-region-drag"
        style={{
          height: '48px',
          paddingLeft: isMacOS && !mainFullScreen ? '84px' : '16px',
          paddingRight: '14px',
          gap: '14px',
          borderBottom: '1px solid var(--p-border)',
          background: 'var(--p-surface)',
        }}
      >
        <div className="flex items-center" style={{ gap: '10px', minWidth: 0, flex: 1 }}>
          <span
            style={{
              ...MONO_LABEL,
              padding: '3px 6px',
              borderRadius: '1px',
              background: 'var(--accent)',
              color: '#fff',
              flexShrink: 0,
            }}
          >
            Presenter
          </span>
          <span
            className="truncate"
            style={{ fontFamily: 'var(--font-mono)', fontSize: '12px', color: 'var(--p-text-dim)', letterSpacing: '0.03em' }}
            title={deckTitle}
          >
            {deckTitle}
          </span>
          <span
            className="flex items-center"
            style={{ ...MONO_LABEL, gap: '6px', color: audienceOpen ? 'var(--p-success)' : 'var(--p-text-faint)', flexShrink: 0 }}
          >
            <span
              aria-hidden
              style={{
                width: '6px',
                height: '6px',
                borderRadius: '50%',
                background: audienceOpen ? 'var(--p-success)' : 'var(--p-text-faint)',
                boxShadow: audienceOpen ? '0 0 8px var(--p-success)' : undefined,
              }}
            />
            {audienceOpen ? 'Live' : 'Not showing'}
          </span>
        </div>

        <div className="flex items-center" style={{ gap: '14px', flexShrink: 0 }}>
          <ElapsedTimer />
          <Divider />
          <WallClock />
          <Divider />
          <DisplayPicker />
          <NotesLayoutMenu />
          <ShortcutHelp />
          <PButton onClick={() => void setMainFullScreen(!mainFullScreen)} title={mainFullScreen ? 'Exit fullscreen (F)' : 'Fullscreen presenter (F)'}>
            {mainFullScreen ? <Minimize2 size={13} /> : <Maximize2 size={13} />}
          </PButton>
          <PButton onClick={() => setSidebarOpen(!sidebarOpen)} title={sidebarOpen ? 'Hide next slide and notes (S)' : 'Show next slide and notes (S)'}>
            {sidebarOpen ? <PanelRightClose size={13} /> : <PanelRight size={13} />}
          </PButton>
          <PButton tone="danger" onClick={() => void end()} title="End presentation (Esc)" label="End">
            <X size={12} />
          </PButton>
        </div>
      </header>

      {/* Progress */}
      <div aria-hidden style={{ height: '2px', background: 'var(--p-border)', flexShrink: 0 }}>
        <div style={{ height: '100%', width: `${progress * 100}%`, background: 'var(--accent)', transition: 'width 200ms ease' }} />
      </div>

      {lastError && (
        <div
          style={{
            padding: '8px 16px',
            background: 'color-mix(in srgb, var(--p-danger) 12%, transparent)',
            borderBottom: '1px solid color-mix(in srgb, var(--p-danger) 35%, transparent)',
            color: 'var(--p-danger)',
            fontSize: '11px',
          }}
        >
          {lastError}
        </div>
      )}

      {/* Body */}
      <div className="flex flex-1 min-h-0">
        <div ref={wheelRef} className="flex flex-col min-w-0" style={{ flex: 1, padding: '18px 20px 12px' }}>
          <div className="flex items-center" style={{ marginBottom: '10px', gap: '14px', minHeight: '16px' }}>
            <SectionLabel>Current · {String(current + 1).padStart(2, '0')}</SectionLabel>
            <div className="flex items-center" style={{ gap: '12px' }}>
              {gotoBuffer && (
                <span style={{ ...MONO_LABEL, color: 'var(--accent)' }}>
                  Go to {gotoBuffer} <Kbd>⏎</Kbd>
                </span>
              )}
              {state.blackout !== 'none' && (
                <span style={{ ...MONO_LABEL, color: 'var(--p-text-dim)' }}>
                  Audience sees {state.blackout} · press {state.blackout === 'black' ? 'B' : 'W'} to restore
                </span>
              )}
              {state.laserEnabled && (
                <span style={{ ...MONO_LABEL, color: 'var(--accent)' }}>Laser on</span>
              )}
            </div>
          </div>
          <div style={{ flex: 1, minHeight: 0 }}>
            <SlideStage
              deck={slideDeck}
              slide={current}
              state={state}
              interactive
              framed
              background="transparent"
              onSurfaceClick={handleSurfaceClick}
              onContextMenu={handleContextMenu}
              onStroke={upsertStroke}
              onErase={eraseStroke}
              onLaser={setLaser}
              emptyLabel="Compile the document to get slides"
            />
          </div>
        </div>

        {sidebarOpen && (
          <aside
            className="flex flex-col shrink-0 min-h-0"
            data-no-slide-wheel
            style={{
              width: 'clamp(280px, 32%, 460px)',
              padding: '18px 20px 12px 0',
              gap: '14px',
            }}
          >
            <div className="flex flex-col" style={{ gap: '8px', flexShrink: 0 }}>
              <SectionLabel>{hasNext ? `Next · ${String(current + 2).padStart(2, '0')}` : 'Next'}</SectionLabel>
              <div
                style={{
                  aspectRatio: slideDeck ? String(slideDeck.aspectRatio(Math.min(current + 1, Math.max(slideCount - 1, 0)))) : '16 / 9',
                  border: '1px solid var(--p-border)',
                  borderRadius: '2px',
                  overflow: 'hidden',
                }}
              >
                {hasNext && slideDeck ? (
                  <SlideStage
                    deck={slideDeck}
                    slide={current + 1}
                    state={state}
                    showAnnotations={false}
                    showLaser={false}
                    showBlackout={false}
                    background="var(--p-surface)"
                  />
                ) : (
                  <div
                    className="flex items-center justify-center h-full"
                    style={{ ...MONO_LABEL, color: 'var(--p-text-faint)', background: 'var(--p-surface)' }}
                  >
                    {slideCount > 0 ? 'End of slides' : 'No slides yet'}
                  </div>
                )}
              </div>
            </div>
            <NotesPanel slide={current} />
          </aside>
        )}
      </div>

      {/* Footer */}
      <footer
        className="flex items-center justify-between shrink-0"
        style={{
          minHeight: '52px',
          padding: '8px 20px',
          borderTop: '1px solid var(--p-border)',
          background: 'var(--p-surface)',
          gap: '12px',
        }}
      >
        <PresentationToolbar />
        <span style={{ ...MONO_LABEL, color: 'var(--p-text-faint)', flexShrink: 0 }}>
          Click slide to advance · Right-click to go back
        </span>
      </footer>

      {gridOpen && slideDeck && (
        <SlideGrid
          deck={slideDeck}
          current={current}
          annotatedSlides={annotatedSlides}
          onSelect={(slide) => {
            goto(slide)
            setGridOpen(false)
          }}
          onClose={() => setGridOpen(false)}
        />
      )}
    </div>
  )
}
