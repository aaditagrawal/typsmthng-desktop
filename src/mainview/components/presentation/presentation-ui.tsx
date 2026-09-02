import type { ReactNode } from 'react'

export const MONO_LABEL: React.CSSProperties = {
  fontFamily: 'var(--font-mono)',
  fontSize: '10px',
  letterSpacing: '0.08em',
  textTransform: 'uppercase',
  fontWeight: 600,
}

export function SectionLabel({ children, style }: { children: ReactNode; style?: React.CSSProperties }) {
  return (
    <div style={{ ...MONO_LABEL, color: 'var(--p-text-faint)', ...style }}>
      {children}
    </div>
  )
}

interface PButtonProps extends React.ButtonHTMLAttributes<HTMLButtonElement> {
  active?: boolean
  tone?: 'default' | 'accent' | 'danger'
  compact?: boolean
  label?: string
}

export function PButton({ active, tone = 'default', compact, label, children, style, className, ...props }: PButtonProps) {
  return (
    <button
      type="button"
      {...props}
      data-active={active ? 'true' : undefined}
      data-tone={tone}
      className={`presentation-btn${className ? ` ${className}` : ''}`}
      style={{
        height: compact ? '26px' : '30px',
        padding: children && label ? '0 10px 0 8px' : label ? '0 10px' : '0 7px',
        gap: '6px',
        ...style,
      }}
    >
      {children}
      {label && <span style={{ ...MONO_LABEL, fontWeight: 600 }}>{label}</span>}
    </button>
  )
}

export function Divider({ vertical = true }: { vertical?: boolean }) {
  return (
    <div
      aria-hidden
      style={vertical
        ? { width: '1px', height: '18px', background: 'var(--p-border)', flexShrink: 0 }
        : { height: '1px', width: '100%', background: 'var(--p-border)', flexShrink: 0 }}
    />
  )
}

export function Kbd({ children }: { children: ReactNode }) {
  return (
    <kbd
      style={{
        display: 'inline-block',
        minWidth: '18px',
        padding: '1px 5px',
        borderRadius: '2px',
        border: '1px solid var(--p-border-strong)',
        background: 'var(--p-elevated)',
        color: 'var(--p-text-dim)',
        fontFamily: 'var(--font-mono)',
        fontSize: '10px',
        textAlign: 'center',
        lineHeight: '14px',
      }}
    >
      {children}
    </kbd>
  )
}

export function formatSlideCounter(index: number, count: number): string {
  const pad = (value: number) => String(value).padStart(2, '0')
  return `${pad(Math.min(index + 1, Math.max(count, 1)))} / ${pad(Math.max(count, 1))}`
}
