import { beforeEach, describe, expect, it, vi } from 'vitest'

const initCompilerMock = vi.fn()
const compileTypstMock = vi.fn()
const ensurePackagesForCompileMock = vi.fn()
const getPackageRuntimeEpochMock = vi.fn(() => 0)
const applyPackageImportCompatRewritesMock = vi.fn((source: string) => source)
const perfMarkMock = vi.fn(() => 0)
const perfMeasureMock = vi.fn(() => ({ ms: 10 }))
const perfSampleMock = vi.fn()

const projectState = {
  currentFilePath: 'main.typ' as string | null,
  getCompileBundle: vi.fn(),
  getCurrentProject: vi.fn(() => ({
    templateMeta: undefined,
    files: [],
  })),
}

vi.mock('./compiler', () => ({
  initCompiler: initCompilerMock,
  compileTypst: compileTypstMock,
  ensurePackagesForCompile: ensurePackagesForCompileMock,
  getPackageRuntimeEpoch: getPackageRuntimeEpochMock,
}))

vi.mock('./package-compat', () => ({
  applyPackageImportCompatRewrites: applyPackageImportCompatRewritesMock,
}))

vi.mock('./perf', () => ({
  perfMark: perfMarkMock,
  perfMeasure: perfMeasureMock,
  perfSample: perfSampleMock,
}))

vi.mock('@/stores/project-store', () => ({
  useProjectStore: {
    getState: () => projectState,
  },
}))

async function loadHarness() {
  vi.resetModules()
  const compileManager = await import('./compile-manager')
  const { useCompileStore } = await import('../stores/compile-store')
  const { useSettingsStore } = await import('../stores/settings-store')
  const { useEditorStore } = await import('../stores/editor-store')

  useCompileStore.setState({
    status: 'idle',
    diagnostics: [],
    svg: null,
    vectorData: null,
    pageDimensions: [],
    totalPages: 0,
    errorCount: 0,
    warningCount: 0,
    compileTime: 0,
    autoCompile: true,
  })
  useSettingsStore.setState({
    autoCompile: true,
    compileDelay: 150,
    pageSize: 'auto',
    settingsOpen: false,
    fontSize: 15,
    lineWrapping: true,
    lineNumbers: true,
    theme: 'dark',
    vimMode: false,
  })
  useEditorStore.setState({
    source: '',
    isDirty: false,
    saveStatus: 'saved',
    editorView: null,
    lastUserEditAt: 0,
  })

  return { compileManager, useCompileStore, useSettingsStore, useEditorStore }
}

describe('compile-manager', () => {
  beforeEach(() => {
    vi.useRealTimers()
    vi.clearAllMocks()

    projectState.currentFilePath = 'main.typ'
    projectState.getCompileBundle.mockReset()
    projectState.getCurrentProject.mockReset()
    projectState.getCurrentProject.mockReturnValue({
      templateMeta: undefined,
      files: [],
    })

    initCompilerMock.mockResolvedValue(undefined)
    ensurePackagesForCompileMock.mockResolvedValue(undefined)
    getPackageRuntimeEpochMock.mockReturnValue(0)
    compileTypstMock.mockResolvedValue({
      svg: '<svg />',
      vectorData: new Uint8Array([1]),
      pageDimensions: [{ width: 640, height: 480 }],
      diagnostics: [],
      success: true,
      timings: {
        compileMs: 4,
        renderMs: 3,
        totalMs: 7,
      },
    })
  })

  it('injects a page preamble only when needed', async () => {
    const { compileManager, useSettingsStore } = await loadHarness()
    useSettingsStore.setState({ pageSize: 'a4' })

    expect(compileManager.applyPagePreamble('Hello')).toBe('#set page(paper: "a4")\nHello')
    expect(compileManager.getInjectedPreambleLineCount('Hello')).toBe(1)
    expect(compileManager.applyPagePreamble('#set page(width: 10cm)\nHello')).toBe('#set page(width: 10cm)\nHello')
  })

  it('skips non-typ files and clears the preview state', async () => {
    projectState.currentFilePath = 'notes.md'
    const { compileManager, useCompileStore } = await loadHarness()
    useCompileStore.setState({
      status: 'success',
      diagnostics: [{ severity: 'warning', path: 'notes.md', range: '', message: 'old' }],
      svg: '<svg />',
      vectorData: new Uint8Array([9]),
      pageDimensions: [{ width: 1, height: 1 }],
      totalPages: 1,
      errorCount: 0,
      warningCount: 1,
      compileTime: 12,
      autoCompile: true,
    })

    await compileManager.forceCompile('plain text', 'notes.md')

    expect(projectState.getCompileBundle).not.toHaveBeenCalled()
    expect(compileTypstMock).not.toHaveBeenCalled()
    expect(useCompileStore.getState()).toMatchObject({
      status: 'idle',
      svg: null,
      vectorData: null,
      pageDimensions: [],
      diagnostics: [],
      totalPages: 0,
      errorCount: 0,
      warningCount: 0,
    })
  })

  it('builds compile inputs, resolves packages, and stores a successful result', async () => {
    projectState.getCompileBundle.mockResolvedValue({
      mainPath: '/main.typ',
      mainSource: '@preview/example:1.0.0\nHello',
      extraFiles: [
        { path: '/chapter.typ', content: '#import "@preview/example:1.0.0": demo' },
      ],
      extraBinaryFiles: [
        { path: '/assets/logo.png', data: new Uint8Array([2, 4]) },
      ],
    })

    const { compileManager, useCompileStore, useSettingsStore } = await loadHarness()
    useSettingsStore.setState({ pageSize: 'a4' })

    await compileManager.forceCompile('live source', 'main.typ')

    expect(projectState.getCompileBundle).toHaveBeenCalledWith('live source', 'main.typ')
    expect(initCompilerMock).toHaveBeenCalledWith('@preview/example:1.0.0\nHello', [
      { path: '/chapter.typ', content: '#import "@preview/example:1.0.0": demo' },
    ])
    expect(ensurePackagesForCompileMock).toHaveBeenCalledWith(['@preview/example:1.0.0'])
    expect(compileTypstMock).toHaveBeenCalledWith(
      '#set page(paper: "a4")\n@preview/example:1.0.0\nHello',
      [{ path: '/chapter.typ', content: '#import "@preview/example:1.0.0": demo' }],
      '/main.typ',
      [{ path: '/assets/logo.png', data: new Uint8Array([2, 4]) }],
    )
    expect(useCompileStore.getState()).toMatchObject({
      status: 'success',
      svg: '<svg />',
      totalPages: 1,
      compileTime: 10,
      diagnostics: [],
    })
  })

  it('surfaces package-resolution failures as compiler diagnostics', async () => {
    projectState.getCompileBundle.mockResolvedValue({
      mainPath: '/main.typ',
      mainSource: '@preview/example:1.0.0\nHello',
      extraFiles: [],
      extraBinaryFiles: [],
    })
    ensurePackagesForCompileMock.mockRejectedValue(new Error('network down'))

    const { compileManager, useCompileStore } = await loadHarness()

    await compileManager.forceCompile('live source', 'main.typ')

    expect(compileTypstMock).not.toHaveBeenCalled()
    expect(useCompileStore.getState().status).toBe('error')
    expect(useCompileStore.getState().diagnostics[0]?.message).toContain('Failed to resolve package dependencies: network down')
  })

  it('re-hydrates packages after the compiler runtime changes', async () => {
    projectState.getCompileBundle.mockResolvedValue({
      mainPath: '/main.typ',
      mainSource: '@preview/rendercv:0.2.0\nHello',
      extraFiles: [],
      extraBinaryFiles: [],
    })
    getPackageRuntimeEpochMock.mockReturnValueOnce(0).mockReturnValueOnce(1)

    const { compileManager } = await loadHarness()

    await compileManager.forceCompile('live source', 'main.typ')
    await compileManager.forceCompile('live source', 'main.typ')

    expect(ensurePackagesForCompileMock).toHaveBeenCalledTimes(2)
    expect(ensurePackagesForCompileMock).toHaveBeenNthCalledWith(1, ['@preview/rendercv:0.2.0'])
    expect(ensurePackagesForCompileMock).toHaveBeenNthCalledWith(2, ['@preview/rendercv:0.2.0'])
  })

  it('debounces scheduled compiles when auto-compile is enabled', async () => {
    vi.useFakeTimers()
    projectState.getCompileBundle.mockResolvedValue({
      mainPath: '/main.typ',
      mainSource: 'Hello',
      extraFiles: [],
      extraBinaryFiles: [],
    })

    const { compileManager, useSettingsStore } = await loadHarness()
    useSettingsStore.setState({ autoCompile: true, compileDelay: 150 })

    compileManager.requestCompile('queued', 'main.typ')
    expect(compileTypstMock).not.toHaveBeenCalled()

    await vi.advanceTimersByTimeAsync(149)
    expect(compileTypstMock).not.toHaveBeenCalled()

    await vi.advanceTimersByTimeAsync(1)
    expect(compileTypstMock).toHaveBeenCalledTimes(1)
  })
})
