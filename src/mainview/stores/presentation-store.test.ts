import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest'
import type { PresentationSnapshot } from '../../shared/presentation'

const rpcRequests = {
  presentationPublish: vi.fn(async (_snapshot: PresentationSnapshot) => ({ ok: true })),
  presentationOpenAudience: vi.fn(async ({ displayId }: { displayId: number | null }) => ({ ok: true, displayId: displayId ?? 2 })),
  presentationCloseAudience: vi.fn(async () => ({ ok: true })),
  presentationGetDisplays: vi.fn(async () => ({ displays: [] })),
  setMainWindowFullScreen: vi.fn(async () => ({ ok: true })),
  readFile: vi.fn(async () => null),
}

const projectFiles: Array<{ path: string; loaded: boolean; isBinary: boolean; content: string }> = []
const projectState = {
  currentFilePath: 'deck.typ' as string | null,
  currentProjectId: 'vault',
  hasSelectedProject: true,
  getCurrentProject: () => ({ name: 'Talk', rootPath: '/vault', files: projectFiles }),
  updateFileContent: vi.fn(),
}

vi.mock('@/lib/desktop-rpc', () => ({
  desktopRpc: { request: rpcRequests },
}))

vi.mock('@/stores/project-store', () => ({
  useProjectStore: {
    getState: () => projectState,
    subscribe: () => () => {},
  },
}))

const PAGES = [{ width: 720, height: 405 }, { width: 720, height: 405 }, { width: 720, height: 405 }]

async function loadHarness() {
  vi.resetModules()
  const { usePresentationStore, computeTimerElapsedMs, formatDuration } = await import('./presentation-store')
  const { useCompileStore } = await import('./compile-store')
  useCompileStore.setState({
    svg: '<svg></svg>',
    pageDimensions: PAGES,
    totalPages: PAGES.length,
    speakerNotes: [{ page: 2, text: 'inline note' }],
    compiledMainPath: 'deck.typ',
  })
  return { usePresentationStore, useCompileStore, computeTimerElapsedMs, formatDuration }
}

describe('presentation store', () => {
  beforeEach(() => {
    vi.useFakeTimers()
    for (const mock of Object.values(rpcRequests)) mock.mockClear()
    projectState.updateFileContent.mockClear()
    projectFiles.length = 0
  })

  afterEach(() => {
    vi.useRealTimers()
  })

  it('starts a single-window session from the compiled document and goes fullscreen', async () => {
    const { usePresentationStore } = await loadHarness()
    await usePresentationStore.getState().start('single')

    const store = usePresentationStore.getState()
    expect(store.mode).toBe('single')
    expect(store.slideDeck?.slideCount).toBe(3)
    expect(store.deck?.title).toBe('Talk · deck.typ')
    expect(store.sidecarPath).toBe('deck.notes.md')
    expect(store.inlineNotes).toEqual([{ page: 2, text: 'inline note' }])
    expect(rpcRequests.setMainWindowFullScreen).toHaveBeenCalledWith({ fullScreen: true })
    // No audience window, so nothing is pushed over RPC.
    expect(rpcRequests.presentationPublish).not.toHaveBeenCalled()
  })

  it('navigates within bounds and un-blanks before moving', async () => {
    const { usePresentationStore } = await loadHarness()
    await usePresentationStore.getState().start('single')
    const api = usePresentationStore.getState()

    api.next()
    api.next()
    api.next()
    expect(usePresentationStore.getState().state.slide).toBe(2)

    api.performAction('toggle-black')
    expect(usePresentationStore.getState().state.blackout).toBe('black')
    api.prev()
    expect(usePresentationStore.getState().state.blackout).toBe('none')
    expect(usePresentationStore.getState().state.slide).toBe(2)

    api.goto(-5)
    expect(usePresentationStore.getState().state.slide).toBe(0)
    api.performAction('last')
    expect(usePresentationStore.getState().state.slide).toBe(2)
  })

  it('manages annotations per slide', async () => {
    const { usePresentationStore } = await loadHarness()
    await usePresentationStore.getState().start('single')
    const api = usePresentationStore.getState()
    const stroke = { id: 's1', tool: 'pen' as const, color: '#fff', width: 0.01, points: [{ x: 0.1, y: 0.1 }] }

    api.upsertStroke(0, stroke)
    api.upsertStroke(0, { ...stroke, points: [...stroke.points, { x: 0.2, y: 0.2 }] })
    api.upsertStroke(1, { ...stroke, id: 's2' })
    expect(usePresentationStore.getState().state.annotations['0']).toHaveLength(1)
    expect(usePresentationStore.getState().state.annotations['0'][0].points).toHaveLength(2)

    api.eraseStroke(0, 's1')
    expect(usePresentationStore.getState().state.annotations['0']).toEqual([])
    api.clearAnnotations(1)
    expect(usePresentationStore.getState().state.annotations['1']).toBeUndefined()
  })

  it('publishes coalesced snapshots to the audience window in presenter mode', async () => {
    const { usePresentationStore } = await loadHarness()
    await usePresentationStore.getState().start('presenter')

    expect(rpcRequests.presentationOpenAudience).toHaveBeenCalledWith({ displayId: null })
    expect(usePresentationStore.getState().audienceOpen).toBe(true)
    expect(usePresentationStore.getState().timerRunning).toBe(true)
    const initialPublishes = rpcRequests.presentationPublish.mock.calls.length
    expect(initialPublishes).toBeGreaterThan(0)

    const api = usePresentationStore.getState()
    api.setLaser({ x: 0.1, y: 0.1, visible: true })
    api.setLaser({ x: 0.2, y: 0.2, visible: true })
    api.setLaser({ x: 0.3, y: 0.3, visible: true })
    expect(rpcRequests.presentationPublish.mock.calls.length).toBe(initialPublishes)

    vi.advanceTimersByTime(40)
    expect(rpcRequests.presentationPublish.mock.calls.length).toBe(initialPublishes + 1)
    const lastCall = rpcRequests.presentationPublish.mock.calls.at(-1)?.[0]
    expect(lastCall?.state?.laser.x).toBeCloseTo(0.3)
  })

  it('applies audience input and resends the snapshot when the audience reports ready', async () => {
    const { usePresentationStore } = await loadHarness()
    await usePresentationStore.getState().start('presenter')
    const api = usePresentationStore.getState()
    const before = rpcRequests.presentationPublish.mock.calls.length

    api.handleInput({ kind: 'action', action: 'next' })
    expect(usePresentationStore.getState().state.slide).toBe(1)
    api.handleInput({ kind: 'goto', slide: 2 })
    expect(usePresentationStore.getState().state.slide).toBe(2)
    api.handleInput({ kind: 'stroke', slide: 2, stroke: { id: 'a', tool: 'pen', color: '#f00', width: 0.01, points: [] } })
    expect(usePresentationStore.getState().state.annotations['2']).toHaveLength(1)

    api.handleInput({ kind: 'ready' })
    const last = rpcRequests.presentationPublish.mock.calls.at(-1)?.[0]
    expect(rpcRequests.presentationPublish.mock.calls.length).toBeGreaterThan(before)
    expect(last?.deck).toBeDefined()
    expect(last?.state).toBeDefined()
  })

  it('writes sidecar notes after a debounce and flushes them on end', async () => {
    const { usePresentationStore } = await loadHarness()
    await usePresentationStore.getState().start('single')
    const api = usePresentationStore.getState()

    api.setSidecarNote(1, 'Remember the demo')
    expect(projectState.updateFileContent).not.toHaveBeenCalled()
    vi.advanceTimersByTime(700)
    expect(projectState.updateFileContent).toHaveBeenCalledTimes(1)
    const [path, markdown] = projectState.updateFileContent.mock.calls[0] as [string, string]
    expect(path).toBe('deck.notes.md')
    expect(markdown).toContain('## Slide 2')
    expect(markdown).toContain('Remember the demo')

    api.setSidecarNote(0, 'Say hello')
    await usePresentationStore.getState().end()
    expect(projectState.updateFileContent).toHaveBeenCalledTimes(2)
    expect(usePresentationStore.getState().mode).toBe('off')
    expect(rpcRequests.setMainWindowFullScreen).toHaveBeenLastCalledWith({ fullScreen: false })
  })

  it('does not create an empty sidecar file', async () => {
    const { usePresentationStore } = await loadHarness()
    await usePresentationStore.getState().start('single')
    usePresentationStore.getState().setSidecarNote(0, '   ')
    vi.advanceTimersByTime(700)
    expect(projectState.updateFileContent).not.toHaveBeenCalled()
  })

  it('refreshes the deck when a recompile lands and clamps the current slide', async () => {
    const { usePresentationStore, useCompileStore } = await loadHarness()
    await usePresentationStore.getState().start('single')
    usePresentationStore.getState().goto(2)

    useCompileStore.setState({ svg: '<svg>v2</svg>', pageDimensions: PAGES.slice(0, 2), totalPages: 2 })
    const store = usePresentationStore.getState()
    expect(store.slideDeck?.slideCount).toBe(2)
    expect(store.state.slide).toBe(1)
  })

  it('ends the session and tells the audience', async () => {
    const { usePresentationStore } = await loadHarness()
    await usePresentationStore.getState().start('presenter')
    await usePresentationStore.getState().end()

    expect(rpcRequests.presentationPublish).toHaveBeenCalledWith({ ended: true })
    expect(rpcRequests.presentationCloseAudience).toHaveBeenCalled()
    expect(usePresentationStore.getState().audienceOpen).toBe(false)
    expect(usePresentationStore.getState().slideDeck).toBeNull()
  })

  it('formats and accumulates the timer across pauses', async () => {
    const { usePresentationStore, computeTimerElapsedMs, formatDuration } = await loadHarness()
    vi.setSystemTime(new Date('2026-01-01T10:00:00Z'))
    const api = usePresentationStore.getState()
    api.toggleTimer()
    vi.setSystemTime(new Date('2026-01-01T10:01:30Z'))
    api.toggleTimer()
    vi.setSystemTime(new Date('2026-01-01T10:05:00Z'))
    expect(computeTimerElapsedMs(usePresentationStore.getState())).toBe(90_000)
    expect(formatDuration(90_000)).toBe('01:30')
    expect(formatDuration(3_725_000)).toBe('1:02:05')
    usePresentationStore.getState().resetTimer()
    expect(computeTimerElapsedMs(usePresentationStore.getState())).toBe(0)
  })
})
