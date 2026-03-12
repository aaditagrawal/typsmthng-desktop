import {
  createTypstCompiler,
  createTypstRenderer,
  initOptions,
  loadFonts,
  MemoryAccessModel,
} from '@myriaddreamin/typst.ts'
import compilerWasmUrl from '@myriaddreamin/typst-ts-web-compiler/pkg/typst_ts_web_compiler_bg.wasm?url'
import rendererWasmUrl from '@myriaddreamin/typst-ts-renderer/pkg/typst_ts_renderer_bg.wasm?url'
import type { Diagnostic } from '@/stores/compile-store'
import { getPreparedPackageForResolver, ensurePackagesForCompile as ensurePackagesForCompileRegistry } from './universe-registry'

let compiler: Awaited<ReturnType<typeof createTypstCompiler>> | null = null
let renderer: Awaited<ReturnType<typeof createTypstRenderer>> | null = null
let initPromise: Promise<void> | null = null
const PROJECT_ROOT = '/'
const packageAccessModel = new MemoryAccessModel()
const insertedPackageRoots = new Set<string>()
let additionalFontData: Uint8Array[] = []

function sameFontData(next: Uint8Array[]): boolean {
  if (additionalFontData.length !== next.length) return false
  for (let i = 0; i < additionalFontData.length; i++) {
    if (additionalFontData[i] !== next[i]) return false
  }
  return true
}

export function configureCompilerBackend(options?: { fontData?: Uint8Array[] }): void {
  const nextFontData = options?.fontData ?? []
  if (sameFontData(nextFontData)) return

  additionalFontData = nextFontData
  compiler = null
  renderer = null
  initPromise = null
}

function normalizeWorkspacePath(input: string): string {
  const normalized = input.replace(/\\/g, '/').replace(/^\/+/, '')
  return normalized ? `/${normalized}` : PROJECT_ROOT
}

async function loadWasmModule(url: string): Promise<ArrayBuffer> {
  return new Promise<ArrayBuffer>((resolve, reject) => {
    const request = new XMLHttpRequest()
    request.open('GET', url, true)
    request.responseType = 'arraybuffer'

    request.onload = () => {
      const okStatus = request.status === 200 || request.status === 0
      const response = request.response
      if (okStatus && response instanceof ArrayBuffer) {
        resolve(response)
        return
      }

      reject(new Error(`Failed to load wasm module: ${request.status}`))
    }

    request.onerror = () => {
      reject(new Error(`Failed to load wasm module: ${request.status}`))
    }

    request.send()
  })
}

function decodeVersion(version: unknown): string {
  if (version === undefined || version === null) return ''
  if (typeof version === 'string') return version

  if (version && typeof version === 'object') {
    const obj = version as Record<string, unknown>
    if (
      typeof obj.major === 'number'
      && typeof obj.minor === 'number'
      && typeof obj.patch === 'number'
    ) {
      return `${obj.major}.${obj.minor}.${obj.patch}`
    }
  }

  return ''
}

function packageRootPath(namespace: string, name: string, version: string): string {
  return `/@memory/fetch/packages/${namespace}/${name}/${version}`
}

function ensurePackageInAccessModel(spec: unknown): string | undefined {
  if (!spec || typeof spec !== 'object') return undefined

  const raw = spec as Record<string, unknown>
  const namespace = String(raw.namespace ?? '')
  const name = String(raw.name ?? '')
  const version = decodeVersion(raw.version)

  if (!namespace || !name || !version) return undefined

  const prepared = getPreparedPackageForResolver({
    namespace,
    name,
    version,
  })
  if (!prepared) return undefined

  const root = packageRootPath(namespace, name, version)
  const rootKey = `${namespace}/${name}/${version}`
  if (!insertedPackageRoots.has(rootKey)) {
    for (const file of prepared.files) {
      packageAccessModel.insertFile(
        `${root}/${file.path}`,
        file.data,
        new Date((file.mtime || Math.floor(Date.now() / 1000)) * 1000),
      )
    }
    insertedPackageRoots.add(rootKey)
  }

  return root
}

export async function initCompilerBackend(): Promise<void> {
  if (initPromise) return initPromise

  initPromise = (async () => {
    try {
      compiler = createTypstCompiler()
      await compiler.init({
        getModule: () => loadWasmModule(compilerWasmUrl),
        beforeBuild: [
          loadFonts(additionalFontData, { assets: ['text'] }),
          initOptions.withAccessModel(packageAccessModel as never),
          initOptions.withPackageRegistry({
            resolve: (spec: unknown) => ensurePackageInAccessModel(spec),
          } as never),
        ],
      })

      renderer = createTypstRenderer()
      await renderer.init({
        getModule: () => loadWasmModule(rendererWasmUrl),
      })
    } catch (err) {
      console.error('Failed to initialize compiler:', err)
      compiler = null
      renderer = null
      initPromise = null
      throw err
    }
  })()

  return initPromise
}

export interface PageDimension {
  width: number
  height: number
}

export interface CompileTimings {
  compileMs: number
  renderMs: number
  totalMs: number
}

export interface CompileResult {
  svg: string | null
  vectorData: Uint8Array | null
  pageDimensions: PageDimension[]
  diagnostics: Diagnostic[]
  success: boolean
  timings?: CompileTimings
}

export interface LivePreviewController {
  dispose: () => void
  refresh: () => void
}

interface DesktopTypstRendererDriver {
  renderDom: (options: {
    format: 'vector'
    artifactContent: Uint8Array
    container: HTMLElement
    pixelPerPt?: number
  }) => Promise<{
    dispose?: () => void
    addViewportChange?: () => void
  }>
}

export async function compileTypstBackend(
  source: string,
  extraFiles?: Array<{ path: string; content: string }>,
  mainFilePath = '/main.typ',
  extraBinaryFiles?: Array<{ path: string; data: Uint8Array }>,
): Promise<CompileResult> {
  if (!compiler || !renderer) {
    throw new Error('Compiler not initialized')
  }

  const totalStart = performance.now()
  const normalizedMainFilePath = normalizeWorkspacePath(mainFilePath)

  compiler.resetShadow()
  compiler.addSource(normalizedMainFilePath, source)

  if (extraFiles) {
    for (const file of extraFiles) {
      compiler.addSource(normalizeWorkspacePath(file.path), file.content)
    }
  }
  if (extraBinaryFiles) {
    for (const file of extraBinaryFiles) {
      compiler.mapShadow(normalizeWorkspacePath(file.path), file.data)
    }
  }

  const compileStart = performance.now()
  const { result: vectorData, diagnostics: rawDiags } = await compiler.compile({
    mainFilePath: normalizedMainFilePath,
    root: PROJECT_ROOT,
    diagnostics: 'full',
  })
  const compileMs = performance.now() - compileStart

  const diagnostics: Diagnostic[] = (rawDiags ?? []).map((d: unknown) => {
    const diag = d as Record<string, unknown>
    return {
      severity: String(diag.severity || 'error') as Diagnostic['severity'],
      path: String(diag.path || ''),
      range: String(diag.range || ''),
      message: String(diag.message || ''),
      package: diag.package ? String(diag.package) : undefined,
    }
  })

  if (!vectorData) {
    return {
      svg: null,
      vectorData: null,
      pageDimensions: [],
      diagnostics,
      success: false,
      timings: {
        compileMs,
        renderMs: 0,
        totalMs: performance.now() - totalStart,
      },
    }
  }

  let svg: string | null = null
  let pageDimensions: PageDimension[] = []

  const renderStart = performance.now()
  await renderer.runWithSession(
    { format: 'vector', artifactContent: vectorData },
    async (session) => {
      const pagesInfo = session.retrievePagesInfo()
      svg = await session.renderSvg({})

      pageDimensions = pagesInfo.map((page) => ({
        width: page.width,
        height: page.height,
      }))
    },
  )
  const renderMs = performance.now() - renderStart

  return {
    svg,
    vectorData,
    pageDimensions,
    diagnostics,
    success: true,
    timings: {
      compileMs,
      renderMs,
      totalMs: performance.now() - totalStart,
    },
  }
}

export async function resolveSourceLocBackend(
  vectorData: Uint8Array,
  path: Uint32Array,
): Promise<string | undefined> {
  if (!renderer) return undefined

  let loc: string | undefined
  await renderer.runWithSession(
    { format: 'vector', artifactContent: vectorData },
    async (session) => {
      loc = session.getSourceLoc(path)
    },
  )
  return loc
}

export async function resolveSourceLocBatchBackend(
  vectorData: Uint8Array,
  paths: Uint32Array[],
): Promise<Array<string | undefined>> {
  if (!renderer || paths.length === 0) return []

  const locs: Array<string | undefined> = new Array(paths.length).fill(undefined)
  await renderer.runWithSession(
    { format: 'vector', artifactContent: vectorData },
    async (session) => {
      for (let i = 0; i < paths.length; i++) {
        try {
          locs[i] = session.getSourceLoc(paths[i])
        } catch {
          locs[i] = undefined
        }
      }
    },
  )
  return locs
}

export async function mountLivePreviewBackend(
  vectorData: Uint8Array,
  container: HTMLElement,
  options?: { pixelPerPt?: number },
): Promise<LivePreviewController> {
  if (!renderer) {
    throw new Error('Compiler not initialized')
  }

  const driver = renderer as unknown as DesktopTypstRendererDriver
  if (typeof driver.renderDom !== 'function') {
    throw new Error('Live preview renderer is unavailable')
  }

  const view = await driver.renderDom({
    format: 'vector',
    artifactContent: vectorData,
    container,
    pixelPerPt: options?.pixelPerPt ?? 2,
  })

  return {
    dispose: () => {
      view.dispose?.()
    },
    refresh: () => {
      view.addViewportChange?.()
    },
  }
}

export async function compileToPdfBackend(
  source: string,
  extraFiles?: Array<{ path: string; content: string }>,
  mainFilePath = '/main.typ',
  extraBinaryFiles?: Array<{ path: string; data: Uint8Array }>,
): Promise<Uint8Array | null> {
  if (!compiler) {
    throw new Error('Compiler not initialized')
  }

  const normalizedMainFilePath = normalizeWorkspacePath(mainFilePath)
  compiler.resetShadow()
  compiler.addSource(normalizedMainFilePath, source)
  if (extraFiles) {
    for (const file of extraFiles) {
      compiler.addSource(normalizeWorkspacePath(file.path), file.content)
    }
  }
  if (extraBinaryFiles) {
    for (const file of extraBinaryFiles) {
      compiler.mapShadow(normalizeWorkspacePath(file.path), file.data)
    }
  }

  const { result } = await compiler.compile({
    mainFilePath: normalizedMainFilePath,
    root: PROJECT_ROOT,
    format: 1,
    diagnostics: 'none',
  })

  return result ?? null
}

export async function ensurePackagesForCompileBackend(specs: string[]): Promise<void> {
  await ensurePackagesForCompileRegistry(specs)
}

export function isCompilerReadyBackend(): boolean {
  return compiler !== null && renderer !== null
}
