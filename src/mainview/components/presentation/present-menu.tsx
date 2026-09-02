import { useEffect, useRef, useState } from 'react'
import { ChevronDown, Monitor, Play, Presentation } from 'lucide-react'
import { useShallow } from 'zustand/react/shallow'
import { isMacOS } from '@/lib/platform'
import { useCompileStore } from '@/stores/compile-store'
import { usePresentationStore } from '@/stores/presentation-store'
import { describeDisplay } from './display-picker'

const MOD = isMacOS ? '⌘' : 'Ctrl'
const ALT = isMacOS ? '⌥' : 'Alt'

/**
 * Toolbar entry point: a primary "Present" action plus a menu to pick the
 * presenter-view display. Lives in the editor toolbar, so it follows the
 * editor theme rather than the presentation palette.
 */
export function PresentMenu() {
  const { start, displays, refreshDisplays } = usePresentationStore(
    useShallow((s) => ({ start: s.start, displays: s.displays, refreshDisplays: s.refreshDisplays })),
  )
  const hasSlides = useCompileStore((s) => s.svg !== null && s.pageDimensions.length > 0)
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    void refreshDisplays()
    const handle = (event: MouseEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) setOpen(false)
    }
    document.addEventListener('mousedown', handle)
    return () => document.removeEventListener('mousedown', handle)
  }, [open, refreshDisplays])

  const externalDisplays = displays.filter((display) => !display.hasMainWindow)

  return (
    <div ref={ref} style={{ position: 'relative', display: 'flex', alignItems: 'center' }}>
      <button
        className="toolbar-button"
        title={`Present (${MOD}⇧P)`}
        aria-label="Present"
        disabled={!hasSlides}
        onClick={() => void start('single')}
        style={{ borderTopRightRadius: 0, borderBottomRightRadius: 0 }}
      >
        <Play size={16} />
      </button>
      <button
        className="toolbar-button"
        title="Presentation options"
        aria-label="Presentation options"
        aria-expanded={open}
        disabled={!hasSlides}
        onClick={() => setOpen((value) => !value)}
        style={{ width: '18px', borderTopLeftRadius: 0, borderBottomLeftRadius: 0, marginLeft: '-1px' }}
      >
        <ChevronDown size={12} />
      </button>

      {open && (
        <div
          role="menu"
          style={{
            position: 'absolute',
            top: '100%',
            right: 0,
            marginTop: '6px',
            minWidth: '300px',
            background: 'var(--bg-modal)',
            border: '1px solid var(--border-strong)',
            borderRadius: '3px',
            boxShadow: '0 12px 32px rgba(0,0,0,0.35)',
            zIndex: 80,
            padding: '6px 0',
            fontFamily: 'var(--font-mono)',
            fontSize: '12px',
          }}
        >
          <MenuItem
            icon={<Play size={13} />}
            title="Present here"
            hint={`Fullscreen in this window · ${MOD}⇧P`}
            onClick={() => {
              setOpen(false)
              void start('single')
            }}
          />
          <MenuItem
            icon={<Presentation size={13} />}
            title="Presenter view"
            hint={externalDisplays.length > 0
              ? `Slides on the external display, notes here · ${MOD}${ALT}P`
              : `No external display detected · ${MOD}${ALT}P`}
            onClick={() => {
              setOpen(false)
              void start('presenter')
            }}
          />
          {externalDisplays.length > 1 && (
            <>
              <div style={{ height: '1px', background: 'var(--border-default)', margin: '4px 0' }} />
              <div
                style={{
                  padding: '6px 12px 2px',
                  fontSize: '10px',
                  letterSpacing: '0.08em',
                  textTransform: 'uppercase',
                  color: 'var(--text-tertiary)',
                }}
              >
                Presenter view on
              </div>
              {displays.map((display, index) => display.hasMainWindow ? null : (
                <MenuItem
                  key={display.id}
                  icon={<Monitor size={13} />}
                  title={describeDisplay(display, index)}
                  onClick={() => {
                    setOpen(false)
                    void start('presenter', { displayId: display.id })
                  }}
                />
              ))}
            </>
          )}
        </div>
      )}
    </div>
  )
}

function MenuItem({ icon, title, hint, onClick }: { icon: React.ReactNode; title: string; hint?: string; onClick: () => void }) {
  return (
    <button
      type="button"
      role="menuitem"
      className="present-menu-item"
      onClick={onClick}
      style={{
        display: 'flex',
        alignItems: 'flex-start',
        gap: '10px',
        width: '100%',
        padding: '8px 12px',
        border: 'none',
        background: 'transparent',
        color: 'var(--text-primary)',
        textAlign: 'left',
        cursor: 'pointer',
        fontFamily: 'var(--font-mono)',
      }}
    >
      <span style={{ marginTop: '1px', color: 'var(--text-secondary)', flexShrink: 0 }}>{icon}</span>
      <span className="flex flex-col" style={{ gap: '2px', minWidth: 0 }}>
        <span style={{ fontSize: '12px' }}>{title}</span>
        {hint && <span style={{ fontSize: '10px', color: 'var(--text-tertiary)' }}>{hint}</span>}
      </span>
    </button>
  )
}
