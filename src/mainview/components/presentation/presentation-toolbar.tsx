import {
  ChevronLeft,
  ChevronRight,
  Circle,
  Eraser,
  Highlighter,
  LayoutGrid,
  MousePointer2,
  Pen,
  Square,
  Trash2,
  Crosshair,
} from 'lucide-react'
import { useShallow } from 'zustand/react/shallow'
import { PEN_COLORS } from '../../../shared/presentation'
import { usePresentationStore } from '@/stores/presentation-store'
import { Divider, MONO_LABEL, PButton, formatSlideCounter } from './presentation-ui'

interface PresentationToolbarProps {
  /** Hide grid toggle etc. on the single-window HUD where it lives elsewhere. */
  variant?: 'presenter' | 'hud'
}

export function PresentationToolbar({ variant = 'presenter' }: PresentationToolbarProps) {
  const {
    slide, tool, penColor, laserEnabled, blackout, slideCount, hasAnnotations, gridOpen,
    next, prev, setTool, setPenColor, toggleLaserEnabled, toggleBlackout, clearAnnotations, setGridOpen,
  } = usePresentationStore(
    useShallow((s) => ({
      slide: s.state.slide,
      tool: s.state.tool,
      penColor: s.state.penColor,
      laserEnabled: s.state.laserEnabled,
      blackout: s.state.blackout,
      slideCount: s.slideDeck?.slideCount ?? 0,
      hasAnnotations: (s.state.annotations[String(s.state.slide)]?.length ?? 0) > 0,
      gridOpen: s.gridOpen,
      next: s.next,
      prev: s.prev,
      setTool: s.setTool,
      setPenColor: s.setPenColor,
      toggleLaserEnabled: s.toggleLaserEnabled,
      toggleBlackout: s.toggleBlackout,
      clearAnnotations: s.clearAnnotations,
      setGridOpen: s.setGridOpen,
    })),
  )

  const toggleTool = (target: typeof tool) => setTool(tool === target ? 'pointer' : target)
  const showColors = tool === 'pen' || tool === 'highlighter'

  return (
    <div className="flex items-center" style={{ gap: '8px', flexWrap: 'wrap' }}>
      <div className="flex items-center" style={{ gap: '4px' }}>
        <PButton onClick={prev} disabled={slide <= 0 && blackout === 'none'} title="Previous slide (←)">
          <ChevronLeft size={14} />
        </PButton>
        <span
          style={{
            ...MONO_LABEL,
            fontSize: '12px',
            color: 'var(--p-text)',
            minWidth: '72px',
            textAlign: 'center',
            fontVariantNumeric: 'tabular-nums',
          }}
        >
          {formatSlideCounter(slide, slideCount)}
        </span>
        <PButton onClick={next} disabled={slide >= slideCount - 1 && blackout === 'none'} title="Next slide (→ / Space / click)">
          <ChevronRight size={14} />
        </PButton>
      </div>

      <Divider />

      <div className="flex items-center" style={{ gap: '4px' }}>
        <PButton active={tool === 'pointer' && !laserEnabled} onClick={() => setTool('pointer')} title="Pointer: click to advance">
          <MousePointer2 size={13} />
        </PButton>
        <PButton active={laserEnabled} onClick={toggleLaserEnabled} title="Laser pointer (L)">
          <Crosshair size={13} />
        </PButton>
        <PButton active={tool === 'pen'} onClick={() => toggleTool('pen')} title="Pen (D)">
          <Pen size={13} />
        </PButton>
        <PButton active={tool === 'highlighter'} onClick={() => toggleTool('highlighter')} title="Highlighter (H)">
          <Highlighter size={13} />
        </PButton>
        <PButton active={tool === 'eraser'} onClick={() => toggleTool('eraser')} title="Eraser (E)">
          <Eraser size={13} />
        </PButton>
        <PButton onClick={() => clearAnnotations(slide)} disabled={!hasAnnotations} title="Clear annotations on this slide (C)">
          <Trash2 size={13} />
        </PButton>
      </div>

      {showColors && (
        <>
          <Divider />
          <div className="flex items-center" style={{ gap: '5px' }} role="radiogroup" aria-label="Pen colour">
            {PEN_COLORS.map((color) => {
              const selected = color.toLowerCase() === penColor.toLowerCase()
              return (
                <button
                  key={color}
                  type="button"
                  role="radio"
                  aria-checked={selected}
                  onClick={() => setPenColor(color)}
                  title={color}
                  style={{
                    width: '18px',
                    height: '18px',
                    borderRadius: '50%',
                    background: color,
                    border: selected ? '2px solid var(--p-text)' : '2px solid rgba(255,255,255,0.18)',
                    boxShadow: selected ? '0 0 0 2px var(--p-bg)' : undefined,
                    cursor: 'pointer',
                    padding: 0,
                  }}
                />
              )
            })}
          </div>
        </>
      )}

      <Divider />

      <div className="flex items-center" style={{ gap: '4px' }}>
        <PButton active={blackout === 'black'} onClick={() => toggleBlackout('black')} title="Black screen (B)">
          <Circle size={13} fill="currentColor" />
        </PButton>
        <PButton active={blackout === 'white'} onClick={() => toggleBlackout('white')} title="White screen (W)">
          <Square size={13} />
        </PButton>
        {variant === 'presenter' && (
          <PButton active={gridOpen} onClick={() => setGridOpen(!gridOpen)} title="All slides (G)">
            <LayoutGrid size={13} />
          </PButton>
        )}
      </div>
    </div>
  )
}
