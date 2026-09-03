import { useEffect, useState } from 'react'
import { Pause, Play, RotateCcw } from 'lucide-react'
import { useShallow } from 'zustand/react/shallow'
import { computeTimerElapsedMs, formatDuration, usePresentationStore } from '@/stores/presentation-store'
import { MONO_LABEL, PButton } from './presentation-ui'

function useNow(intervalMs: number): number {
  const [now, setNow] = useState(() => Date.now())
  useEffect(() => {
    const timer = setInterval(() => setNow(Date.now()), intervalMs)
    return () => clearInterval(timer)
  }, [intervalMs])
  return now
}

export function ElapsedTimer({ large = false }: { large?: boolean }) {
  const timer = usePresentationStore(
    useShallow((s) => ({
      timerRunning: s.timerRunning,
      timerStartedAt: s.timerStartedAt,
      timerAccumulatedMs: s.timerAccumulatedMs,
      toggleTimer: s.toggleTimer,
      resetTimer: s.resetTimer,
    })),
  )
  const now = useNow(250)
  const elapsed = computeTimerElapsedMs(timer, now)

  return (
    <div className="flex items-center" style={{ gap: '6px' }}>
      <span
        style={{
          fontFamily: 'var(--font-mono)',
          fontVariantNumeric: 'tabular-nums',
          fontSize: large ? '28px' : '15px',
          fontWeight: 600,
          color: timer.timerRunning ? 'var(--p-text)' : 'var(--p-text-dim)',
          letterSpacing: '0.02em',
          minWidth: large ? '96px' : '56px',
        }}
        title={timer.timerRunning ? 'Elapsed (running)' : 'Elapsed (paused)'}
      >
        {formatDuration(elapsed)}
      </span>
      <PButton compact onClick={timer.toggleTimer} title={timer.timerRunning ? 'Pause timer (T)' : 'Start timer (T)'}>
        {timer.timerRunning ? <Pause size={12} /> : <Play size={12} />}
      </PButton>
      <PButton compact onClick={timer.resetTimer} title="Reset timer (R)">
        <RotateCcw size={12} />
      </PButton>
    </div>
  )
}

export function WallClock({ large = false }: { large?: boolean }) {
  const now = useNow(1000)
  const date = new Date(now)
  const hh = String(date.getHours()).padStart(2, '0')
  const mm = String(date.getMinutes()).padStart(2, '0')
  return (
    <span
      style={{
        ...MONO_LABEL,
        fontSize: large ? '28px' : '15px',
        fontWeight: 600,
        letterSpacing: '0.02em',
        textTransform: 'none',
        fontVariantNumeric: 'tabular-nums',
        color: 'var(--p-text-dim)',
      }}
      title="Current time"
    >
      {hh}:{mm}
    </span>
  )
}
