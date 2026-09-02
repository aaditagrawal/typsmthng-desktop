import { describe, expect, it } from 'vitest'
import {
  resolvePresentationKey,
  resolvePresentationMouseButton,
  resolveWheelStep,
  WHEEL_STEP_THRESHOLD,
} from './presentation-input'

function key(k: string, extra: Partial<Parameters<typeof resolvePresentationKey>[0]> = {}) {
  return resolvePresentationKey({ key: k, ...extra })
}

describe('resolvePresentationKey', () => {
  it('maps clicker and keyboard navigation keys', () => {
    for (const k of ['ArrowRight', 'ArrowDown', 'PageDown', ' ', 'Enter', 'n', 'F5']) {
      expect(key(k)).toEqual({ type: 'action', action: 'next' })
    }
    for (const k of ['ArrowLeft', 'ArrowUp', 'PageUp', 'Backspace', 'p']) {
      expect(key(k)).toEqual({ type: 'action', action: 'prev' })
    }
    expect(key('Home')).toEqual({ type: 'action', action: 'first' })
    expect(key('End')).toEqual({ type: 'action', action: 'last' })
  })

  it('treats shift-space, shift-enter and shift-F5 like a remote going back / to start', () => {
    expect(key(' ', { shiftKey: true })).toEqual({ type: 'action', action: 'prev' })
    expect(key('Enter', { shiftKey: true })).toEqual({ type: 'action', action: 'prev' })
    expect(key('F5', { shiftKey: true })).toEqual({ type: 'action', action: 'first' })
  })

  it('maps blanking, tools and view toggles', () => {
    expect(key('b')).toEqual({ type: 'action', action: 'toggle-black' })
    expect(key('.')).toEqual({ type: 'action', action: 'toggle-black' })
    expect(key('w')).toEqual({ type: 'action', action: 'toggle-white' })
    expect(key('l')).toEqual({ type: 'action', action: 'toggle-laser' })
    expect(key('d')).toEqual({ type: 'action', action: 'toggle-pen' })
    expect(key('h')).toEqual({ type: 'action', action: 'toggle-highlighter' })
    expect(key('e')).toEqual({ type: 'action', action: 'toggle-eraser' })
    expect(key('c')).toEqual({ type: 'action', action: 'clear-annotations' })
    expect(key('g')).toEqual({ type: 'action', action: 'toggle-grid' })
    expect(key('t')).toEqual({ type: 'action', action: 'toggle-timer' })
    expect(key('f')).toEqual({ type: 'action', action: 'toggle-fullscreen' })
    expect(key('Escape')).toEqual({ type: 'action', action: 'exit' })
  })

  it('collects digits and commits with Enter while a number is being typed', () => {
    expect(key('4')).toEqual({ type: 'digit', digit: 4 })
    expect(resolvePresentationKey({ key: 'Enter' }, { typingGoto: true })).toEqual({ type: 'goto-commit' })
  })

  it('ignores modified chords so app shortcuts keep working', () => {
    expect(key('ArrowRight', { metaKey: true })).toBeNull()
    expect(key('b', { ctrlKey: true })).toBeNull()
    expect(key('p', { altKey: true })).toBeNull()
    expect(key('x')).toBeNull()
  })
})

describe('resolvePresentationMouseButton', () => {
  it('advances on primary, goes back on secondary and thumb-back, forward on thumb-forward', () => {
    expect(resolvePresentationMouseButton(0)).toBe('next')
    expect(resolvePresentationMouseButton(2)).toBe('prev')
    expect(resolvePresentationMouseButton(3)).toBe('prev')
    expect(resolvePresentationMouseButton(4)).toBe('next')
    expect(resolvePresentationMouseButton(1)).toBeNull()
  })
})

describe('resolveWheelStep', () => {
  it('steps once the accumulated delta crosses the threshold', () => {
    expect(resolveWheelStep(WHEEL_STEP_THRESHOLD - 1)).toBeNull()
    expect(resolveWheelStep(WHEEL_STEP_THRESHOLD)).toEqual({ action: 'next', remainder: 0 })
    expect(resolveWheelStep(-WHEEL_STEP_THRESHOLD)).toEqual({ action: 'prev', remainder: 0 })
  })
})
