import {
  compileToPdfClient,
  compileTypstClient,
  ensurePackagesForCompileClient,
  initCompilerClient,
  isCompilerReadyClient,
  mountLivePreviewClient,
  resolveSourceLocBatchClient,
  resolveSourceLocClient,
} from './compiler-client'
export type { CompileResult, LivePreviewController, PageDimension, CompileTimings } from './compiler-backend'

export const initCompiler = initCompilerClient
export const compileTypst = compileTypstClient
export const resolveSourceLoc = resolveSourceLocClient
export const resolveSourceLocBatch = resolveSourceLocBatchClient
export const compileToPdf = compileToPdfClient
export const ensurePackagesForCompile = ensurePackagesForCompileClient
export const mountLivePreview = mountLivePreviewClient
export const isCompilerReady = isCompilerReadyClient
