import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { LayoutGrid, Maximize2, Minimize2, StickyNote, X } from 'lucide-react'
import { useShallow } from 'zustand/react/shallow'
import { resolvePresentationMouseButton } from '@/lib/presentation-input'
import { usePresentationStore } from '@/stores/presentation-store'
import { NotesPanel } from './notes-panel'
import { PresentationToolbar } from './presentation-toolbar'
import { ElapsedTimer, WallClock } from './presentation-timer'
import { Divider, Kbd, MONO_LABEL, PButton, SectionLabel } from './presentation-ui'
import { SlideGrid } from './slide-grid'
import { SlideStage } from './slide-stage'
import { usePresentationKeyboard, usePresentationPointerNavigation } from './use-presentation-input'

const HUD_HIDE_DELAY_MS = 2600
const NOTES_DRAWER_WIDTH = 'clamp(300px, 34%, 480px)'

/**
 * Keeps chrome visible while the mouse moves and hides it (plus the cursor)
 * shortly after it stops, so a single-screen talk looks like a real deck.
 */
function useAutoHide(active: boolean): { visible: boolean; hold: (hold: boolean) => void; poke: () => void } {
  const [visible, setVisible] = useState(true)
  const holdRef = useRef(false)
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const schedule = useCallback(() => {
    if (timerRef.current) clearTimeout(timerRef.current)
    timerRef.current = setTimeout(() => {
      if (!holdRef.current) setVisible(false)
    }, HUD_HIDE_DELAY_MS)
  }, [])

  const poke = useCallback(() => {
    setVisible(true)
    schedule()
  }, [schedule])

  const hold = useCallback((value: boolean) => {
    holdRef.current = value
    if (value) {
      if (timerRef.current) clearTimeout(timerRef.current)
      setVisible(true)
    } else {
      schedule()
    }
  }, [schedule])

  useEffect(() => {
    if (!active) {
      setVisible(true)
      return
    }
    schedule()
    window.addEventListener('mousemove', poke)
    window.addEventListener('keydown', poke)
    return () => {
      window.removeEventListener('mousemove', poke)
      window.removeEventListener('keydown', poke)
      if (timerRef.current) clearTimeout(timerRef.current)
    }
  }, [active, poke, schedule])

  return { visible, hold, poke }
}

export default function SingleWindowPresentation() {
  const {
    slideDeck, state, gridOpen, notesOverlayOpen, setGridOpen, setNotesOverlayOpen, goto, performAction,
    upsertStroke, eraseStroke, setLaser, end, mainFullScreen, setMainFullScreen,
  } = usePresentationStore(
    useShallow((s) => ({
      slideDeck: s.slideDeck,
      state: s.state,
      gridOpen: s.gridOpen,
      notesOverlayOpen: s.notesOverlayOpen,
      setGridOpen: s.setGridOpen,
      setNotesOverlayOpen: s.setNotesOverlayOpen,
      goto: s.goto,
      performAction: s.performAction,
      upsertStroke: s.upsertStroke,
      eraseStroke: s.eraseStroke,
      setLaser: s.setLaser,
      end: s.end,
      mainFullScreen: s.mainFullScreen,
      setMainFullScreen: s.setMainFullScreen,
    })),
  )
  const gotoBuffer = usePresentationKeyboard({ onAction: performAction, onGoto: goto })
  const wheelRef = usePresentationPointerNavigation<HTMLDivElement>(performAction, !gridOpen)
  const hud = useAutoHide(!gridOpen && !notesOverlayOpen)

  const slideCount = slideDeck?.slideCount ?? 0
  const current = state.slide
  const hasNext = current + 1 < slideCount
  const chromeVisible = hud.visible || notesOverlayOpen || gridOpen

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
    <div
      ref={wheelRef}
      className="presentation-root h-full w-full select-none"
      style={{ position: 'relative' }}
    >
      <SlideStage
        deck={slideDeck}
        slide={current}
        state={state}
        interactive
        cursorHidden={!chromeVisible}
        onSurfaceClick={handleSurfaceClick}
        onContextMenu={handleContextMenu}
        onStroke={upsertStroke}
        onErase={eraseStroke}
        onLaser={setLaser}
        emptyLabel="Compile the document to get slides"
      />

      {gotoBuffer && (
        <div
          style={{
            position: 'absolute',
            top: '24px',
            left: '50%',
            transform: 'translateX(-50%)',
            padding: '8px 14px',
            background: 'rgba(8,8,10,0.85)',
            border: '1px solid var(--p-border-strong)',
            borderRadius: '3px',
            ...MONO_LABEL,
            fontSize: '13px',
            color: 'var(--p-text)',
          }}
        >
          Go to slide {gotoBuffer} <Kbd>⏎</Kbd>
        </div>
      )}

      {state.blackout !== 'none' && chromeVisible && (
        <div
          style={{
            position: 'absolute',
            top: '24px',
            left: '50%',
            transform: 'translateX(-50%)',
            padding: '6px 12px',
            background: 'rgba(8,8,10,0.85)',
            border: '1px solid var(--p-border-strong)',
            borderRadius: '3px',
            ...MONO_LABEL,
            color: 'var(--p-text-dim)',
          }}
        >
          {state.blackout === 'black' ? 'Black' : 'White'} screen · press <Kbd>{state.blackout === 'black' ? 'B' : 'W'}</Kbd> or click to resume
        </div>
      )}

      {/* Bottom HUD, centred within the area not covered by the notes drawer */}
      <div
        className="presentation-hud"
        data-hidden={chromeVisible ? 'false' : 'true'}
        style={{
          position: 'absolute',
          left: '18px',
          right: notesOverlayOpen ? `calc(${NOTES_DRAWER_WIDTH} + 18px)` : '18px',
          bottom: '18px',
          display: 'flex',
          justifyContent: 'center',
          pointerEvents: 'none',
          zIndex: 20,
        }}
      >
        <div
          onMouseEnter={() => hud.hold(true)}
          onMouseLeave={() => hud.hold(false)}
          style={{
            display: 'flex',
            alignItems: 'center',
            gap: '12px',
            padding: '8px 12px',
            maxWidth: '100%',
            overflowX: 'auto',
            background: 'rgba(12,12,14,0.86)',
            backdropFilter: 'blur(14px)',
            border: '1px solid var(--p-border-strong)',
            borderRadius: '4px',
            boxShadow: '0 12px 40px rgba(0,0,0,0.5)',
            pointerEvents: 'auto',
          }}
        >
          <PresentationToolbar variant="hud" />
          <Divider />
          <ElapsedTimer />
          <Divider />
          <WallClock />
          <Divider />
          <PButton active={gridOpen} onClick={() => setGridOpen(!gridOpen)} title="All slides (G)">
            <LayoutGrid size={13} />
          </PButton>
          <PButton active={notesOverlayOpen} onClick={() => setNotesOverlayOpen(!notesOverlayOpen)} title="Notes and next slide (S)">
            <StickyNote size={13} />
          </PButton>
          <PButton onClick={() => void setMainFullScreen(!mainFullScreen)} title={mainFullScreen ? 'Exit fullscreen (F)' : 'Fullscreen (F)'}>
            {mainFullScreen ? <Minimize2 size={13} /> : <Maximize2 size={13} />}
          </PButton>
          <PButton tone="danger" onClick={() => void end()} title="End presentation (Esc)">
            <X size={13} />
          </PButton>
        </div>
      </div>

      {/* Notes drawer */}
      {notesOverlayOpen && (
        <aside
          className="flex flex-col"
          data-no-slide-wheel
          onMouseEnter={() => hud.hold(true)}
          onMouseLeave={() => hud.hold(false)}
          style={{
            position: 'absolute',
            top: 0,
            right: 0,
            bottom: 0,
            width: NOTES_DRAWER_WIDTH,
            padding: '18px 18px 90px',
            gap: '14px',
            background: 'rgba(10,10,12,0.92)',
            backdropFilter: 'blur(16px)',
            borderLeft: '1px solid var(--p-border)',
            zIndex: 30,
          }}
        >
          <div className="flex items-center justify-between">
            <SectionLabel>{hasNext ? `Next · ${String(current + 2).padStart(2, '0')}` : 'Next'}</SectionLabel>
            <PButton compact onClick={() => setNotesOverlayOpen(false)} title="Close notes (S)">
              <X size={11} />
            </PButton>
          </div>
          <div
            style={{
              flexShrink: 0,
              aspectRatio: slideDeck ? String(slideDeck.aspectRatio(Math.min(current + 1, Math.max(slideCount - 1, 0)))) : '16 / 9',
              border: '1px solid var(--p-border)',
              borderRadius: '2px',
              overflow: 'hidden',
            }}
          >
            {hasNext && slideDeck ? (
              <SlideStage deck={slideDeck} slide={current + 1} state={state} showAnnotations={false} showLaser={false} showBlackout={false} background="var(--p-surface)" />
            ) : (
              <div className="flex items-center justify-center h-full" style={{ ...MONO_LABEL, color: 'var(--p-text-faint)', background: 'var(--p-surface)' }}>
                End of slides
              </div>
            )}
          </div>
          <NotesPanel slide={current} compact />
        </aside>
      )}

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
