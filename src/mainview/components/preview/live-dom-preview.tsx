import { useEffect, useRef, useState, type ReactNode } from 'react'
import { AlertCircle, Eye, Loader2 } from 'lucide-react'
import { useShallow } from 'zustand/react/shallow'
import { mountLivePreview, type LivePreviewController } from '@/lib/compiler'
import { useCompileStore } from '@/stores/compile-store'

const BRUTALIST_FONT = {
  fontFamily: 'var(--font-mono)',
  fontSize: '11px',
  letterSpacing: '0.05em',
  textTransform: 'uppercase' as const,
}

function EmptyState({
  icon,
  title,
  detail,
}: {
  icon: ReactNode
  title: string
  detail?: string
}) {
  return (
    <div
      className="h-full flex items-center justify-center"
      style={{ minHeight: '220px' }}
    >
      <div className="flex flex-col items-center gap-3 max-w-sm text-center" style={{ color: 'var(--text-tertiary)' }}>
        {icon}
        <span style={BRUTALIST_FONT}>{title}</span>
        {detail && (
          <span style={{ ...BRUTALIST_FONT, fontSize: '10px', color: 'var(--text-quaternary)' }}>
            {detail}
          </span>
        )}
      </div>
    </div>
  )
}

export function LiveDomPreview() {
  const { status, vectorData, diagnostics } = useCompileStore(
    useShallow((s) => ({ status: s.status, vectorData: s.vectorData, diagnostics: s.diagnostics }))
  )
  const hostRef = useRef<HTMLDivElement>(null)
  const controllerRef = useRef<LivePreviewController | null>(null)
  const [mountError, setMountError] = useState<string | null>(null)

  useEffect(() => {
    let cancelled = false

    const disposeController = () => {
      controllerRef.current?.dispose()
      controllerRef.current = null
    }

    if (!vectorData || !hostRef.current) {
      disposeController()
      return
    }

    hostRef.current.innerHTML = ''
    setMountError(null)
    disposeController()

    void mountLivePreview(vectorData, hostRef.current, { pixelPerPt: 2 })
      .then((controller) => {
        if (cancelled) {
          controller.dispose()
          return
        }
        controllerRef.current = controller
        controller.refresh()
      })
      .catch((err) => {
        if (cancelled) return
        disposeController()
        setMountError(err instanceof Error ? err.message : 'Failed to initialize live preview')
      })

    return () => {
      cancelled = true
      disposeController()
    }
  }, [vectorData])

  useEffect(() => {
    const host = hostRef.current
    const controller = controllerRef.current
    if (!host || !controller) return

    const refresh = () => controller.refresh()
    const resizeObserver = new ResizeObserver(() => refresh())
    resizeObserver.observe(host)
    host.addEventListener('scroll', refresh, { passive: true })
    window.addEventListener('resize', refresh)

    return () => {
      resizeObserver.disconnect()
      host.removeEventListener('scroll', refresh)
      window.removeEventListener('resize', refresh)
    }
  }, [vectorData, mountError])

  const errorCount = diagnostics.filter((d) => d.severity === 'error').length

  return (
    <div className="h-full flex flex-col">
      {status === 'compiling' && !vectorData && (
        <EmptyState
          icon={<Loader2 size={20} className="animate-spin" style={{ color: 'var(--accent)' }} />}
          title="Preparing live preview..."
          detail="The desktop liveview mounts the Typst DOM renderer directly after compile."
        />
      )}

      {!vectorData && status === 'error' && (
        <EmptyState
          icon={<AlertCircle size={20} style={{ color: 'var(--status-error)' }} />}
          title={`Compilation failed${errorCount > 0 ? ` (${errorCount})` : ''}`}
          detail="Fix the source errors to mount the desktop liveview."
        />
      )}

      {!vectorData && (status === 'idle' || status === 'success') && (
        <EmptyState
          icon={<Eye size={20} />}
          title="No live preview available"
          detail="Compile a Typst document to mount the desktop liveview surface."
        />
      )}

      {vectorData && mountError && (
        <EmptyState
          icon={<AlertCircle size={20} style={{ color: 'var(--status-error)' }} />}
          title="Live preview could not start"
          detail={mountError}
        />
      )}

      <div
        ref={hostRef}
        className="flex-1 overflow-auto"
        style={{
          display: vectorData && !mountError ? 'block' : 'none',
          background: 'var(--bg-app)',
          padding: '24px 18px 40px',
        }}
      />
    </div>
  )
}
