import { beforeEach, describe, expect, it, vi } from 'vitest'

const compilerMock = {
  init: vi.fn(),
  resetShadow: vi.fn(),
  addSource: vi.fn(),
  mapShadow: vi.fn(),
  compile: vi.fn(),
}

const rendererMock = {
  init: vi.fn(),
  runWithSession: vi.fn(),
}

const createTypstCompilerMock = vi.fn(() => compilerMock)
const createTypstRendererMock = vi.fn(() => rendererMock)
const loadFontsMock = vi.fn(() => 'fonts-loader')
const withAccessModelMock = vi.fn(() => 'access-model')
const withPackageRegistryMock = vi.fn((registry: unknown) => registry)
const ensurePackagesForCompileRegistryMock = vi.fn()
const getPreparedPackageForResolverMock = vi.fn(() => null)

vi.mock('@myriaddreamin/typst.ts', () => ({
  createTypstCompiler: createTypstCompilerMock,
  createTypstRenderer: createTypstRendererMock,
  loadFonts: loadFontsMock,
  MemoryAccessModel: class {
    insertFile = vi.fn()
  },
  initOptions: {
    withAccessModel: withAccessModelMock,
    withPackageRegistry: withPackageRegistryMock,
  },
}))

vi.mock('./universe-registry', () => ({
  ensurePackagesForCompile: ensurePackagesForCompileRegistryMock,
  getPreparedPackageForResolver: getPreparedPackageForResolverMock,
}))

vi.mock('@myriaddreamin/typst-ts-web-compiler/pkg/typst_ts_web_compiler_bg.wasm?url', () => ({
  default: '/compiler.wasm',
}))

vi.mock('@myriaddreamin/typst-ts-renderer/pkg/typst_ts_renderer_bg.wasm?url', () => ({
  default: '/renderer.wasm',
}))

class MockXMLHttpRequest {
  static requestedUrls: string[] = []

  responseType = ''
  response: ArrayBuffer | null = null
  status = 200
  onload: (() => void) | null = null
  onerror: (() => void) | null = null

  open(_method: string, url: string): void {
    MockXMLHttpRequest.requestedUrls.push(url)
  }

  send(): void {
    this.response = new Uint8Array([1, 2, 3]).buffer
    this.onload?.()
  }
}

async function loadModule() {
  vi.resetModules()
  return import('./compiler-backend')
}

describe('compiler-backend', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    MockXMLHttpRequest.requestedUrls = []
    globalThis.XMLHttpRequest = MockXMLHttpRequest as unknown as typeof XMLHttpRequest

    compilerMock.init.mockImplementation(async (options?: { getModule?: () => Promise<ArrayBuffer> }) => {
      await options?.getModule?.()
    })
    rendererMock.init.mockImplementation(async (options?: { getModule?: () => Promise<ArrayBuffer> }) => {
      await options?.getModule?.()
    })
    rendererMock.runWithSession.mockImplementation(async (_artifact, callback) => callback({
      retrievePagesInfo: () => [{ width: 800, height: 600 }],
      renderSvg: async () => '<svg>compiled</svg>',
      getSourceLoc: (path: Uint32Array) => `loc:${Array.from(path).join('.')}`,
    }))
  })

  it('loads wasm modules during init without relying on MIME streaming', async () => {
    const mod = await loadModule()

    await mod.initCompilerBackend()

    expect(createTypstCompilerMock).toHaveBeenCalledTimes(1)
    expect(createTypstRendererMock).toHaveBeenCalledTimes(1)
    expect(MockXMLHttpRequest.requestedUrls).toEqual(['/compiler.wasm', '/renderer.wasm'])
    expect(loadFontsMock).toHaveBeenCalled()
    expect(withAccessModelMock).toHaveBeenCalled()
    expect(withPackageRegistryMock).toHaveBeenCalled()
  })

  it('includes configured external fonts in the compiler font loader', async () => {
    const mod = await loadModule()
    const localFont = new Uint8Array([8, 6, 7, 5, 3, 0, 9])

    mod.configureCompilerBackend({ fontData: [localFont] })
    await mod.initCompilerBackend()

    expect(loadFontsMock).toHaveBeenCalledWith([localFont], { assets: ['text'] })
  })

  it('compiles typst, maps diagnostics, and renders svg output', async () => {
    const mod = await loadModule()
    compilerMock.compile.mockResolvedValue({
      result: new Uint8Array([9, 8, 7]),
      diagnostics: [
        {
          severity: 'warning',
          path: 'main.typ',
          range: '1:1-1:4',
          message: 'warn',
          package: '@preview/test:1.0.0',
        },
      ],
    })

    await mod.initCompilerBackend()
    const result = await mod.compileTypstBackend(
      '= Title',
      [{ path: '/chapter.typ', content: 'Body' }],
      '/main.typ',
      [{ path: '/assets/logo.png', data: new Uint8Array([4, 5]) }],
    )

    expect(compilerMock.resetShadow).toHaveBeenCalled()
    expect(compilerMock.addSource).toHaveBeenCalledWith('/main.typ', '= Title')
    expect(compilerMock.addSource).toHaveBeenCalledWith('/chapter.typ', 'Body')
    expect(compilerMock.mapShadow).toHaveBeenCalledWith('/assets/logo.png', new Uint8Array([4, 5]))
    expect(result.success).toBe(true)
    expect(result.svg).toBe('<svg>compiled</svg>')
    expect(result.pageDimensions).toEqual([{ width: 800, height: 600 }])
    expect(result.diagnostics).toEqual([
      {
        severity: 'warning',
        path: 'main.typ',
        range: '1:1-1:4',
        message: 'warn',
        package: '@preview/test:1.0.0',
      },
    ])
  })

  it('returns a failed compile result when the compiler produces no artifact', async () => {
    const mod = await loadModule()
    compilerMock.compile.mockResolvedValue({
      result: null,
      diagnostics: [{ severity: 'error', path: 'main.typ', range: '1:1', message: 'bad input' }],
    })

    await mod.initCompilerBackend()
    const result = await mod.compileTypstBackend('broken')

    expect(result.success).toBe(false)
    expect(result.svg).toBeNull()
    expect(result.vectorData).toBeNull()
    expect(rendererMock.runWithSession).not.toHaveBeenCalled()
    expect(result.diagnostics[0]?.message).toBe('bad input')
  })

  it('resolves source locations in single and batch mode and exports pdf data', async () => {
    const mod = await loadModule()
    compilerMock.compile.mockResolvedValue({
      result: new Uint8Array([1, 3, 3, 7]),
      diagnostics: [],
    })

    await mod.initCompilerBackend()

    const single = await mod.resolveSourceLocBackend(new Uint8Array([1]), new Uint32Array([4, 2]))
    const batch = await mod.resolveSourceLocBatchBackend(
      new Uint8Array([1]),
      [new Uint32Array([1]), new Uint32Array([2, 3])],
    )
    const pdf = await mod.compileToPdfBackend('= PDF')

    expect(single).toBe('loc:4.2')
    expect(batch).toEqual(['loc:1', 'loc:2.3'])
    expect(pdf).toEqual(new Uint8Array([1, 3, 3, 7]))
    expect(compilerMock.compile).toHaveBeenLastCalledWith({
      mainFilePath: '/main.typ',
      root: '/',
      format: 1,
      diagnostics: 'none',
    })
  })

  it('normalizes relative workspace paths before compiling', async () => {
    const mod = await loadModule()
    compilerMock.compile.mockResolvedValue({
      result: new Uint8Array([6, 6]),
      diagnostics: [],
    })

    await mod.initCompilerBackend()
    await mod.compileTypstBackend(
      'relative',
      [{ path: 'nested/chapter.typ', content: 'chapter' }],
      'main.typ',
      [{ path: 'assets/logo.png', data: new Uint8Array([1]) }],
    )

    expect(compilerMock.addSource).toHaveBeenCalledWith('/main.typ', 'relative')
    expect(compilerMock.addSource).toHaveBeenCalledWith('/nested/chapter.typ', 'chapter')
    expect(compilerMock.mapShadow).toHaveBeenCalledWith('/assets/logo.png', new Uint8Array([1]))
    expect(compilerMock.compile).toHaveBeenLastCalledWith({
      mainFilePath: '/main.typ',
      root: '/',
      diagnostics: 'full',
    })
  })

  it('forwards package hydration requests to the registry service', async () => {
    const mod = await loadModule()

    await mod.ensurePackagesForCompileBackend(['@preview/example:1.0.0'])

    expect(ensurePackagesForCompileRegistryMock).toHaveBeenCalledWith(['@preview/example:1.0.0'])
  })
})
