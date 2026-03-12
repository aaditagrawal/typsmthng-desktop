import { beforeEach, describe, expect, it, vi } from 'vitest'

const wrapMock = vi.fn()
const initCompilerBackendMock = vi.fn()
const compileTypstBackendMock = vi.fn()
const configureCompilerBackendMock = vi.fn()
const resolveSourceLocBackendMock = vi.fn()
const resolveSourceLocBatchBackendMock = vi.fn()
const compileToPdfBackendMock = vi.fn()
const ensurePackagesForCompileBackendMock = vi.fn()
const isCompilerReadyBackendMock = vi.fn(() => false)
const settingsState = {
  systemFontsEnabled: false,
}

vi.mock('comlink', () => ({
  wrap: wrapMock,
}))

vi.mock('@/stores/settings-store', () => ({
  useSettingsStore: {
    getState: () => settingsState,
  },
}))

vi.mock('./compiler-backend', () => ({
  initCompilerBackend: initCompilerBackendMock,
  compileTypstBackend: compileTypstBackendMock,
  configureCompilerBackend: configureCompilerBackendMock,
  resolveSourceLocBackend: resolveSourceLocBackendMock,
  resolveSourceLocBatchBackend: resolveSourceLocBatchBackendMock,
  compileToPdfBackend: compileToPdfBackendMock,
  ensurePackagesForCompileBackend: ensurePackagesForCompileBackendMock,
  isCompilerReadyBackend: isCompilerReadyBackendMock,
}))

function installWindow(): void {
  Object.defineProperty(globalThis, 'window', {
    configurable: true,
    value: {
        localStorage: {
          getItem: vi.fn(() => null),
        },
        queryLocalFonts: undefined,
      },
    })
}

async function loadModule() {
  vi.resetModules()
  return import('./compiler-client')
}

describe('compiler-client', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    vi.spyOn(console, 'warn').mockImplementation(() => {})
    settingsState.systemFontsEnabled = false
    installWindow()
    Reflect.deleteProperty(globalThis, 'Worker')
    initCompilerBackendMock.mockResolvedValue(undefined)
    compileTypstBackendMock.mockResolvedValue({
      svg: '<svg />',
      vectorData: new Uint8Array([1]),
      pageDimensions: [{ width: 1, height: 1 }],
      diagnostics: [],
      success: true,
    })
    resolveSourceLocBackendMock.mockResolvedValue('main.typ:1:1')
    resolveSourceLocBatchBackendMock.mockResolvedValue(['a', 'b'])
    compileToPdfBackendMock.mockResolvedValue(new Uint8Array([7]))
    ensurePackagesForCompileBackendMock.mockResolvedValue(undefined)
    isCompilerReadyBackendMock.mockReturnValue(false)
  })

  it('falls back to the backend when workers are unavailable', async () => {
    const mod = await loadModule()

    await mod.initCompilerClient()
    const result = await mod.compileTypstClient('= Title')

    expect(initCompilerBackendMock).toHaveBeenCalledTimes(1)
    expect(compileTypstBackendMock).toHaveBeenCalledWith('= Title', undefined, '/main.typ', undefined)
    expect(result.success).toBe(true)
    expect(mod.isCompilerReadyClient()).toBe(true)
  })

  it('uses the worker transport when available', async () => {
    const workerApi = {
      initCompiler: vi.fn().mockResolvedValue(undefined),
      compileTypst: vi.fn().mockResolvedValue({
        svg: '<svg>worker</svg>',
        vectorData: new Uint8Array([3]),
        pageDimensions: [{ width: 320, height: 240 }],
        diagnostics: [],
        success: true,
      }),
      resolveSourceLoc: vi.fn().mockResolvedValue('worker-loc'),
      resolveSourceLocBatch: vi.fn().mockResolvedValue(['worker-a']),
      compileToPdf: vi.fn().mockResolvedValue(new Uint8Array([5])),
      ensurePackagesForCompile: vi.fn().mockResolvedValue(undefined),
      isCompilerReady: vi.fn().mockReturnValue(true),
    }
    class MockWorker {
      terminate = vi.fn()
    }

    Object.defineProperty(globalThis, 'Worker', {
      configurable: true,
      value: MockWorker,
    })
    wrapMock.mockReturnValue(workerApi)

    const mod = await loadModule()
    const result = await mod.compileTypstClient('worker-source')

    expect(workerApi.initCompiler).toHaveBeenCalledWith({ systemFontData: [] })
    expect(workerApi.compileTypst).toHaveBeenCalledWith('worker-source', undefined, '/main.typ', undefined)
    expect(compileTypstBackendMock).not.toHaveBeenCalled()
    expect(result.svg).toBe('<svg>worker</svg>')
  })

  it('loads matching local fonts when system font support is enabled', async () => {
    settingsState.systemFontsEnabled = true
    Object.defineProperty(globalThis, 'window', {
      configurable: true,
      value: {
        localStorage: {
          getItem: vi.fn(() => null),
        },
        queryLocalFonts: vi.fn().mockResolvedValue([
          {
            family: 'SF Pro Text',
            fullName: 'SF Pro Text Regular',
            postscriptName: 'SFProText-Regular',
            style: 'Regular',
            blob: vi.fn().mockResolvedValue(new Blob([new Uint8Array([1, 2, 3])])),
          },
          {
            family: 'Helvetica',
            fullName: 'Helvetica Regular',
            postscriptName: 'Helvetica-Regular',
            style: 'Regular',
            blob: vi.fn().mockResolvedValue(new Blob([new Uint8Array([4, 5, 6])])),
          },
        ]),
      },
    })

    const mod = await loadModule()
    await mod.compileTypstClient('#set text(font: "SF Pro Text")\nHello')

    expect(configureCompilerBackendMock).toHaveBeenCalledWith({
      systemFontData: [new Uint8Array([1, 2, 3])],
    })
    expect(initCompilerBackendMock).toHaveBeenCalledTimes(1)
    expect(compileTypstBackendMock).toHaveBeenCalled()
  })

  it('terminates a failing worker and falls back to backend calls', async () => {
    let lastWorker: { terminate: ReturnType<typeof vi.fn> } | null = null
    class MockWorker {
      terminate = vi.fn()

      constructor() {
        lastWorker = this
      }
    }
    const workerApi = {
      initCompiler: vi.fn().mockResolvedValue(undefined),
      compileTypst: vi.fn().mockResolvedValue({
        svg: '<svg />',
        vectorData: new Uint8Array([1]),
        pageDimensions: [{ width: 1, height: 1 }],
        diagnostics: [],
        success: true,
      }),
      resolveSourceLoc: vi.fn().mockResolvedValue(undefined),
      resolveSourceLocBatch: vi.fn().mockResolvedValue([]),
      compileToPdf: vi.fn().mockRejectedValue(new Error('worker exploded')),
      ensurePackagesForCompile: vi.fn().mockResolvedValue(undefined),
      isCompilerReady: vi.fn().mockReturnValue(true),
    }

    Object.defineProperty(globalThis, 'Worker', {
      configurable: true,
      value: MockWorker,
    })
    wrapMock.mockReturnValue(workerApi)

    const mod = await loadModule()
    const pdf = await mod.compileToPdfClient('pdf-source')

    expect(workerApi.compileToPdf).toHaveBeenCalled()
    expect(initCompilerBackendMock).toHaveBeenCalledTimes(1)
    expect((lastWorker as { terminate: ReturnType<typeof vi.fn> } | null)?.terminate).toHaveBeenCalledTimes(1)
    expect(compileToPdfBackendMock).toHaveBeenCalledWith('pdf-source', undefined, '/main.typ', undefined)
    expect(pdf).toEqual(new Uint8Array([7]))
  })

  it('honors the local override that disables worker usage', async () => {
    Object.defineProperty(globalThis, 'window', {
      configurable: true,
      value: {
        localStorage: {
          getItem: vi.fn(() => '1'),
        },
      },
    })
    Object.defineProperty(globalThis, 'Worker', {
      configurable: true,
      value: class MockWorker {},
    })

    const mod = await loadModule()
    await mod.ensurePackagesForCompileClient(['@preview/example:1.0.0'])

    expect(wrapMock).not.toHaveBeenCalled()
    expect(ensurePackagesForCompileBackendMock).toHaveBeenCalledWith(['@preview/example:1.0.0'])
  })
})
