import { useCallback, useEffect, useRef, useState } from 'react'
import type { PresentationAction } from '../../../shared/presentation'
import {
  isEditableTarget,
  resolvePresentationKey,
  resolvePresentationMouseButton,
  resolveWheelStep,
} from '@/lib/presentation-input'

const GOTO_BUFFER_TIMEOUT_MS = 1800

interface PresentationInputHandlers {
  onAction: (action: PresentationAction) => void
  onGoto: (slide: number) => void
  enabled?: boolean
}

/**
 * Window-level keyboard handling for a presentation surface, including the
 * "type a number, press Enter" jump used by PowerPoint and clicker remotes.
 */
export function usePresentationKeyboard({ onAction, onGoto, enabled = true }: PresentationInputHandlers): string {
  const [buffer, setBuffer] = useState('')
  const bufferRef = useRef('')
  const timerRef = useRef<ReturnType<typeof setTimeout> | null>(null)

  const clearBuffer = useCallback(() => {
    bufferRef.current = ''
    setBuffer('')
    if (timerRef.current) {
      clearTimeout(timerRef.current)
      timerRef.current = null
    }
  }, [])

  useEffect(() => {
    if (!enabled) return

    const handleKeyDown = (event: KeyboardEvent) => {
      if (isEditableTarget(event.target)) return
      const result = resolvePresentationKey(event, { typingGoto: bufferRef.current.length > 0 })
      if (!result) return
      event.preventDefault()

      if (result.type === 'digit') {
        bufferRef.current = (bufferRef.current + String(result.digit)).slice(0, 4)
        setBuffer(bufferRef.current)
        if (timerRef.current) clearTimeout(timerRef.current)
        timerRef.current = setTimeout(clearBuffer, GOTO_BUFFER_TIMEOUT_MS)
        return
      }

      if (result.type === 'goto-commit') {
        const target = Number.parseInt(bufferRef.current, 10)
        clearBuffer()
        if (Number.isFinite(target) && target > 0) onGoto(target - 1)
        return
      }

      if (bufferRef.current) clearBuffer()
      onAction(result.action)
    }

    window.addEventListener('keydown', handleKeyDown)
    return () => {
      window.removeEventListener('keydown', handleKeyDown)
      clearBuffer()
    }
  }, [enabled, onAction, onGoto, clearBuffer])

  return buffer
}

/**
 * Thumb buttons on mice (back/forward) and wheel notches over the slide.
 * Attach `ref` to the element that should react to the wheel.
 */
export function usePresentationPointerNavigation<T extends HTMLElement>(
  onAction: (action: PresentationAction) => void,
  enabled = true,
): React.RefObject<T | null> {
  const ref = useRef<T | null>(null)
  const wheelAccumulator = useRef(0)
  const wheelResetTimer = useRef<ReturnType<typeof setTimeout> | null>(null)

  useEffect(() => {
    if (!enabled) return
    const element = ref.current

    const handleWheel = (event: WheelEvent) => {
      if (event.ctrlKey || event.metaKey) return
      // Scrollable chrome (notes drawer, menus) keeps its native wheel behaviour.
      if (isEditableTarget(event.target) || (event.target as Element | null)?.closest?.('[data-no-slide-wheel]')) return
      event.preventDefault()
      // Trackpads keep emitting inertia deltas after a flick; ignore them
      // briefly so one gesture moves exactly one slide.
      if (wheelResetTimer.current) return
      wheelAccumulator.current += event.deltaY !== 0 ? event.deltaY : event.deltaX
      const step = resolveWheelStep(wheelAccumulator.current)
      if (step) {
        wheelAccumulator.current = step.remainder
        onAction(step.action)
        wheelResetTimer.current = setTimeout(() => {
          wheelResetTimer.current = null
          wheelAccumulator.current = 0
        }, 350)
      }
    }

    const handleAuxButtons = (event: MouseEvent) => {
      if (event.button !== 3 && event.button !== 4) return
      if (isEditableTarget(event.target)) return
      event.preventDefault()
      const action = resolvePresentationMouseButton(event.button)
      if (action) onAction(action)
    }

    element?.addEventListener('wheel', handleWheel, { passive: false })
    window.addEventListener('mouseup', handleAuxButtons)
    return () => {
      element?.removeEventListener('wheel', handleWheel)
      window.removeEventListener('mouseup', handleAuxButtons)
      if (wheelResetTimer.current) clearTimeout(wheelResetTimer.current)
    }
  }, [enabled, onAction])

  return ref
}
