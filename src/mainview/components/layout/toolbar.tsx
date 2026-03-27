import {
  Sun,
  Moon,
  Monitor,
  Download,
  FolderOpen,
  FolderSearch,
  Settings,
  PanelLeft,
  PanelLeftClose,
  Star,
} from 'lucide-react'
import { useShallow } from 'zustand/react/shallow'
import { useUIStore } from '@/stores/ui-store'
import { useEditorStore } from '@/stores/editor-store'
import { useProjectStore } from '@/stores/project-store'
import { compileToPdf, ensurePackagesForCompile } from '@/lib/compiler'
import { useSettingsStore } from '@/stores/settings-store'
import { isMacOS, revealLabel } from '@/lib/platform'
import { applyPagePreamble, ensureCompilerReady } from '@/lib/compile-manager'
import { findPreviewImportSpecs } from '@/lib/universe-registry'

function ThemeToggle() {
  const theme = useUIStore((s) => s.theme)
  const setTheme = useUIStore((s) => s.setTheme)

  const cycle = () => {
    const next = theme === 'system' ? 'light' : theme === 'light' ? 'dark' : 'system'
    setTheme(next)
  }

  const Icon = theme === 'system' ? Monitor : theme === 'light' ? Sun : Moon
  const label = theme === 'system'
    ? 'Theme: system (click to switch to light)'
    : theme === 'light'
      ? 'Theme: light (click to switch to dark)'
      : 'Theme: dark (click to switch to system)'

  return (
    <button onClick={cycle} className="toolbar-button" title={label} aria-label={label}>
      <Icon size={16} />
    </button>
  )
}

async function handleDownloadPdf() {
  try {
    const store = useProjectStore.getState()
    const project = store.getCurrentProject()
    const currentFilePath = store.currentFilePath
    const liveSource = useEditorStore.getState().source
    const compileInputs = await store.getCompileBundle(liveSource, currentFilePath)

    await ensureCompilerReady()

    const packageSpecs = new Set<string>(findPreviewImportSpecs(compileInputs.mainSource))
    for (const file of compileInputs.extraFiles) {
      for (const spec of findPreviewImportSpecs(file.content)) {
        packageSpecs.add(spec)
      }
    }

    if (packageSpecs.size > 0) {
      await ensurePackagesForCompile([...packageSpecs])
    }

    const pdf = await compileToPdf(
      applyPagePreamble(compileInputs.mainSource),
      compileInputs.extraFiles,
      compileInputs.mainPath,
      compileInputs.extraBinaryFiles,
    )

    if (!pdf) return

    const blob = new Blob([new Uint8Array(pdf)], { type: 'application/pdf' })
    const url = URL.createObjectURL(blob)
    const anchor = document.createElement('a')
    anchor.href = url
    anchor.download = `${project?.name ?? 'document'}.pdf`
    anchor.click()
    setTimeout(() => URL.revokeObjectURL(url), 10_000)
  } catch (error) {
    console.error('Failed to export PDF:', error)
    window.alert('Failed to export PDF. Please try again.')
  }
}

export function Toolbar() {
  const {
    sidebarOpen,
    setSidebarOpen,
    currentProjectId,
    currentFilePath,
    currentFavorite,
  } = useProjectStore(
    useShallow((s) => ({
      sidebarOpen: s.sidebarOpen,
      setSidebarOpen: s.setSidebarOpen,
      currentProjectId: s.currentProjectId,
      currentFilePath: s.currentFilePath,
      currentFavorite: s.metadata?.recentVaults.find((entry) => entry.rootPath === s.currentProjectId)?.favorite ?? false,
    })),
  )
  const setSettingsOpen = useSettingsStore((s) => s.setSettingsOpen)

  const fileName = currentFilePath?.split('/').pop() ?? 'main.typ'

  return (
    <header
      className="flex items-center h-10 shrink-0 select-none"
      style={{
        background: 'var(--bg-surface)',
        borderBottom: '1px solid var(--border-default)',
      } as React.CSSProperties}
    >
      {/* Left section — extra left padding for macOS traffic lights */}
      <div className="flex items-center gap-1 shrink-0" style={{ paddingLeft: isMacOS ? '84px' : '16px', paddingRight: '12px', position: 'relative', zIndex: 10 } as React.CSSProperties}>
        <button
          className="inline-flex items-center justify-center shrink-0"
          onClick={() => useProjectStore.getState().goHome()}
          title="Back to projects"
          style={{
            width: '24px',
            height: '24px',
            background: 'var(--accent)',
            color: '#fff',
            fontFamily: 'var(--font-mono)',
            fontWeight: 700,
            fontSize: '12px',
            lineHeight: 1,
            borderRadius: '1px',
            letterSpacing: '-0.01em',
            border: 'none',
            cursor: 'pointer',
          }}
        >
          t.
        </button>
        <div style={{ width: '4px' }} />
        <button
          className="toolbar-button"
          title={sidebarOpen ? 'Hide sidebar' : 'Show sidebar'}
          onClick={() => setSidebarOpen(!sidebarOpen)}
        >
          {sidebarOpen ? <PanelLeftClose size={16} /> : <PanelLeft size={16} />}
        </button>
        <button
          className="toolbar-button"
          title="Open project"
          onClick={() => void useProjectStore.getState().openProjectDialog()}
        >
          <FolderOpen size={16} />
        </button>
        <button
          className="toolbar-button"
          title={revealLabel}
          onClick={() => void useProjectStore.getState().revealCurrentProjectInFinder()}
          disabled={!currentProjectId}
        >
          <FolderSearch size={16} />
        </button>
        <button
          className="toolbar-button"
          title="Settings"
          onClick={() => setSettingsOpen(true)}
        >
          <Settings size={16} />
        </button>
        <ThemeToggle />
      </div>

      {/* Center — file tab (drag region for window move) */}
      <div className="flex-1 flex justify-center electrobun-webkit-app-region-drag">
        <div
          className="flex items-center gap-2"
          style={{
            padding: '4px 12px',
            color: 'var(--text-secondary)',
            fontFamily: 'var(--font-mono)',
            fontSize: '12px',
            letterSpacing: '0.05em',
            textTransform: 'uppercase',
            maxWidth: 'min(560px, 52vw)',
          }}
        >
          <span className="truncate" style={{ minWidth: 0, flex: 1 }}>
            {fileName}
          </span>
          <Star
            size={12}
            fill={currentFavorite ? 'currentColor' : 'none'}
            style={{ color: currentFavorite ? 'var(--accent)' : 'var(--text-tertiary)', flexShrink: 0 }}
          />
        </div>
      </div>

      {/* Right section */}
      <div className="flex items-center gap-1 pl-3 pr-5 shrink-0" style={{ position: 'relative', zIndex: 10 }}>
        <button
          className="toolbar-button"
          title={currentFavorite ? 'Unfavorite project' : 'Favorite project'}
          onClick={() => currentProjectId && void useProjectStore.getState().toggleFavoriteProject(currentProjectId)}
          disabled={!currentProjectId}
        >
          <Star
            size={16}
            fill={currentFavorite ? 'currentColor' : 'none'}
            style={{ color: currentFavorite ? 'var(--accent)' : undefined }}
          />
        </button>
        <button className="toolbar-button" title="Download PDF" onClick={() => void handleDownloadPdf()}>
          <Download size={16} />
        </button>
      </div>
    </header>
  )
}
