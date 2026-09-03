import { useEffect, lazy, Suspense } from 'react'
import { Loader2 } from 'lucide-react'
import { useProjectStore } from '@/stores/project-store'
import { useSettingsStore } from '@/stores/settings-store'
import { useUIStore } from '@/stores/ui-store'
import { usePresentationStore } from '@/stores/presentation-store'
import { preloadWorkspaceShell } from '@/components/workspace/preload'
import { isLinux, isMacOS } from '@/lib/platform'
import {
  desktopRpc,
  onPresentationAudienceClosed,
  onPresentationCommand,
  onPresentationInput,
} from '@/lib/desktop-rpc'
import { perfMark, perfMeasure } from '@/lib/perf'

const HomeShell = lazy(() => import('@/components/home/home-shell'))
const WorkspaceShell = lazy(() => import('@/components/workspace/workspace-shell'))
const PresenterShell = lazy(() => import('@/components/presentation/presenter-shell'))
const SingleWindowPresentation = lazy(() => import('@/components/presentation/single-window-presentation'))

function FullscreenLoading({ label }: { label: string }) {
  return (
    <div
      className="flex items-center justify-center h-full w-full"
      style={{ background: 'transparent' }}
    >
      <div className="flex flex-col items-center gap-3" style={{ color: 'var(--text-tertiary)' }}>
        <Loader2 size={20} className="animate-spin" />
        <span
          style={{
            fontFamily: 'var(--font-mono)',
            fontSize: '12px',
            letterSpacing: '0.05em',
            textTransform: 'uppercase',
          }}
        >
          {label}
        </span>
      </div>
    </div>
  )
}

function usePresentationBridge() {
  useEffect(() => {
    const unsubscribeInput = onPresentationInput((input) => {
      usePresentationStore.getState().handleInput(input)
    })
    const unsubscribeClosed = onPresentationAudienceClosed(() => {
      usePresentationStore.setState({ audienceOpen: false, audienceDisplayId: null })
      void usePresentationStore.getState().refreshDisplays()
    })
    const unsubscribeCommand = onPresentationCommand((command) => {
      const store = usePresentationStore.getState()
      if (!useProjectStore.getState().hasSelectedProject) return
      switch (command) {
        case 'present-here':
          void store.start('single')
          break
        case 'presenter-view':
          void store.start('presenter')
          break
        case 'end-presentation':
          void store.end()
          break
      }
    })
    // Leaving the project (home screen, switching vaults) tears the deck down.
    const unsubscribeProject = useProjectStore.subscribe((state, prev) => {
      if (state.currentProjectId === prev.currentProjectId && state.hasSelectedProject === prev.hasSelectedProject) return
      if (usePresentationStore.getState().mode !== 'off') void usePresentationStore.getState().end()
    })
    return () => {
      unsubscribeInput()
      unsubscribeClosed()
      unsubscribeCommand()
      unsubscribeProject()
    }
  }, [])
}

export default function App() {
  const loading = useProjectStore((s) => s.loading)
  const hasSelectedProject = useProjectStore((s) => s.hasSelectedProject)
  const loadProjects = useProjectStore((s) => s.loadProjects)
  const presentationMode = usePresentationStore((s) => s.mode)

  const translucent = useSettingsStore((s) => s.translucent)

  usePresentationBridge()

  useEffect(() => {
    const start = perfMark()
    void loadProjects().then(() => {
      perfMeasure('app.bootstrap', start)
    })
    useSettingsStore.getState().loadSettings()
  }, [loadProjects])

  useEffect(() => {
    document.documentElement.classList.toggle('opaque', isLinux || !translucent)
  }, [translucent])

  useEffect(() => {
    if (presentationMode === 'off') return
    // Drop focus from the editor so its keymaps do not swallow slide keys.
    ;(document.activeElement as HTMLElement | null)?.blur?.()
  }, [presentationMode])

  useEffect(() => {
    const handleKeyDown = (e: KeyboardEvent) => {
      const mod = e.metaKey || e.ctrlKey
      const presenting = usePresentationStore.getState().mode !== 'off'
      if (mod && e.key === 's') {
        e.preventDefault()
        void useProjectStore.getState().saveCurrentProject()
      }
      if (mod && e.key === 'k' && !presenting) {
        e.preventDefault()
        // CommandSearch is only mounted inside the workspace shell; toggling on
        // the home screen would silently flip state with no visible UI.
        if (!useProjectStore.getState().hasSelectedProject) return
        const { commandSearchOpen, setCommandSearchOpen } = useUIStore.getState()
        setCommandSearchOpen(!commandSearchOpen)
      }
      if (mod && e.shiftKey && (e.key === 'P' || e.key === 'p') && !e.altKey) {
        e.preventDefault()
        if (!useProjectStore.getState().hasSelectedProject) return
        const store = usePresentationStore.getState()
        if (store.mode === 'off') void store.start('single')
        else void store.end()
      }
      if (mod && e.altKey && (e.key === 'P' || e.key === 'p' || e.code === 'KeyP')) {
        e.preventDefault()
        if (!useProjectStore.getState().hasSelectedProject) return
        void usePresentationStore.getState().start('presenter')
      }
      if ((isMacOS && e.metaKey && e.key === 'q') || (!isMacOS && e.ctrlKey && e.key === 'q')) {
        e.preventDefault()
        void desktopRpc.request.quitApp()
      }
    }
    window.addEventListener('keydown', handleKeyDown)
    return () => window.removeEventListener('keydown', handleKeyDown)
  }, [])

  if (loading) {
    return <FullscreenLoading label="Loading..." />
  }

  if (!hasSelectedProject) {
    return (
      <Suspense fallback={<FullscreenLoading label="Loading home..." />}>
        <HomeShell onPreloadWorkspace={() => { void preloadWorkspaceShell() }} />
      </Suspense>
    )
  }

  const presenting = presentationMode !== 'off'

  return (
    <div className="h-full w-full" style={{ position: 'relative' }}>
      {/* The workspace stays mounted while presenting so editor state, the
          compile pipeline, and file watchers keep running underneath. */}
      <div
        aria-hidden={presenting || undefined}
        className="h-full w-full"
        style={presenting ? { visibility: 'hidden', pointerEvents: 'none', position: 'absolute', inset: 0 } : undefined}
      >
        <Suspense fallback={<FullscreenLoading label="Loading workspace..." />}>
          <WorkspaceShell />
        </Suspense>
      </div>
      {presenting && (
        <div className="h-full w-full" style={{ position: 'absolute', inset: 0, zIndex: 100 }}>
          <Suspense fallback={<FullscreenLoading label="Starting presentation..." />}>
            {presentationMode === 'presenter' ? <PresenterShell /> : <SingleWindowPresentation />}
          </Suspense>
        </div>
      )}
    </div>
  )
}
