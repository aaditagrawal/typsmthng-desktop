import type { PresentationAction } from '../../shared/presentation'

export interface KeyLike {
  key: string
  code?: string
  shiftKey?: boolean
  metaKey?: boolean
  ctrlKey?: boolean
  altKey?: boolean
}

export type PresentationKeyResult =
  | { type: 'action'; action: PresentationAction }
  | { type: 'digit'; digit: number }
  | { type: 'goto-commit' }
  | null

/**
 * Keys emitted by presentation remotes (Logitech Spotlight / R400 / R800,
 * Kensington, generic USB clickers) are plain keyboard events: Page Up/Down,
 * arrows, F5, Escape, and `.`/`b` for blanking. They all map here alongside
 * the PowerPoint / Keynote conventions presenters already know.
 */
export function resolvePresentationKey(event: KeyLike, options?: { typingGoto?: boolean }): PresentationKeyResult {
  if (event.metaKey || event.ctrlKey || event.altKey) return null
  const key = event.key

  if (/^[0-9]$/.test(key)) return { type: 'digit', digit: Number(key) }

  switch (key) {
    case 'ArrowRight':
    case 'ArrowDown':
    case 'PageDown':
    case ' ':
    case 'Spacebar':
    case 'n':
    case 'N':
    case 'j':
      return event.shiftKey && key === ' ' ? action('prev') : action('next')
    case 'Enter':
      if (options?.typingGoto) return { type: 'goto-commit' }
      return event.shiftKey ? action('prev') : action('next')
    case 'ArrowLeft':
    case 'ArrowUp':
    case 'PageUp':
    case 'Backspace':
    case 'p':
    case 'P':
    case 'k':
      return action('prev')
    case 'Home':
      return action('first')
    case 'End':
      return action('last')
    case 'b':
    case 'B':
    case '.':
      return action('toggle-black')
    case 'w':
    case 'W':
    case ',':
      return action('toggle-white')
    case 'l':
    case 'L':
      return action('toggle-laser')
    case 'd':
    case 'D':
      return action('toggle-pen')
    case 'h':
    case 'H':
      return action('toggle-highlighter')
    case 'e':
    case 'E':
      return action('toggle-eraser')
    case 'c':
    case 'C':
    case 'Delete':
      return action('clear-annotations')
    case 'g':
    case 'G':
    case 'o':
    case 'O':
      return action('toggle-grid')
    case 's':
    case 'S':
      return action('toggle-notes')
    case 't':
    case 'T':
      return action('toggle-timer')
    case 'r':
    case 'R':
      return action('reset-timer')
    case 'f':
    case 'F':
    case 'F11':
      return action('toggle-fullscreen')
    case 'F5':
      return event.shiftKey ? action('first') : action('next')
    case 'Escape':
      return action('exit')
    default:
      return null
  }
}

function action(value: PresentationAction): PresentationKeyResult {
  return { type: 'action', action: value }
}

/**
 * Mouse buttons on the slide surface. Primary click advances, secondary goes
 * back (PowerPoint), and the browser back/forward thumb buttons map to
 * prev/next for mice that expose them.
 */
export function resolvePresentationMouseButton(button: number): PresentationAction | null {
  switch (button) {
    case 0:
      return 'next'
    case 2:
    case 3:
      return 'prev'
    case 4:
      return 'next'
    default:
      return null
  }
}

/**
 * Wheel notches step slides; trackpads emit many small deltas, so the caller
 * accumulates until this threshold is crossed.
 */
export const WHEEL_STEP_THRESHOLD = 40

export function resolveWheelStep(accumulated: number): { action: PresentationAction; remainder: number } | null {
  if (accumulated >= WHEEL_STEP_THRESHOLD) return { action: 'next', remainder: 0 }
  if (accumulated <= -WHEEL_STEP_THRESHOLD) return { action: 'prev', remainder: 0 }
  return null
}

export function isEditableTarget(target: EventTarget | null): boolean {
  if (!target || typeof (target as HTMLElement).tagName !== 'string') return false
  const element = target as HTMLElement
  const tag = element.tagName.toLowerCase()
  return tag === 'input' || tag === 'textarea' || tag === 'select' || element.isContentEditable
}
