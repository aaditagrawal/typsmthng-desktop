import { describe, expect, it } from 'vitest'
import { computeSlideGeometry, resolveNotesLayout } from './slide-deck'
import { fitSlide } from '../components/presentation/slide-stage'
import { clampSlide } from '../../shared/presentation'

const widescreen = { width: 720, height: 405 }
const dualScreen = { width: 1440, height: 405 }
const a4 = { width: 595, height: 842 }

describe('resolveNotesLayout', () => {
  it('honours explicit choices', () => {
    expect(resolveNotesLayout('right', [widescreen])).toBe('right')
    expect(resolveNotesLayout('none', [dualScreen])).toBe('none')
  })

  it('auto-detects double-width pages produced by touying / polylux notes', () => {
    expect(resolveNotesLayout('auto', [dualScreen, dualScreen])).toBe('right')
    expect(resolveNotesLayout('auto', [widescreen, widescreen])).toBe('none')
    expect(resolveNotesLayout('auto', [a4])).toBe('none')
  })

  it('never splits decks with mixed page sizes or no pages', () => {
    expect(resolveNotesLayout('auto', [dualScreen, widescreen])).toBe('none')
    expect(resolveNotesLayout('auto', [])).toBe('none')
  })
})

describe('computeSlideGeometry', () => {
  it('uses the whole page when notes are not rendered', () => {
    const [slide] = computeSlideGeometry([a4], 'none')
    expect(slide.content).toEqual({ x: 0, y: 0, width: 595, height: 842 })
    expect(slide.notes).toBeNull()
  })

  it('splits the page in half for right-hand notes', () => {
    const [slide] = computeSlideGeometry([dualScreen], 'right')
    expect(slide.content).toEqual({ x: 0, y: 0, width: 720, height: 405 })
    expect(slide.notes).toEqual({ x: 720, y: 0, width: 720, height: 405 })
  })
})

describe('fitSlide', () => {
  it('letterboxes wide containers and pillarboxes tall ones', () => {
    expect(fitSlide({ width: 1000, height: 1000 }, 16 / 9)).toEqual({ width: 1000, height: 562.5, left: 0, top: 218.75 })
    const wide = fitSlide({ width: 1000, height: 300 }, 16 / 9)
    expect(wide.height).toBe(300)
    expect(wide.width).toBeCloseTo(533.333)
    expect(wide.left).toBeCloseTo(233.333)
    expect(wide.top).toBe(0)
  })

  it('handles portrait slides and empty containers', () => {
    const portrait = fitSlide({ width: 1920, height: 1080 }, 595 / 842)
    expect(portrait.height).toBe(1080)
    expect(portrait.width).toBeCloseTo(1080 * (595 / 842))
    expect(fitSlide({ width: 0, height: 0 }, 1)).toEqual({ width: 0, height: 0, left: 0, top: 0 })
  })
})

describe('clampSlide', () => {
  it('keeps indices inside the deck', () => {
    expect(clampSlide(-3, 10)).toBe(0)
    expect(clampSlide(12, 10)).toBe(9)
    expect(clampSlide(4.7, 10)).toBe(4)
    expect(clampSlide(Number.NaN, 10)).toBe(0)
    expect(clampSlide(3, 0)).toBe(0)
  })
})
