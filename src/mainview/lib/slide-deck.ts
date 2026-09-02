import type { NotesLayout, PresentationDeck, PresentationPageSize } from '../../shared/presentation'

export type ResolvedNotesLayout = 'right' | 'none'
export type SlideRegion = 'content' | 'notes'

export interface SlideBox {
  x: number
  y: number
  width: number
  height: number
}

export interface SlideGeometry {
  index: number
  /** Full page size in pt. */
  page: PresentationPageSize
  /** The part of the page the audience sees. */
  content: SlideBox
  /** The part of the page holding rendered speaker notes, if any. */
  notes: SlideBox | null
}

const SVG_NS = 'http://www.w3.org/2000/svg'
const SVG_SLICE_CACHE_LIMIT = 16
const BLOB_URL_CACHE_LIMIT = 96
/** Two side-by-side 4:3 pages are 2.67:1; ultrawide 21:9 slides are 2.33:1. */
const SPLIT_NOTES_MIN_ASPECT = 2.6

export function resolveNotesLayout(layout: NotesLayout, pages: PresentationPageSize[]): ResolvedNotesLayout {
  if (layout === 'right') return 'right'
  if (layout === 'none' || pages.length === 0) return 'none'

  const first = pages[0]
  if (!first || first.height <= 0) return 'none'
  const uniform = pages.every((page) =>
    Math.abs(page.width - first.width) < 0.5 && Math.abs(page.height - first.height) < 0.5)
  if (!uniform) return 'none'
  return first.width / first.height >= SPLIT_NOTES_MIN_ASPECT ? 'right' : 'none'
}

export function computeSlideGeometry(
  pages: PresentationPageSize[],
  layout: ResolvedNotesLayout,
): SlideGeometry[] {
  return pages.map((page, index) => {
    if (layout === 'right') {
      const half = page.width / 2
      return {
        index,
        page,
        content: { x: 0, y: 0, width: half, height: page.height },
        notes: { x: half, y: 0, width: half, height: page.height },
      }
    }
    return {
      index,
      page,
      content: { x: 0, y: 0, width: page.width, height: page.height },
      notes: null,
    }
  })
}

interface ParsedDocument {
  sharedMarkup: string
  pageMarkup: string[]
  rootAttributes: string
}

function formatNumber(value: number): string {
  return Number.isInteger(value) ? String(value) : value.toFixed(3)
}

function serializeAttributes(element: Element, skip: Set<string>): string {
  const parts: string[] = []
  for (const attr of Array.from(element.attributes)) {
    if (skip.has(attr.name)) continue
    parts.push(`${attr.name}="${attr.value.replace(/"/g, '&quot;')}"`)
  }
  return parts.join(' ')
}

/**
 * Split the combined typst SVG (pages stacked vertically inside
 * `<g class="typst-page" transform="translate(0, y)">`) into reusable pieces.
 * Falls back to treating the whole document as a single page when the
 * structure is unexpected so presenting never hard-fails.
 */
function parseDocument(svg: string, pageCount: number): ParsedDocument {
  const doc = new DOMParser().parseFromString(svg, 'image/svg+xml')
  const root = doc.documentElement
  const serializer = new XMLSerializer()

  if (!root || root.nodeName.toLowerCase() !== 'svg' || doc.querySelector('parsererror')) {
    return { sharedMarkup: '', pageMarkup: [svg], rootAttributes: '' }
  }

  const pageGroups: Element[] = []
  const shared: string[] = []
  for (const child of Array.from(root.children)) {
    const tag = child.nodeName.toLowerCase()
    if (tag === 'script') continue
    if (tag === 'g' && child.classList.contains('typst-page')) {
      pageGroups.push(child)
      continue
    }
    shared.push(serializer.serializeToString(child))
  }

  if (pageGroups.length === 0 || (pageCount > 0 && pageGroups.length !== pageCount)) {
    // Unknown structure: keep everything and let the viewBox do the cropping.
    const inner = Array.from(root.children)
      .filter((child) => child.nodeName.toLowerCase() !== 'script')
      .map((child) => serializer.serializeToString(child))
      .join('')
    return {
      sharedMarkup: '',
      pageMarkup: [inner],
      rootAttributes: serializeAttributes(root, new Set(['viewBox', 'width', 'height', 'style', 'class'])),
    }
  }

  const pageMarkup = pageGroups.map((group) => {
    group.removeAttribute('transform')
    return serializer.serializeToString(group)
  })

  return {
    sharedMarkup: shared.join(''),
    pageMarkup,
    rootAttributes: serializeAttributes(root, new Set(['viewBox', 'width', 'height', 'style', 'class'])),
  }
}

class LruCache<K, V> {
  private map = new Map<K, V>()

  constructor(private readonly limit: number, private readonly onEvict?: (value: V) => void) {}

  get(key: K): V | undefined {
    const value = this.map.get(key)
    if (value === undefined) return undefined
    this.map.delete(key)
    this.map.set(key, value)
    return value
  }

  set(key: K, value: V): void {
    if (this.map.has(key)) this.map.delete(key)
    this.map.set(key, value)
    if (this.map.size > this.limit) {
      const oldest = this.map.keys().next().value as K
      const evicted = this.map.get(oldest)
      this.map.delete(oldest)
      if (evicted !== undefined) this.onEvict?.(evicted)
    }
  }

  clear(): void {
    if (this.onEvict) {
      for (const value of this.map.values()) this.onEvict(value)
    }
    this.map.clear()
  }
}

/**
 * A compiled deck sliced into slides. Slide SVG strings and blob URLs are
 * produced lazily and cached, so a 100-slide deck does not materialise 100
 * copies of the shared glyph defs up front.
 */
export class SlideDeck {
  readonly revision: number
  readonly title: string
  readonly layout: ResolvedNotesLayout
  readonly slides: SlideGeometry[]
  private parsed: ParsedDocument | null = null
  private readonly svgCache = new LruCache<string, string>(SVG_SLICE_CACHE_LIMIT)
  private readonly urlCache = new LruCache<string, string>(BLOB_URL_CACHE_LIMIT, (url) => URL.revokeObjectURL(url))
  private disposed = false

  constructor(private readonly deck: PresentationDeck) {
    this.revision = deck.revision
    this.title = deck.title
    this.layout = resolveNotesLayout(deck.notesLayout, deck.pages)
    this.slides = computeSlideGeometry(deck.pages, this.layout)
  }

  get slideCount(): number {
    return this.slides.length
  }

  get hasRenderedNotes(): boolean {
    return this.layout === 'right'
  }

  private ensureParsed(): ParsedDocument {
    if (!this.parsed) {
      this.parsed = parseDocument(this.deck.svg, this.deck.pages.length)
    }
    return this.parsed
  }

  private boxFor(index: number, region: SlideRegion): SlideBox | null {
    const slide = this.slides[index]
    if (!slide) return null
    return region === 'notes' ? slide.notes : slide.content
  }

  getSvg(index: number, region: SlideRegion = 'content'): string | null {
    const box = this.boxFor(index, region)
    if (!box) return null

    const key = `${index}:${region}`
    const cached = this.svgCache.get(key)
    if (cached) return cached

    const parsed = this.ensureParsed()
    const body = parsed.pageMarkup[Math.min(index, parsed.pageMarkup.length - 1)] ?? ''
    // When the structure was not recognised the fallback holds the whole
    // document; offset the viewBox to the page's vertical position instead.
    const fallbackOffset = parsed.pageMarkup.length === 1 && this.deck.pages.length > 1
      ? this.deck.pages.slice(0, index).reduce((sum, page) => sum + page.height, 0)
      : 0
    const viewBox = `${formatNumber(box.x)} ${formatNumber(box.y + fallbackOffset)} ${formatNumber(box.width)} ${formatNumber(box.height)}`
    const attrs = parsed.rootAttributes || `xmlns="${SVG_NS}" xmlns:xlink="http://www.w3.org/1999/xlink"`
    const svg = `<svg ${attrs} class="typst-doc typst-slide" viewBox="${viewBox}" width="${formatNumber(box.width)}" height="${formatNumber(box.height)}">${parsed.sharedMarkup}${body}</svg>`

    this.svgCache.set(key, svg)
    return svg
  }

  /** Blob URL suitable for `<img src>`; stays valid until `dispose()`. */
  getUrl(index: number, region: SlideRegion = 'content'): string | null {
    if (this.disposed) return null
    const key = `${index}:${region}`
    const cached = this.urlCache.get(key)
    if (cached) return cached

    const svg = this.getSvg(index, region)
    if (!svg) return null
    const url = URL.createObjectURL(new Blob([svg], { type: 'image/svg+xml' }))
    this.urlCache.set(key, url)
    return url
  }

  aspectRatio(index: number, region: SlideRegion = 'content'): number {
    const box = this.boxFor(index, region)
    if (!box || box.height <= 0) return 16 / 9
    return box.width / box.height
  }

  dispose(): void {
    this.disposed = true
    this.urlCache.clear()
    this.svgCache.clear()
    this.parsed = null
  }
}
