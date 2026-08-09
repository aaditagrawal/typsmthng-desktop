import { describe, expect, it } from 'vitest'
import {
  DEFAULT_WINDOW_FRAME,
  MAX_WINDOW_COORDINATE,
  MAX_WINDOW_HEIGHT,
  MAX_WINDOW_WIDTH,
  MIN_WINDOW_COORDINATE,
  MIN_WINDOW_HEIGHT,
  MIN_WINDOW_WIDTH,
  clampWindowState,
} from './window-state'

describe('clampWindowState', () => {
  it('passes through a valid frame unchanged (aside from rounding)', () => {
    expect(
      clampWindowState({
        x: 180.4,
        y: 80.6,
        width: 1480.2,
        height: 940.8,
      }),
    ).toEqual({
      x: 180,
      y: 81,
      width: 1480,
      height: 941,
    })
  })

  it('enforces minimum width and height', () => {
    expect(clampWindowState({ width: 400, height: 200 })).toEqual({
      width: MIN_WINDOW_WIDTH,
      height: MIN_WINDOW_HEIGHT,
    })
  })

  it('falls back to defaults for non-finite sizes', () => {
    expect(
      clampWindowState({
        width: Number.NaN,
        height: Number.POSITIVE_INFINITY,
      }),
    ).toEqual({
      width: DEFAULT_WINDOW_FRAME.width,
      height: DEFAULT_WINDOW_FRAME.height,
    })
  })

  it('clamps wildly negative coordinates', () => {
    expect(
      clampWindowState({
        x: -5000,
        y: -9999,
        width: 1200,
        height: 800,
      }),
    ).toEqual({
      x: MIN_WINDOW_COORDINATE,
      y: MIN_WINDOW_COORDINATE,
      width: 1200,
      height: 800,
    })
  })

  it('omits missing coordinates', () => {
    expect(clampWindowState({ width: 1480, height: 940 })).toEqual({
      width: 1480,
      height: 940,
    })
  })

  it('replaces non-finite coordinates with defaults when present', () => {
    expect(
      clampWindowState({
        x: Number.NaN,
        y: Number.NEGATIVE_INFINITY,
        width: 1000,
        height: 700,
      }),
    ).toEqual({
      x: DEFAULT_WINDOW_FRAME.x,
      y: DEFAULT_WINDOW_FRAME.y,
      width: 1000,
      height: 700,
    })
  })

  it('enforces maximum width, height, and coordinates', () => {
    expect(
      clampWindowState({
        x: 500_000,
        y: 999_999,
        width: 1_000_000,
        height: 1_000_000,
      }),
    ).toEqual({
      x: MAX_WINDOW_COORDINATE,
      y: MAX_WINDOW_COORDINATE,
      width: MAX_WINDOW_WIDTH,
      height: MAX_WINDOW_HEIGHT,
    })
  })
})
