import { useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import type { LaserPointer, PresentationState, Stroke, StrokePoint } from '../../../shared/presentation'
import type { SlideDeck, SlideRegion } from '@/lib/slide-deck'

/** Overlay coordinate space; height derives from the slide aspect ratio. */
const OVERLAY_WIDTH = 1000
const MIN_POINT_DISTANCE = 0.0015
const ERASER_RADIUS = 0.02
const HIGHLIGHTER_WIDTH = 0.035
const PEN_WIDTH = 0.006

export interface SlideStageProps {
  deck: SlideDeck | null
  slide: number
  state: PresentationState
  region?: SlideRegion
  /** Enable drawing, erasing, and laser emission from this surface. */
  interactive?: boolean
  showAnnotations?: boolean
  showLaser?: boolean
  showBlackout?: boolean
  /** Colour behind the letterboxed slide. */
  background?: string
  /** Fired for pointer-tool clicks (not while drawing). */
  onSurfaceClick?: (event: React.MouseEvent<HTMLDivElement>) => void
  onContextMenu?: (event: React.MouseEvent<HTMLDivElement>) => void
  onStroke?: (slide: number, stroke: Stroke) => void
  onErase?: (slide: number, strokeId: string) => void
  onLaser?: (pointer: LaserPointer) => void
  className?: string
  style?: React.CSSProperties
  /** Renders a decorative frame and drop shadow around the slide. */
  framed?: boolean
  /** Force-hide the cursor (idle audience surface); tools override this. */
  cursorHidden?: boolean
  emptyLabel?: string
}

interface FitBox {
  width: number
  height: number
  left: number
  top: number
}

function useElementSize<T extends HTMLElement>(): [React.RefObject<T | null>, { width: number; height: number }] {
  const ref = useRef<T | null>(null)
  const [size, setSize] = useState({ width: 0, height: 0 })

  useLayoutEffect(() => {
    const element = ref.current
    if (!element) return
    const update = () => {
      const rect = element.getBoundingClientRect()
      setSize((prev) => (
        Math.abs(prev.width - rect.width) < 0.5 && Math.abs(prev.height - rect.height) < 0.5
          ? prev
          : { width: rect.width, height: rect.height }
      ))
    }
    update()
    const observer = new ResizeObserver(update)
    observer.observe(element)
    return () => observer.disconnect()
  }, [])

  return [ref, size]
}

export function fitSlide(container: { width: number; height: number }, aspect: number): FitBox {
  if (container.width <= 0 || container.height <= 0) return { width: 0, height: 0, left: 0, top: 0 }
  const containerAspect = container.width / container.height
  if (containerAspect > aspect) {
    const height = container.height
    const width = height * aspect
    return { width, height, left: (container.width - width) / 2, top: 0 }
  }
  const width = container.width
  const height = width / aspect
  return { width, height, left: 0, top: (container.height - height) / 2 }
}

function strokePath(stroke: Stroke, overlayHeight: number): string {
  const points = stroke.points
  if (points.length === 0) return ''
  const toX = (p: StrokePoint) => (p.x * OVERLAY_WIDTH).toFixed(2)
  const toY = (p: StrokePoint) => (p.y * overlayHeight).toFixed(2)
  if (points.length === 1) {
    // Dots still need a visible mark.
    return `M ${toX(points[0])} ${toY(points[0])} l 0.01 0`
  }
  let d = `M ${toX(points[0])} ${toY(points[0])}`
  for (let i = 1; i < points.length; i++) {
    const prev = points[i - 1]
    const curr = points[i]
    const midX = ((prev.x + curr.x) / 2) * OVERLAY_WIDTH
    const midY = ((prev.y + curr.y) / 2) * overlayHeight
    d += ` Q ${toX(prev)} ${toY(prev)} ${midX.toFixed(2)} ${midY.toFixed(2)}`
  }
  const last = points[points.length - 1]
  d += ` L ${toX(last)} ${toY(last)}`
  return d
}

function hitStroke(strokes: Stroke[], point: StrokePoint, aspect: number): Stroke | null {
  for (let i = strokes.length - 1; i >= 0; i--) {
    const stroke = strokes[i]
    const radius = Math.max(ERASER_RADIUS, stroke.width)
    for (const p of stroke.points) {
      const dx = p.x - point.x
      const dy = (p.y - point.y) / aspect
      if (dx * dx + dy * dy <= radius * radius) return stroke
    }
  }
  return null
}

function newStrokeId(): string {
  return `${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`
}

function StrokeLayer({
  strokes,
  overlayHeight,
  blend = 'normal',
  opacity = 1,
}: {
  strokes: Stroke[]
  overlayHeight: number
  blend?: 'normal' | 'multiply'
  opacity?: number
}) {
  return (
    <svg
      viewBox={`0 0 ${OVERLAY_WIDTH} ${overlayHeight}`}
      preserveAspectRatio="none"
      style={{
        position: 'absolute',
        inset: 0,
        width: '100%',
        height: '100%',
        pointerEvents: 'none',
        mixBlendMode: blend,
        opacity,
      }}
    >
      {strokes.map((stroke) => (
        <path
          key={stroke.id}
          d={strokePath(stroke, overlayHeight)}
          fill="none"
          stroke={stroke.color}
          strokeWidth={stroke.width * OVERLAY_WIDTH}
          strokeLinecap="round"
          strokeLinejoin="round"
        />
      ))}
    </svg>
  )
}

function SlideImage({ src, fadeKey }: { src: string | null; fadeKey: string }) {
  const [layers, setLayers] = useState<Array<{ key: string; src: string }>>([])

  useEffect(() => {
    if (!src) {
      setLayers([])
      return
    }
    setLayers((prev) => {
      if (prev.length > 0 && prev[prev.length - 1].key === fadeKey) return prev
      // Keep the previous slide underneath so the new one can fade in over it.
      return [...prev.slice(-1), { key: fadeKey, src }]
    })
  }, [src, fadeKey])

  useEffect(() => {
    if (layers.length <= 1) return
    const timer = setTimeout(() => setLayers((prev) => prev.slice(-1)), 220)
    return () => clearTimeout(timer)
  }, [layers])

  return (
    <>
      {layers.map((layer, index) => (
        <img
          key={layer.key}
          src={layer.src}
          alt=""
          draggable={false}
          className={index === layers.length - 1 && layers.length > 1 ? 'presentation-slide-enter' : undefined}
          style={{
            position: 'absolute',
            inset: 0,
            width: '100%',
            height: '100%',
            display: 'block',
            userSelect: 'none',
            pointerEvents: 'none',
          }}
        />
      ))}
    </>
  )
}

export function SlideStage({
  deck,
  slide,
  state,
  region = 'content',
  interactive = false,
  showAnnotations = true,
  showLaser = true,
  showBlackout = true,
  background = '#000',
  onSurfaceClick,
  onContextMenu,
  onStroke,
  onErase,
  onLaser,
  className,
  style,
  framed = false,
  cursorHidden = false,
  emptyLabel = 'Waiting for slides',
}: SlideStageProps) {
  const [containerRef, containerSize] = useElementSize<HTMLDivElement>()
  const aspect = deck ? deck.aspectRatio(slide, region) : 16 / 9
  const box = useMemo(() => fitSlide(containerSize, aspect), [containerSize, aspect])
  const overlayHeight = OVERLAY_WIDTH / aspect
  const src = deck ? deck.getUrl(slide, region) : null
  const strokes = showAnnotations ? (state.annotations[String(slide)] ?? []) : []
  const drawingRef = useRef<{ pointerId: number; stroke: Stroke } | null>(null)
  const erasingRef = useRef<number | null>(null)
  const suppressClickRef = useRef(false)

  const tool = state.tool
  const drawingTool = interactive && (tool === 'pen' || tool === 'highlighter')
  const erasingTool = interactive && tool === 'eraser'
  const laserActive = interactive && state.laserEnabled

  const toNormalized = useCallback((event: { clientX: number; clientY: number }, element: HTMLElement): StrokePoint => {
    const rect = element.getBoundingClientRect()
    return {
      x: rect.width > 0 ? (event.clientX - rect.left) / rect.width : 0,
      y: rect.height > 0 ? (event.clientY - rect.top) / rect.height : 0,
    }
  }, [])

  const handlePointerDown = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
    if (!interactive || !deck) return
    if (event.button !== 0) return
    const point = toNormalized(event, event.currentTarget)

    if (drawingTool) {
      event.preventDefault()
      event.currentTarget.setPointerCapture(event.pointerId)
      const stroke: Stroke = {
        id: newStrokeId(),
        tool: tool === 'highlighter' ? 'highlighter' : 'pen',
        color: state.penColor,
        width: tool === 'highlighter' ? HIGHLIGHTER_WIDTH : PEN_WIDTH,
        points: [point],
      }
      drawingRef.current = { pointerId: event.pointerId, stroke }
      suppressClickRef.current = true
      onStroke?.(slide, stroke)
      return
    }

    if (erasingTool) {
      event.preventDefault()
      event.currentTarget.setPointerCapture(event.pointerId)
      erasingRef.current = event.pointerId
      suppressClickRef.current = true
      const hit = hitStroke(strokes, point, aspect)
      if (hit) onErase?.(slide, hit.id)
    }
  }, [interactive, deck, drawingTool, erasingTool, tool, state.penColor, toNormalized, onStroke, onErase, slide, strokes, aspect])

  const handlePointerMove = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
    if (!interactive) return
    const point = toNormalized(event, event.currentTarget)

    const drawing = drawingRef.current
    if (drawing && drawing.pointerId === event.pointerId) {
      const last = drawing.stroke.points[drawing.stroke.points.length - 1]
      const dx = point.x - last.x
      const dy = point.y - last.y
      if (dx * dx + dy * dy < MIN_POINT_DISTANCE * MIN_POINT_DISTANCE) return
      const next: Stroke = { ...drawing.stroke, points: [...drawing.stroke.points, point] }
      drawingRef.current = { pointerId: event.pointerId, stroke: next }
      onStroke?.(slide, next)
      return
    }

    if (erasingRef.current === event.pointerId) {
      const hit = hitStroke(strokes, point, aspect)
      if (hit) onErase?.(slide, hit.id)
      return
    }

    if (laserActive && onLaser) {
      const inside = point.x >= 0 && point.x <= 1 && point.y >= 0 && point.y <= 1
      onLaser({ x: point.x, y: point.y, visible: inside })
    }
  }, [interactive, toNormalized, onStroke, onErase, onLaser, slide, strokes, aspect, laserActive])

  const endPointer = useCallback((event: React.PointerEvent<HTMLDivElement>) => {
    if (drawingRef.current?.pointerId === event.pointerId) {
      drawingRef.current = null
    }
    if (erasingRef.current === event.pointerId) {
      erasingRef.current = null
    }
    if (event.currentTarget.hasPointerCapture(event.pointerId)) {
      event.currentTarget.releasePointerCapture(event.pointerId)
    }
  }, [])

  const handlePointerLeave = useCallback(() => {
    if (laserActive && onLaser && state.laser.visible) {
      onLaser({ ...state.laser, visible: false })
    }
  }, [laserActive, onLaser, state.laser])

  const handleClick = useCallback((event: React.MouseEvent<HTMLDivElement>) => {
    if (suppressClickRef.current) {
      suppressClickRef.current = false
      event.preventDefault()
      event.stopPropagation()
      return
    }
    onSurfaceClick?.(event)
  }, [onSurfaceClick])

  const cursor = drawingTool
    ? 'crosshair'
    : erasingTool
      ? 'cell'
      : laserActive || cursorHidden
        ? 'none'
        : interactive && onSurfaceClick
          ? 'pointer'
          : 'default'
  const penStrokes = strokes.filter((stroke) => stroke.tool === 'pen')
  const highlightStrokes = strokes.filter((stroke) => stroke.tool === 'highlighter')

  const laserVisible = showLaser && state.laser.visible && state.laserEnabled
  const laserSize = Math.max(12, box.width * 0.018)

  return (
    <div
      ref={containerRef}
      className={className}
      onClick={handleClick}
      onContextMenu={onContextMenu}
      style={{
        position: 'relative',
        width: '100%',
        height: '100%',
        overflow: 'hidden',
        background,
        cursor: cursorHidden
          ? 'none'
          : interactive && onSurfaceClick && !drawingTool && !erasingTool && !laserActive
            ? 'pointer'
            : undefined,
        ...style,
      }}
    >
      {box.width > 0 && (
        <div
          role={interactive ? 'button' : undefined}
          onPointerDown={handlePointerDown}
          onPointerMove={handlePointerMove}
          onPointerUp={endPointer}
          onPointerCancel={endPointer}
          onLostPointerCapture={endPointer}
          onPointerLeave={handlePointerLeave}
          style={{
            position: 'absolute',
            left: `${box.left}px`,
            top: `${box.top}px`,
            width: `${box.width}px`,
            height: `${box.height}px`,
            background: deck ? '#fff' : 'transparent',
            boxShadow: framed ? '0 12px 40px rgba(0,0,0,0.45), 0 0 0 1px rgba(255,255,255,0.08)' : undefined,
            cursor,
            touchAction: 'none',
            userSelect: 'none',
          }}
        >
          {deck ? (
            <SlideImage src={src} fadeKey={`${deck.revision}:${slide}:${region}`} />
          ) : (
            <div
              style={{
                position: 'absolute',
                inset: 0,
                display: 'flex',
                alignItems: 'center',
                justifyContent: 'center',
                color: 'rgba(255,255,255,0.35)',
                fontFamily: 'var(--font-mono)',
                fontSize: '12px',
                letterSpacing: '0.08em',
                textTransform: 'uppercase',
                border: '1px dashed rgba(255,255,255,0.15)',
              }}
            >
              {emptyLabel}
            </div>
          )}

          {/* Highlighter strokes multiply against the slide; pens draw on top. */}
          {highlightStrokes.length > 0 && (
            <StrokeLayer strokes={highlightStrokes} overlayHeight={overlayHeight} blend="multiply" opacity={0.45} />
          )}
          {penStrokes.length > 0 && (
            <StrokeLayer strokes={penStrokes} overlayHeight={overlayHeight} />
          )}

          {laserVisible && (
            <div
              aria-hidden
              style={{
                position: 'absolute',
                left: `${state.laser.x * 100}%`,
                top: `${state.laser.y * 100}%`,
                width: `${laserSize}px`,
                height: `${laserSize}px`,
                marginLeft: `${-laserSize / 2}px`,
                marginTop: `${-laserSize / 2}px`,
                borderRadius: '50%',
                background: 'radial-gradient(circle, rgba(255,60,40,1) 0%, rgba(255,60,40,0.85) 45%, rgba(255,60,40,0) 100%)',
                boxShadow: '0 0 12px 4px rgba(255,60,40,0.55)',
                pointerEvents: 'none',
              }}
            />
          )}
        </div>
      )}

      {showBlackout && state.blackout !== 'none' && (
        <div
          aria-hidden
          style={{
            position: 'absolute',
            inset: 0,
            background: state.blackout === 'black' ? '#000' : '#fff',
            pointerEvents: 'none',
          }}
        />
      )}
    </div>
  )
}
