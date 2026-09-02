import { useCallback, useEffect, useRef, useState } from 'react'
import { create } from 'zustand'
import {
  createInitialPresentationState,
  type LaserPointer,
  type PresentationAction,
  type PresentationInput,
  type PresentationSnapshot,
  type PresentationState,
  type Stroke,
} from '../../../shared/presentation'
import { desktopRpc, onPresentationSnapshot } from '@/lib/desktop-rpc'
import { resolvePresentationMouseButton } from '@/lib/presentation-input'
import { SlideDeck } from '@/lib/slide-deck'
import { SlideStage } from './slide-stage'
import { MONO_LABEL } from './presentation-ui'
import { usePresentationKeyboard, usePresentationPointerNavigation } from './use-presentation-input'

const CURSOR_HIDE_DELAY_MS = 1800
/** Laser updates are coalesced to one RPC per interval. */
const LASER_SEND_INTERVAL_MS = 24

interface AudienceStoreState {
  deck: SlideDeck | null
  state: PresentationState
  ended: boolean
  connected: boolean
  applySnapshot: (snapshot: PresentationSnapshot) => void
}

const useAudienceStore = create<AudienceStoreState>((set, get) => ({
  deck: null,
  state: createInitialPresentationState(),
  ended: true,
  connected: false,
  applySnapshot: (snapshot) => {
    const patch: Partial<AudienceStoreState> = { connected: true }
    if (snapshot.ended) {
      get().deck?.dispose()
      patch.deck = null
      patch.ended = true
      patch.state = createInitialPresentationState()
      set(patch)
      return
    }
    if (snapshot.deck !== undefined) {
      const previous = get().deck
      if (snapshot.deck === null) {
        previous?.dispose()
        patch.deck = null
      } else if (!previous || previous.revision !== snapshot.deck.revision) {
        patch.deck = new SlideDeck(snapshot.deck)
        previous?.dispose()
      }
    }
    if (snapshot.state !== undefined) {
      patch.state = snapshot.state
      patch.ended = false
    }
    set(patch)
  },
}))

function sendInput(input: PresentationInput): void {
  void desktopRpc.request.presentationInput(input).catch((error) => {
    console.warn('Failed to relay presentation input:', error)
  })
}

/**
 * Coalesces high-frequency inputs (laser motion, in-progress strokes) so only
 * the latest value per key crosses the RPC bridge each interval. Strokes are
 * sent whole and upserted by id, so dropping intermediate frames is lossless.
 */
function useCoalescedSender(): (key: string, input: PresentationInput) => void {
  const pending = useRef(new Map<string, PresentationInput>())
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(() => () => {
    if (timer.current) clearTimeout(timer.current)
  }, [])

  return useCallback((key: string, input: PresentationInput) => {
    pending.current.set(key, input)
    if (timer.current) return
    timer.current = setTimeout(() => {
      timer.current = null
      const batch = Array.from(pending.current.values())
      pending.current.clear()
      for (const entry of batch) sendInput(entry)
    }, LASER_SEND_INTERVAL_MS)
  }, [])
}

function IdleScreen({ connected }: { connected: boolean }) {
  return (
    <div
      className="flex flex-col items-center justify-center h-full w-full"
      style={{ gap: '14px', color: 'var(--p-text-faint)' }}
    >
      <span
        style={{
          ...MONO_LABEL,
          fontSize: '13px',
          padding: '6px 10px',
          borderRadius: '1px',
          background: 'var(--accent)',
          color: '#fff',
        }}
      >
        typsmthng
      </span>
      <span style={{ ...MONO_LABEL, fontSize: '11px' }}>
        {connected ? 'Waiting for the presenter' : 'Connecting…'}
      </span>
    </div>
  )
}

export default function AudienceApp() {
  const deck = useAudienceStore((s) => s.deck)
  const state = useAudienceStore((s) => s.state)
  const ended = useAudienceStore((s) => s.ended)
  const connected = useAudienceStore((s) => s.connected)
  const [cursorHidden, setCursorHidden] = useState(false)
  const cursorTimer = useRef<ReturnType<typeof setTimeout> | null>(null)
  const sendCoalesced = useCoalescedSender()

  useEffect(() => {
    document.documentElement.classList.add('dark', 'opaque')
    document.title = 'typsmthng — Presentation'
    const unsubscribe = onPresentationSnapshot((snapshot) => {
      useAudienceStore.getState().applySnapshot(snapshot)
    })

    // Pull the current snapshot in case the presenter published before this
    // window finished loading, then announce readiness so it republishes.
    void desktopRpc.request.presentationGetSnapshot()
      .then((snapshot) => useAudienceStore.getState().applySnapshot(snapshot))
      .catch(() => {})
      .finally(() => sendInput({ kind: 'ready' }))

    return unsubscribe
  }, [])

  useEffect(() => {
    const poke = () => {
      setCursorHidden(false)
      if (cursorTimer.current) clearTimeout(cursorTimer.current)
      cursorTimer.current = setTimeout(() => setCursorHidden(true), CURSOR_HIDE_DELAY_MS)
    }
    poke()
    window.addEventListener('mousemove', poke)
    return () => {
      window.removeEventListener('mousemove', poke)
      if (cursorTimer.current) clearTimeout(cursorTimer.current)
    }
  }, [])

  const onAction = useCallback((action: PresentationAction) => {
    // Fullscreen is owned by the presenter window; the audience stays fullscreen.
    if (action === 'toggle-fullscreen') return
    sendInput({ kind: 'action', action })
  }, [])
  const onGoto = useCallback((slide: number) => sendInput({ kind: 'goto', slide }), [])
  usePresentationKeyboard({ onAction, onGoto, enabled: !ended })
  const wheelRef = usePresentationPointerNavigation<HTMLDivElement>(onAction, !ended)

  const handleSurfaceClick = useCallback((event: React.MouseEvent<HTMLDivElement>) => {
    if (ended || state.tool !== 'pointer') return
    const action = resolvePresentationMouseButton(event.button)
    if (action) onAction(action)
  }, [ended, state.tool, onAction])

  const handleContextMenu = useCallback((event: React.MouseEvent<HTMLDivElement>) => {
    event.preventDefault()
    if (ended || state.tool !== 'pointer') return
    onAction('prev')
  }, [ended, state.tool, onAction])

  const sendLaser = useCallback((pointer: LaserPointer) => sendCoalesced('laser', { kind: 'laser', pointer }), [sendCoalesced])
  const handleStroke = useCallback(
    (slide: number, stroke: Stroke) => sendCoalesced(`stroke:${stroke.id}`, { kind: 'stroke', slide, stroke }),
    [sendCoalesced],
  )
  const handleErase = useCallback((slide: number, strokeId: string) => sendInput({ kind: 'erase', slide, strokeId }), [])

  const interactiveCursorTool = state.tool !== 'pointer' || state.laserEnabled

  return (
    <div
      ref={wheelRef}
      className="presentation-root h-full w-full select-none"
      style={{ cursor: cursorHidden && !interactiveCursorTool ? 'none' : undefined }}
    >
      {ended || !deck ? (
        <IdleScreen connected={connected} />
      ) : (
        <SlideStage
          deck={deck}
          slide={state.slide}
          state={state}
          interactive
          onSurfaceClick={handleSurfaceClick}
          onContextMenu={handleContextMenu}
          onStroke={handleStroke}
          onErase={handleErase}
          onLaser={sendLaser}
        />
      )}
    </div>
  )
}
