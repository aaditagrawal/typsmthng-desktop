import { wrap } from 'comlink'
import type { Remote } from 'comlink'
import { useSettingsStore } from '@/stores/settings-store'
import { loadDeclaredFontData } from './declared-fonts'
import {
  compileToPdfBackend,
  compileTypstBackend,
  configureCompilerBackend,
  ensurePackagesForCompileBackend,
  initCompilerBackend,
  isCompilerReadyBackend,
  mountLivePreviewBackend,
  resolveSourceLocBackend,
  resolveSourceLocBatchBackend,
  type CompileResult,
  type LivePreviewController,
} from './compiler-backend'

interface CompilerInitOptions {
  fontData?: Uint8Array[]
}

interface CompilerWorkerApi {
  initCompiler: (options?: CompilerInitOptions) => Promise<void>
  compileTypst: (
    source: string,
    extraFiles?: Array<{ path: string; content: string }>,
    mainFilePath?: string,
    extraBinaryFiles?: Array<{ path: string; data: Uint8Array }>,
  ) => Promise<CompileResult>
  resolveSourceLoc: (vectorData: Uint8Array, path: Uint32Array) => Promise<string | undefined>
  resolveSourceLocBatch: (vectorData: Uint8Array, paths: Uint32Array[]) => Promise<Array<string | undefined>>
  compileToPdf: (
    source: string,
    extraFiles?: Array<{ path: string; content: string }>,
    mainFilePath?: string,
    extraBinaryFiles?: Array<{ path: string; data: Uint8Array }>,
  ) => Promise<Uint8Array | null>
  ensurePackagesForCompile: (specs: string[]) => Promise<void>
  isCompilerReady: () => boolean
}

let worker: Worker | null = null
let workerApi: Remote<CompilerWorkerApi> | null = null
let workerAvailable = typeof Worker !== 'undefined'
let workerFailedPromise: Promise<never> | null = null
let workerFailedReject: ((err: Error) => void) | null = null
let workerInitPromise: Promise<void> | null = null
let consecutiveWorkerFailures = 0
let compilerReady = false
let backendInitPromise: Promise<void> | null = null
let currentCompilerConfigKey = ''
let currentFontData: Uint8Array[] = []
let packageRuntimeEpoch = 0
let lastEnsuredSpecs: string[] = []
const WORKER_INIT_TIMEOUT_MS = 20_000
const WORKER_CALL_TIMEOUT_MS = 300_000
const MAX_CONSECUTIVE_WORKER_FAILURES = 3

function resetWorkerTransport(): void {
  if (worker || workerApi) {
    packageRuntimeEpoch += 1
  }
  // Settle any in-flight comlink promises: a terminated worker never
  // responds, so without this the callers racing workerFailedPromise
  // would await forever.
  workerFailedReject?.(new Error('typst worker terminated'))
  if (worker) {
    worker.terminate()
    worker = null
  }
  workerApi = null
  workerFailedPromise = null
  workerFailedReject = null
  workerInitPromise = null
}

async function ensureCompilerConfig(
  source?: string,
  extraFiles?: Array<{ path: string; content: string }>,
): Promise<void> {
  const { systemFontsEnabled, googleFontsEnabled } = useSettingsStore.getState()
  const { key, data } = source
    ? await loadDeclaredFontData(source, extraFiles, {
      systemFontsEnabled,
      googleFontsEnabled,
    })
    : { key: '', data: [] as Uint8Array[] }

  if (key === currentCompilerConfigKey) return

  currentCompilerConfigKey = key
  currentFontData = data
  compilerReady = false
  backendInitPromise = null
  configureCompilerBackend({ fontData: currentFontData })
  resetWorkerTransport()
}

function shouldDisableWorker(): boolean {
  if (typeof window === 'undefined') return false
  try {
    return window.localStorage.getItem('typst_worker_disabled') === '1'
  } catch {
    return false
  }
}

async function getWorkerApi(): Promise<Remote<CompilerWorkerApi> | null> {
  if (!workerAvailable || shouldDisableWorker()) return null
  if (workerApi) return workerApi

  try {
    worker = new Worker(new URL('../workers/typst-worker.ts', import.meta.url), { type: 'module' })
    workerFailedPromise = new Promise<never>((_, reject) => {
      workerFailedReject = reject
      worker?.addEventListener('error', () => {
        reject(new Error('typst worker failed to load'))
      })
      worker?.addEventListener('messageerror', () => {
        reject(new Error('typst worker message error'))
      })
    })
    workerFailedPromise.catch(() => {})
    workerApi = wrap<CompilerWorkerApi>(worker)
    return workerApi
  } catch (err) {
    console.warn('Falling back to main-thread compiler (worker init failed):', err)
    workerAvailable = false
    resetWorkerTransport()
    return null
  }
}

/**
 * Race a worker call against worker death and a safety timeout. A worker
 * killed mid-call (WASM OOM/abort) fires the `error` event instead of
 * rejecting the comlink promise, which would otherwise hang the compile
 * pipeline (and its lock) forever.
 */
async function raceWorkerCall<T>(call: Promise<T>, timeoutMs: number): Promise<T> {
  const failed = workerFailedPromise
  let timer: ReturnType<typeof setTimeout> | undefined
  const timeout = new Promise<never>((_, reject) => {
    timer = setTimeout(() => reject(new Error('typst worker call timed out')), timeoutMs)
  })
  try {
    return await Promise.race(failed ? [call, failed, timeout] : [call, timeout])
  } finally {
    clearTimeout(timer)
  }
}

/** Per-worker init: a fresh worker (after a transport reset) must re-init before use. */
async function ensureWorkerInitialized(api: Remote<CompilerWorkerApi>): Promise<void> {
  if (!workerInitPromise) {
    workerInitPromise = raceWorkerCall(
      Promise.resolve(api.initCompiler({ fontData: currentFontData })),
      WORKER_INIT_TIMEOUT_MS,
    ).catch((err: unknown) => {
      workerInitPromise = null
      throw err
    })
  }
  await workerInitPromise
}

async function ensureBackendInitialized(): Promise<void> {
  if (isCompilerReadyBackend()) return
  if (!backendInitPromise) {
    backendInitPromise = initCompilerBackend().catch((err) => {
      backendInitPromise = null
      throw err
    })
  }
  await backendInitPromise
}

async function callWithFallback<T>(
  runWorker: (api: Remote<CompilerWorkerApi>) => Promise<T>,
  runFallback: () => Promise<T>,
): Promise<T> {
  const api = await getWorkerApi()
  if (!api) return runFallback()

  try {
    await ensureWorkerInitialized(api)
    const result = await raceWorkerCall(runWorker(api), WORKER_CALL_TIMEOUT_MS)
    consecutiveWorkerFailures = 0
    return result
  } catch (err) {
    console.warn('Worker compiler call failed, using fallback path:', err)
    consecutiveWorkerFailures += 1
    if (consecutiveWorkerFailures >= MAX_CONSECUTIVE_WORKER_FAILURES) {
      console.warn(
        `Disabling compiler worker after ${consecutiveWorkerFailures} consecutive failures; compiles stay on the main thread this session.`,
      )
      workerAvailable = false
    }
    resetWorkerTransport()
    return runFallback()
  }
}

async function callWithCompilerFallback<T>(
  runWorker: (api: Remote<CompilerWorkerApi>) => Promise<T>,
  runFallback: () => Promise<T>,
): Promise<T> {
  return callWithFallback(runWorker, async () => {
    await ensureBackendInitialized()
    if (lastEnsuredSpecs.length > 0) {
      await ensurePackagesForCompileBackend(lastEnsuredSpecs)
    }
    return runFallback()
  })
}

export async function initCompilerClient(
  source?: string,
  extraFiles?: Array<{ path: string; content: string }>,
): Promise<void> {
  if (source) {
    await ensureCompilerConfig(source, extraFiles)
  }
  if (compilerReady) return

  await callWithFallback(
    async () => {
      // ensureWorkerInitialized already ran inside callWithFallback.
      compilerReady = true
    },
    async () => {
      await ensureBackendInitialized()
      compilerReady = true
    },
  )
}

export async function compileTypstClient(
  source: string,
  extraFiles?: Array<{ path: string; content: string }>,
  mainFilePath = '/main.typ',
  extraBinaryFiles?: Array<{ path: string; data: Uint8Array }>,
): Promise<CompileResult> {
  await ensureCompilerConfig(source, extraFiles)
  if (!compilerReady) {
    await initCompilerClient(source, extraFiles)
  }

  return callWithCompilerFallback(
    (api) => api.compileTypst(source, extraFiles, mainFilePath, extraBinaryFiles),
    () => compileTypstBackend(source, extraFiles, mainFilePath, extraBinaryFiles),
  )
}

export async function resolveSourceLocClient(
  vectorData: Uint8Array,
  path: Uint32Array,
): Promise<string | undefined> {
  return callWithCompilerFallback(
    (api) => api.resolveSourceLoc(vectorData, path),
    () => resolveSourceLocBackend(vectorData, path),
  )
}

export async function resolveSourceLocBatchClient(
  vectorData: Uint8Array,
  paths: Uint32Array[],
): Promise<Array<string | undefined>> {
  return callWithCompilerFallback(
    (api) => api.resolveSourceLocBatch(vectorData, paths),
    () => resolveSourceLocBatchBackend(vectorData, paths),
  )
}

export async function compileToPdfClient(
  source: string,
  extraFiles?: Array<{ path: string; content: string }>,
  mainFilePath = '/main.typ',
  extraBinaryFiles?: Array<{ path: string; data: Uint8Array }>,
): Promise<Uint8Array | null> {
  await ensureCompilerConfig(source, extraFiles)
  if (!compilerReady) {
    await initCompilerClient(source, extraFiles)
  }

  return callWithCompilerFallback(
    (api) => api.compileToPdf(source, extraFiles, mainFilePath, extraBinaryFiles),
    () => compileToPdfBackend(source, extraFiles, mainFilePath, extraBinaryFiles),
  )
}

export async function ensurePackagesForCompileClient(specs: string[]): Promise<void> {
  lastEnsuredSpecs = specs
  await callWithFallback(
    (api) => api.ensurePackagesForCompile(specs),
    () => ensurePackagesForCompileBackend(specs),
  )
}

export async function mountLivePreviewClient(
  vectorData: Uint8Array,
  container: HTMLElement,
  options?: { pixelPerPt?: number },
): Promise<LivePreviewController> {
  if (!compilerReady) {
    await initCompilerClient()
  }
  await ensureBackendInitialized()
  return mountLivePreviewBackend(vectorData, container, options)
}

export function isCompilerReadyClient(): boolean {
  return compilerReady || isCompilerReadyBackend()
}

export function getPackageRuntimeEpochClient(): number {
  return packageRuntimeEpoch
}
