import { useEffect, useRef, useState } from 'react'
import { ChevronDown, Monitor, MonitorOff, MonitorX } from 'lucide-react'
import { useShallow } from 'zustand/react/shallow'
import type { DisplayInfo } from '../../../shared/presentation'
import { usePresentationStore } from '@/stores/presentation-store'
import { MONO_LABEL, PButton } from './presentation-ui'

export function describeDisplay(display: DisplayInfo, index: number): string {
  const size = `${Math.round(display.bounds.width)}×${Math.round(display.bounds.height)}`
  const tags = [display.isPrimary ? 'primary' : null, display.hasMainWindow ? 'this window' : null].filter(Boolean)
  return `Display ${index + 1} · ${size}${tags.length ? ` · ${tags.join(', ')}` : ''}`
}

export function DisplayPicker() {
  const { displays, audienceOpen, audienceDisplayId, refreshDisplays, openAudience, closeAudience } = usePresentationStore(
    useShallow((s) => ({
      displays: s.displays,
      audienceOpen: s.audienceOpen,
      audienceDisplayId: s.audienceDisplayId,
      refreshDisplays: s.refreshDisplays,
      openAudience: s.openAudience,
      closeAudience: s.closeAudience,
    })),
  )
  const [open, setOpen] = useState(false)
  const ref = useRef<HTMLDivElement>(null)

  useEffect(() => {
    if (!open) return
    void refreshDisplays()
    const handleClickOutside = (event: MouseEvent) => {
      if (ref.current && !ref.current.contains(event.target as Node)) setOpen(false)
    }
    document.addEventListener('mousedown', handleClickOutside)
    return () => document.removeEventListener('mousedown', handleClickOutside)
  }, [open, refreshDisplays])

  const activeIndex = displays.findIndex((display) => display.id === audienceDisplayId)
  const label = audienceOpen
    ? `Audience · Display ${activeIndex >= 0 ? activeIndex + 1 : '?'}`
    : 'No audience window'
  const Icon = audienceOpen ? Monitor : MonitorOff

  return (
    <div ref={ref} style={{ position: 'relative' }}>
      <PButton
        onClick={() => setOpen((value) => !value)}
        title="Choose the display for the audience window"
        style={{ paddingRight: '8px' }}
        active={audienceOpen}
      >
        <Icon size={13} />
        <span style={{ ...MONO_LABEL }}>{label}</span>
        <ChevronDown size={10} />
      </PButton>

      {open && (
        <div
          style={{
            position: 'absolute',
            top: '100%',
            right: 0,
            marginTop: '6px',
            minWidth: '280px',
            background: 'var(--p-elevated)',
            border: '1px solid var(--p-border-strong)',
            borderRadius: '3px',
            boxShadow: '0 12px 32px rgba(0,0,0,0.5)',
            zIndex: 60,
            padding: '6px 0',
          }}
        >
          <div style={{ ...MONO_LABEL, color: 'var(--p-text-faint)', padding: '6px 12px 4px' }}>
            Audience display
          </div>
          {displays.length === 0 && (
            <div style={{ padding: '8px 12px', color: 'var(--p-text-faint)', fontSize: '11px' }}>
              No displays detected.
            </div>
          )}
          {displays.map((display, index) => {
            const isActive = audienceOpen && display.id === audienceDisplayId
            return (
              <button
                key={display.id}
                type="button"
                className="presentation-menu-item"
                data-active={isActive ? 'true' : undefined}
                onClick={() => {
                  setOpen(false)
                  void openAudience(display.id)
                }}
              >
                <Monitor size={13} style={{ flexShrink: 0 }} />
                <span style={{ flex: 1 }}>{describeDisplay(display, index)}</span>
                {isActive && <span style={{ ...MONO_LABEL, color: 'var(--accent)' }}>live</span>}
              </button>
            )
          })}
          {displays.length === 1 && (
            <div style={{ padding: '6px 12px 8px', color: 'var(--p-text-faint)', fontSize: '10.5px', lineHeight: 1.5 }}>
              Only one display is connected. The audience window will cover this screen; use Present here for single-screen talks.
            </div>
          )}
          <div style={{ height: '1px', background: 'var(--p-border)', margin: '4px 0' }} />
          <button
            type="button"
            className="presentation-menu-item"
            disabled={!audienceOpen}
            onClick={() => {
              setOpen(false)
              void closeAudience()
            }}
          >
            <MonitorX size={13} />
            <span>Close audience window</span>
          </button>
        </div>
      )}
    </div>
  )
}
