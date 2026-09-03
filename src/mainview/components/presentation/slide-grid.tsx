import { useEffect, useRef } from 'react'
import { X } from 'lucide-react'
import type { SlideDeck } from '@/lib/slide-deck'
import { MONO_LABEL, PButton, SectionLabel } from './presentation-ui'

interface SlideGridProps {
  deck: SlideDeck
  current: number
  annotatedSlides: Set<number>
  onSelect: (slide: number) => void
  onClose: () => void
}

export function SlideGrid({ deck, current, annotatedSlides, onSelect, onClose }: SlideGridProps) {
  const currentRef = useRef<HTMLButtonElement>(null)

  useEffect(() => {
    currentRef.current?.scrollIntoView({ block: 'center' })
    currentRef.current?.focus()
  }, [])

  return (
    <div
      role="dialog"
      aria-label="All slides"
      style={{
        position: 'absolute',
        inset: 0,
        zIndex: 40,
        background: 'rgba(8, 8, 10, 0.94)',
        backdropFilter: 'blur(12px)',
        display: 'flex',
        flexDirection: 'column',
      }}
    >
      <div
        className="flex items-center justify-between shrink-0"
        style={{ padding: '14px 20px', borderBottom: '1px solid var(--p-border)' }}
      >
        <SectionLabel style={{ color: 'var(--p-text-dim)' }}>
          All slides · {deck.slideCount}
        </SectionLabel>
        <PButton compact onClick={onClose} title="Close (Esc or G)">
          <X size={12} />
        </PButton>
      </div>
      <div
        className="overflow-auto"
        style={{
          flex: 1,
          padding: '20px',
          display: 'grid',
          gridTemplateColumns: 'repeat(auto-fill, minmax(200px, 1fr))',
          gap: '16px',
          alignContent: 'start',
        }}
      >
        {deck.slides.map((slide) => {
          const isCurrent = slide.index === current
          return (
            <button
              key={slide.index}
              ref={isCurrent ? currentRef : undefined}
              type="button"
              className="presentation-grid-item"
              data-current={isCurrent ? 'true' : undefined}
              onClick={() => onSelect(slide.index)}
              title={`Go to slide ${slide.index + 1}`}
            >
              <div style={{ aspectRatio: String(deck.aspectRatio(slide.index)), background: '#fff' }}>
                <img
                  src={deck.getUrl(slide.index) ?? undefined}
                  alt=""
                  loading="lazy"
                  draggable={false}
                  style={{ width: '100%', height: '100%', display: 'block' }}
                />
              </div>
              <div
                className="flex items-center justify-between"
                style={{ padding: '6px 8px', ...MONO_LABEL, color: isCurrent ? 'var(--accent)' : 'var(--p-text-faint)' }}
              >
                <span>{String(slide.index + 1).padStart(2, '0')}</span>
                {annotatedSlides.has(slide.index) && (
                  <span title="Has annotations" style={{ color: 'var(--p-text-dim)' }}>✎</span>
                )}
              </div>
            </button>
          )
        })}
      </div>
    </div>
  )
}
