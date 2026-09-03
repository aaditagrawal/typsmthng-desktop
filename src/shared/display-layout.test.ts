import { describe, expect, it } from 'vitest'
import { findDisplayForFrame, insetFrame, pickDefaultAudienceDisplay } from './display-layout'

const primary = { id: 1, bounds: { x: 0, y: 0, width: 1920, height: 1080 }, isPrimary: true }
const external = { id: 2, bounds: { x: 1920, y: 0, width: 3840, height: 2160 }, isPrimary: false }
const third = { id: 3, bounds: { x: -1280, y: 0, width: 1280, height: 720 }, isPrimary: false }

describe('findDisplayForFrame', () => {
  it('returns the display containing the frame centre', () => {
    expect(findDisplayForFrame([primary, external], { x: 2000, y: 100, width: 800, height: 600 })?.id).toBe(2)
    expect(findDisplayForFrame([primary, external], { x: 100, y: 100, width: 800, height: 600 })?.id).toBe(1)
  })

  it('handles frames that straddle displays by their centre point', () => {
    expect(findDisplayForFrame([primary, external], { x: 1500, y: 0, width: 1000, height: 600 })?.id).toBe(2)
  })

  it('returns null for empty input or off-screen frames', () => {
    expect(findDisplayForFrame([], { x: 0, y: 0, width: 10, height: 10 })).toBeNull()
    expect(findDisplayForFrame([primary], null)).toBeNull()
    expect(findDisplayForFrame([primary], { x: 9000, y: 9000, width: 10, height: 10 })).toBeNull()
  })
})

describe('pickDefaultAudienceDisplay', () => {
  it('prefers an external non-primary display that does not hold the presenter', () => {
    expect(pickDefaultAudienceDisplay([primary, external], 1)?.id).toBe(2)
  })

  it('falls back to the primary display when the presenter sits on the external one', () => {
    expect(pickDefaultAudienceDisplay([primary, external], 2)?.id).toBe(1)
  })

  it('prefers non-primary displays when several are available', () => {
    expect(pickDefaultAudienceDisplay([primary, external, third], 2)?.id).toBe(3)
  })

  it('covers the only display when there is just one', () => {
    expect(pickDefaultAudienceDisplay([primary], 1)?.id).toBe(1)
    expect(pickDefaultAudienceDisplay([], 1)).toBeNull()
  })
})

describe('insetFrame', () => {
  it('keeps the frame inside the display bounds', () => {
    expect(insetFrame(external.bounds, 48)).toEqual({ x: 1968, y: 48, width: 3744, height: 2064 })
  })

  it('never collapses below a usable minimum', () => {
    expect(insetFrame({ x: 0, y: 0, width: 300, height: 100 }, 48)).toEqual({ x: 48, y: 48, width: 320, height: 200 })
  })
})
