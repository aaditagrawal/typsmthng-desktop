import { useCallback, useRef, useState } from 'react'
import { Panel, Group, Separator } from 'react-resizable-panels'
import { Toolbar } from '@/components/layout/toolbar'
import { StatusBar } from '@/components/layout/status-bar'
import { TypstEditor } from '@/components/editor/typst-editor'
import { PreviewPanel } from '@/components/preview/preview-panel'
import { FileTree } from '@/components/sidebar/file-tree'
import { SettingsModal } from '@/components/settings/settings-modal'
import { CommandSearch } from '@/components/search/command-search'
import { ImagePreviewModal } from '@/components/preview/image-preview-modal'
import { ErrorBoundary } from '@/components/layout/error-boundary'
import { useProjectStore } from '@/stores/project-store'
import { useEditorStore } from '@/stores/editor-store'

function ConflictBanner() {
  const conflict = useProjectStore((s) => s.activeConflict)
  const reloadConflictFile = useProjectStore((s) => s.reloadConflictFile)
  const dismissConflict = useProjectStore((s) => s.dismissConflict)
  const saveStatus = useEditorStore((s) => s.saveStatus)

  if (!conflict) return null

  return (
    <div
      className="flex items-center justify-between gap-4 px-4 py-3 shrink-0"
      style={{
        background: 'color-mix(in srgb, var(--status-warning) 10%, var(--bg-elevated))',
        border: '1px solid color-mix(in srgb, var(--status-warning) 35%, var(--border-default))',
        borderRadius: '2px',
      }}
    >
      <div style={{ color: 'var(--text-primary)', fontSize: '12px', fontFamily: 'var(--font-mono)' }}>
        External change detected in <span style={{ color: 'var(--accent)' }}>{conflict.path}</span>. Reload from disk or keep the editor buffer.
      </div>
      <div className="flex items-center gap-2">
        <button
          onClick={() => void reloadConflictFile()}
          title="Reload from disk"
          style={{
            height: '30px',
            padding: '0 12px',
            borderRadius: '2px',
            border: '1px solid color-mix(in srgb, var(--accent) 42%, transparent)',
            background: 'color-mix(in srgb, var(--accent) 14%, transparent)',
            color: 'var(--text-primary)',
            fontSize: '12px',
            cursor: 'pointer',
          }}
        >
          Reload
        </button>
        <button
          onClick={dismissConflict}
          title="Keep local buffer"
          style={{
            height: '30px',
            padding: '0 12px',
            borderRadius: '2px',
            border: '1px solid color-mix(in srgb, var(--border-default) 80%, transparent)',
            background: 'transparent',
            color: 'var(--text-primary)',
            fontSize: '12px',
            cursor: 'pointer',
          }}
        >
          Keep
        </button>
        <span style={{ color: 'var(--text-tertiary)', fontSize: '11px', fontFamily: 'var(--font-mono)' }}>
          {saveStatus === 'unsaved' ? 'dirty buffer' : 'clean buffer'}
        </span>
      </div>
    </div>
  )
}

function SidebarResizeHandle({ onDrag }: { onDrag: (delta: number) => void }) {
  const dragging = useRef(false)
  const lastX = useRef(0)

  const onMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault()
    dragging.current = true
    lastX.current = e.clientX

    const onMouseMove = (ev: MouseEvent) => {
      if (!dragging.current) return
      const delta = ev.clientX - lastX.current
      lastX.current = ev.clientX
      onDrag(delta)
    }
    const onMouseUp = () => {
      dragging.current = false
      document.removeEventListener('mousemove', onMouseMove)
      document.removeEventListener('mouseup', onMouseUp)
    }
    document.addEventListener('mousemove', onMouseMove)
    document.addEventListener('mouseup', onMouseUp)
  }, [onDrag])

  return (
    <div
      onMouseDown={onMouseDown}
      style={{
        width: '1px',
        minWidth: '1px',
        background: 'var(--border-default)',
        cursor: 'col-resize',
        position: 'relative',
        flexShrink: 0,
      }}
    >
      <div style={{ position: 'absolute', inset: '0 -4px', zIndex: 1 }} />
    </div>
  )
}

export default function WorkspaceShell() {
  const sidebarOpen = useProjectStore((s) => s.sidebarOpen)
  const [sidebarWidth, setSidebarWidth] = useState(240)

  const handleSidebarDrag = useCallback((delta: number) => {
    setSidebarWidth((w) => Math.min(400, Math.max(160, w + delta)))
  }, [])

  return (
    <ErrorBoundary fallbackMessage="The application encountered an unexpected error.">
      <div className="flex flex-col h-full w-full" style={{ background: 'var(--bg-app)' }}>
        <Toolbar />
        <ConflictBanner />
        <div className="flex flex-1 min-h-0">
          {sidebarOpen && (
            <>
              <div
                className="shrink-0 overflow-hidden"
                style={{
                  width: `${sidebarWidth}px`,
                  background: 'var(--bg-surface)',
                }}
              >
                <FileTree />
              </div>
              <SidebarResizeHandle onDrag={handleSidebarDrag} />
            </>
          )}
          <Group orientation="horizontal" className="flex-1">
            <Panel defaultSize={50} minSize={25}>
              <ErrorBoundary fallbackMessage="Editor crashed.">
                <TypstEditor />
              </ErrorBoundary>
            </Panel>
            <Separator />
            <Panel defaultSize={50} minSize={25}>
              <ErrorBoundary fallbackMessage="Preview crashed.">
                <PreviewPanel />
              </ErrorBoundary>
            </Panel>
          </Group>
        </div>
        <StatusBar />
        <SettingsModal />
        <CommandSearch />
        <ImagePreviewModal />
      </div>
    </ErrorBoundary>
  )
}
