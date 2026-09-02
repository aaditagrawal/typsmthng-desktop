import { create } from 'zustand'
import {
  clampSlide,
  createInitialPresentationState,
  type BlackoutMode,
  type DisplayInfo,
  type LaserPointer,
  type NotesLayout,
  type PresentationAction,
  type PresentationDeck,
  type PresentationInput,
  type PresentationState,
  type PresentationTool,
  type Stroke,
} from '../../shared/presentation'
import { desktopRpc } from '@/lib/desktop-rpc'
import { SlideDeck } from '@/lib/slide-deck'
import {
  notesSidecarPath,
  parseSidecarNotes,
  serializeSidecarNotes,
  type InlineSpeakerNote,
} from '@/lib/presentation-notes'
import { normalizeExtension } from '@/lib/file-classification'
import { useCompileStore } from '@/stores/compile-store'
import { useProjectStore } from '@/stores/project-store'

export type PresentationMode = 'off' | 'single' | 'presenter'

/** Publishing cadence for the audience; laser motion is the hot path. */
const PUBLISH_INTERVAL_MS = 33
const NOTES_WRITE_DEBOUNCE_MS = 600
const DECK_DISPOSE_DELAY_MS = 1000
const NOTES_LAYOUT_STORAGE_KEY = 'typsmthng.presentation.notesLayout'

interface PresentationStoreState {
  mode: PresentationMode
  deck: PresentationDeck | null
  slideDeck: SlideDeck | null
  state: PresentationState
  notesLayout: NotesLayout
  gridOpen: boolean
  /** Single-window mode: the slide-over notes drawer. */
  notesOverlayOpen: boolean
  /** Presenter mode: the next-slide + notes column. */
  sidebarOpen: boolean
  timerRunning: boolean
  timerStartedAt: number | null
  timerAccumulatedMs: number
  sourcePath: string | null
  sidecarPath: string | null
  sidecarNotes: Map<number, string>
  inlineNotes: InlineSpeakerNote[]
  displays: DisplayInfo[]
  audienceOpen: boolean
  audienceDisplayId: number | null
  mainFullScreen: boolean
  lastError: string | null

  start: (mode: Exclude<PresentationMode, 'off'>, options?: { displayId?: number | null }) => Promise<void>
  end: () => Promise<void>
  refreshDeck: () => void
  goto: (slide: number) => void
  next: () => void
  prev: () => void
  setBlackout: (mode: BlackoutMode) => void
  toggleBlackout: (mode: Exclude<BlackoutMode, 'none'>) => void
  setTool: (tool: PresentationTool) => void
  setPenColor: (color: string) => void
  toggleLaserEnabled: () => void
  setLaser: (pointer: LaserPointer) => void
  upsertStroke: (slide: number, stroke: Stroke) => void
  eraseStroke: (slide: number, strokeId: string) => void
  clearAnnotations: (slide?: number) => void
  setGridOpen: (open: boolean) => void
  setNotesOverlayOpen: (open: boolean) => void
  setSidebarOpen: (open: boolean) => void
  toggleTimer: () => void
  resetTimer: () => void
  setNotesLayout: (layout: NotesLayout) => void
  setSidecarNote: (slide: number, text: string) => void
  refreshDisplays: () => Promise<void>
  openAudience: (displayId: number | null) => Promise<void>
  closeAudience: () => Promise<void>
  setMainFullScreen: (fullScreen: boolean) => Promise<void>
  handleInput: (input: PresentationInput) => void
  performAction: (action: PresentationAction) => void
}

function loadNotesLayoutPreference(): NotesLayout {
  try {
    const stored = window.localStorage?.getItem(NOTES_LAYOUT_STORAGE_KEY)
    if (stored === 'auto' || stored === 'right' || stored === 'none') return stored
  } catch {}
  return 'auto'
}

function saveNotesLayoutPreference(layout: NotesLayout): void {
  try {
    window.localStorage?.setItem(NOTES_LAYOUT_STORAGE_KEY, layout)
  } catch {}
}

function resolvePresentedPath(): string | null {
  const projectState = useProjectStore.getState()
  const current = projectState.currentFilePath
  if (current && normalizeExtension(current) === '.typ') return current
  return useCompileStore.getState().compiledMainPath ?? current
}

function deckTitle(sourcePath: string | null): string {
  const project = useProjectStore.getState().getCurrentProject()
  const file = sourcePath?.split('/').pop()
  if (project && file) return `${project.name} · ${file}`
  return file ?? project?.name ?? 'Presentation'
}

let deckRevision = 0
let publishTimer: ReturnType<typeof setTimeout> | null = null
let publishPendingState = false
let publishPendingDeck = false
let notesWriteTimer: ReturnType<typeof setTimeout> | null = null
let unsubscribeCompile: (() => void) | null = null

function flushPublish(): void {
  publishTimer = null
  const { audienceOpen, deck, state } = usePresentationStore.getState()
  const sendDeck = publishPendingDeck
  const sendState = publishPendingState
  publishPendingDeck = false
  publishPendingState = false
  if (!audienceOpen) return
  if (!sendDeck && !sendState) return

  void desktopRpc.request.presentationPublish({
    ...(sendDeck ? { deck } : {}),
    ...(sendState ? { state } : {}),
  }).catch((error) => {
    console.warn('Failed to publish presentation snapshot:', error)
  })
}

function schedulePublish(parts: { deck?: boolean; state?: boolean }, immediate = false): void {
  if (parts.deck) publishPendingDeck = true
  if (parts.state) publishPendingState = true
  if (immediate) {
    if (publishTimer) clearTimeout(publishTimer)
    flushPublish()
    return
  }
  if (publishTimer) return
  publishTimer = setTimeout(flushPublish, PUBLISH_INTERVAL_MS)
}

function writeSidecarNow(): void {
  if (notesWriteTimer) {
    clearTimeout(notesWriteTimer)
    notesWriteTimer = null
  }
  const { sidecarPath, sidecarNotes, deck } = usePresentationStore.getState()
  if (!sidecarPath) return
  const project = useProjectStore.getState().getCurrentProject()
  if (!project) return
  const exists = project.files.some((file) => file.path === sidecarPath)
  const hasNotes = Array.from(sidecarNotes.values()).some((text) => text.trim().length > 0)
  // Never create an empty sidecar just because the panel was focused.
  if (!exists && !hasNotes) return
  const markdown = serializeSidecarNotes(sidecarNotes, deck?.title ?? sidecarPath)
  useProjectStore.getState().updateFileContent(sidecarPath, markdown)
}

function scheduleNotesWrite(): void {
  if (notesWriteTimer) clearTimeout(notesWriteTimer)
  notesWriteTimer = setTimeout(writeSidecarNow, NOTES_WRITE_DEBOUNCE_MS)
}

async function loadSidecarNotes(sidecarPath: string): Promise<Map<number, string>> {
  const project = useProjectStore.getState().getCurrentProject()
  if (!project) return new Map()
  const entry = project.files.find((file) => file.path === sidecarPath)
  if (!entry) return new Map()
  if (entry.loaded && !entry.isBinary) return parseSidecarNotes(entry.content).notes
  try {
    const file = await desktopRpc.request.readFile({ rootPath: project.rootPath, path: sidecarPath })
    return file && !file.isBinary ? parseSidecarNotes(file.content).notes : new Map()
  } catch {
    return new Map()
  }
}

function updateState(
  set: (partial: Partial<PresentationStoreState> | ((state: PresentationStoreState) => Partial<PresentationStoreState>)) => void,
  patch: (state: PresentationState) => Partial<PresentationState>,
  immediate = false,
): void {
  set((store) => ({ state: { ...store.state, ...patch(store.state) } }))
  schedulePublish({ state: true }, immediate)
}

export const usePresentationStore = create<PresentationStoreState>((set, get) => ({
  mode: 'off',
  deck: null,
  slideDeck: null,
  state: createInitialPresentationState(),
  notesLayout: loadNotesLayoutPreference(),
  gridOpen: false,
  notesOverlayOpen: false,
  sidebarOpen: true,
  timerRunning: false,
  timerStartedAt: null,
  timerAccumulatedMs: 0,
  sourcePath: null,
  sidecarPath: null,
  sidecarNotes: new Map(),
  inlineNotes: [],
  displays: [],
  audienceOpen: false,
  audienceDisplayId: null,
  mainFullScreen: false,
  lastError: null,

  start: async (mode, options) => {
    const sourcePath = resolvePresentedPath()
    const sidecarPath = sourcePath ? notesSidecarPath(sourcePath) : null

    if (get().mode === 'off') {
      set({
        state: createInitialPresentationState(),
        timerRunning: false,
        timerStartedAt: null,
        timerAccumulatedMs: 0,
        gridOpen: false,
        notesOverlayOpen: false,
        lastError: null,
      })
    }

    set({ mode, sourcePath, sidecarPath })
    get().refreshDeck()

    if (!unsubscribeCompile) {
      unsubscribeCompile = useCompileStore.subscribe((state, prev) => {
        if (state.svg === prev.svg && state.pageDimensions === prev.pageDimensions && state.speakerNotes === prev.speakerNotes) return
        if (usePresentationStore.getState().mode === 'off') return
        usePresentationStore.getState().refreshDeck()
      })
    }

    if (sidecarPath) {
      const sidecarNotes = await loadSidecarNotes(sidecarPath)
      if (get().sidecarPath === sidecarPath) set({ sidecarNotes })
    }

    if (!get().timerRunning) get().toggleTimer()

    if (mode === 'single') {
      await get().setMainFullScreen(true)
      return
    }

    await get().refreshDisplays()
    await get().openAudience(options?.displayId ?? null)
  },

  end: async () => {
    if (get().mode === 'off') return
    if (notesWriteTimer) writeSidecarNow()

    unsubscribeCompile?.()
    unsubscribeCompile = null
    if (publishTimer) {
      clearTimeout(publishTimer)
      publishTimer = null
    }
    publishPendingDeck = false
    publishPendingState = false

    const { slideDeck, mainFullScreen, audienceOpen } = get()
    slideDeck?.dispose()
    set({
      mode: 'off',
      deck: null,
      slideDeck: null,
      gridOpen: false,
      notesOverlayOpen: false,
      timerRunning: false,
      audienceOpen: false,
      audienceDisplayId: null,
    })

    const tasks: Promise<unknown>[] = []
    if (audienceOpen) {
      tasks.push(desktopRpc.request.presentationPublish({ ended: true }).catch(() => {}))
      tasks.push(desktopRpc.request.presentationCloseAudience().catch(() => {}))
    }
    if (mainFullScreen) {
      tasks.push(get().setMainFullScreen(false))
    }
    await Promise.all(tasks)
  },

  refreshDeck: () => {
    const { svg, pageDimensions, speakerNotes } = useCompileStore.getState()
    const { slideDeck: previous, notesLayout, sourcePath, state } = get()
    if (!svg || pageDimensions.length === 0) {
      return
    }

    const deck: PresentationDeck = {
      revision: ++deckRevision,
      title: deckTitle(sourcePath),
      svg,
      pages: pageDimensions,
      notesLayout,
    }
    const slideDeck = new SlideDeck(deck)
    // The old blob URLs may still be painting the outgoing slide during the
    // cross-fade, so release them after the transition has finished.
    if (previous) setTimeout(() => previous.dispose(), DECK_DISPOSE_DELAY_MS)

    set({
      deck,
      slideDeck,
      inlineNotes: speakerNotes,
      state: { ...state, slide: clampSlide(state.slide, slideDeck.slideCount) },
    })
    schedulePublish({ deck: true, state: true }, true)
  },

  goto: (slide) => {
    const count = get().slideDeck?.slideCount ?? 0
    const target = clampSlide(slide, count)
    updateState(set, (state) => ({
      slide: target,
      blackout: 'none',
      laser: state.laser.visible ? { ...state.laser, visible: false } : state.laser,
    }), true)
  },

  next: () => {
    const { state } = get()
    if (state.blackout !== 'none') {
      get().setBlackout('none')
      return
    }
    get().goto(state.slide + 1)
  },

  prev: () => {
    const { state } = get()
    if (state.blackout !== 'none') {
      get().setBlackout('none')
      return
    }
    get().goto(state.slide - 1)
  },

  setBlackout: (mode) => updateState(set, () => ({ blackout: mode }), true),

  toggleBlackout: (mode) => {
    const current = get().state.blackout
    get().setBlackout(current === mode ? 'none' : mode)
  },

  setTool: (tool) => updateState(set, () => ({ tool }), true),

  setPenColor: (color) => updateState(set, () => ({ penColor: color })),

  toggleLaserEnabled: () => updateState(set, (state) => ({
    laserEnabled: !state.laserEnabled,
    laser: state.laserEnabled ? { ...state.laser, visible: false } : state.laser,
  }), true),

  setLaser: (pointer) => {
    const { state } = get()
    if (
      state.laser.visible === pointer.visible
      && Math.abs(state.laser.x - pointer.x) < 0.0005
      && Math.abs(state.laser.y - pointer.y) < 0.0005
    ) return
    updateState(set, () => ({ laser: pointer }))
  },

  upsertStroke: (slide, stroke) => {
    updateState(set, (state) => {
      const key = String(slide)
      const existing = state.annotations[key] ?? []
      const index = existing.findIndex((entry) => entry.id === stroke.id)
      const nextStrokes = index >= 0
        ? existing.map((entry, i) => (i === index ? stroke : entry))
        : [...existing, stroke]
      return { annotations: { ...state.annotations, [key]: nextStrokes } }
    })
  },

  eraseStroke: (slide, strokeId) => {
    updateState(set, (state) => {
      const key = String(slide)
      const existing = state.annotations[key]
      if (!existing) return {}
      return { annotations: { ...state.annotations, [key]: existing.filter((entry) => entry.id !== strokeId) } }
    })
  },

  clearAnnotations: (slide) => {
    updateState(set, (state) => {
      if (slide === undefined) return { annotations: {} }
      const key = String(slide)
      if (!state.annotations[key]) return {}
      const { [key]: _removed, ...rest } = state.annotations
      return { annotations: rest }
    }, true)
  },

  setGridOpen: (open) => set({ gridOpen: open }),
  setNotesOverlayOpen: (open) => set({ notesOverlayOpen: open }),
  setSidebarOpen: (open) => set({ sidebarOpen: open }),

  toggleTimer: () => {
    const { timerRunning, timerStartedAt, timerAccumulatedMs } = get()
    if (timerRunning) {
      const elapsed = timerStartedAt ? Date.now() - timerStartedAt : 0
      set({ timerRunning: false, timerStartedAt: null, timerAccumulatedMs: timerAccumulatedMs + elapsed })
    } else {
      set({ timerRunning: true, timerStartedAt: Date.now() })
    }
  },

  resetTimer: () => {
    const { timerRunning } = get()
    set({ timerAccumulatedMs: 0, timerStartedAt: timerRunning ? Date.now() : null })
  },

  setNotesLayout: (layout) => {
    saveNotesLayoutPreference(layout)
    set({ notesLayout: layout })
    if (get().mode !== 'off') get().refreshDeck()
  },

  setSidecarNote: (slide, text) => {
    const next = new Map(get().sidecarNotes)
    if (text.trim()) next.set(slide, text)
    else next.delete(slide)
    set({ sidecarNotes: next })
    scheduleNotesWrite()
  },

  refreshDisplays: async () => {
    try {
      const { displays } = await desktopRpc.request.presentationGetDisplays()
      set({ displays })
    } catch (error) {
      console.warn('Failed to list displays:', error)
    }
  },

  openAudience: async (displayId) => {
    try {
      const result = await desktopRpc.request.presentationOpenAudience({ displayId })
      if (!result.ok) {
        set({ lastError: 'Could not open the audience window.' })
        return
      }
      set({ audienceOpen: true, audienceDisplayId: result.displayId, lastError: null })
      schedulePublish({ deck: true, state: true }, true)
      await get().refreshDisplays()
    } catch (error) {
      set({ lastError: error instanceof Error ? error.message : 'Could not open the audience window.' })
    }
  },

  closeAudience: async () => {
    set({ audienceOpen: false, audienceDisplayId: null })
    try {
      await desktopRpc.request.presentationCloseAudience()
    } catch {}
    await get().refreshDisplays()
  },

  setMainFullScreen: async (fullScreen) => {
    set({ mainFullScreen: fullScreen })
    try {
      await desktopRpc.request.setMainWindowFullScreen({ fullScreen })
    } catch (error) {
      console.warn('Failed to toggle fullscreen:', error)
    }
  },

  handleInput: (input) => {
    const store = get()
    if (store.mode === 'off') return
    switch (input.kind) {
      case 'action':
        store.performAction(input.action)
        break
      case 'goto':
        store.goto(input.slide)
        break
      case 'laser':
        if (store.state.laserEnabled || input.pointer.visible) {
          updateState(set, (state) => ({ laser: input.pointer, laserEnabled: state.laserEnabled || input.pointer.visible }))
        }
        break
      case 'stroke':
        store.upsertStroke(input.slide, input.stroke)
        break
      case 'erase':
        store.eraseStroke(input.slide, input.strokeId)
        break
      case 'ready':
        schedulePublish({ deck: true, state: true }, true)
        break
    }
  },

  performAction: (action) => {
    const store = get()
    switch (action) {
      case 'next':
        store.next()
        break
      case 'prev':
        store.prev()
        break
      case 'first':
        store.goto(0)
        break
      case 'last':
        store.goto((store.slideDeck?.slideCount ?? 1) - 1)
        break
      case 'toggle-black':
        store.toggleBlackout('black')
        break
      case 'toggle-white':
        store.toggleBlackout('white')
        break
      case 'toggle-laser':
        store.toggleLaserEnabled()
        break
      case 'toggle-pen':
        store.setTool(store.state.tool === 'pen' ? 'pointer' : 'pen')
        break
      case 'toggle-highlighter':
        store.setTool(store.state.tool === 'highlighter' ? 'pointer' : 'highlighter')
        break
      case 'toggle-eraser':
        store.setTool(store.state.tool === 'eraser' ? 'pointer' : 'eraser')
        break
      case 'clear-annotations':
        store.clearAnnotations(store.state.slide)
        break
      case 'toggle-grid':
        set({ gridOpen: !store.gridOpen })
        break
      case 'toggle-notes':
        if (store.mode === 'presenter') set({ sidebarOpen: !store.sidebarOpen })
        else set({ notesOverlayOpen: !store.notesOverlayOpen })
        break
      case 'toggle-timer':
        store.toggleTimer()
        break
      case 'reset-timer':
        store.resetTimer()
        break
      case 'toggle-fullscreen':
        void store.setMainFullScreen(!store.mainFullScreen)
        break
      case 'exit':
        if (store.gridOpen) {
          set({ gridOpen: false })
          break
        }
        if (store.notesOverlayOpen && store.mode === 'single') {
          set({ notesOverlayOpen: false })
          break
        }
        void store.end()
        break
    }
  },
}))

export function computeTimerElapsedMs(store: Pick<PresentationStoreState, 'timerRunning' | 'timerStartedAt' | 'timerAccumulatedMs'>, now = Date.now()): number {
  const running = store.timerRunning && store.timerStartedAt ? now - store.timerStartedAt : 0
  return store.timerAccumulatedMs + running
}

export function formatDuration(ms: number): string {
  const totalSeconds = Math.max(0, Math.floor(ms / 1000))
  const hours = Math.floor(totalSeconds / 3600)
  const minutes = Math.floor((totalSeconds % 3600) / 60)
  const seconds = totalSeconds % 60
  const mm = String(minutes).padStart(2, '0')
  const ss = String(seconds).padStart(2, '0')
  return hours > 0 ? `${hours}:${mm}:${ss}` : `${mm}:${ss}`
}
