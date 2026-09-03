import { create } from 'zustand'
import type { PageDimension } from '@/lib/compiler'
import type { InlineSpeakerNote } from '@/lib/presentation-notes'

export type CompileStatus = 'idle' | 'compiling' | 'success' | 'error'

export interface Diagnostic {
  severity: 'error' | 'warning' | 'info'
  path: string
  range: string
  message: string
  package?: string
}

interface CompileState {
  status: CompileStatus
  compilerReady: boolean
  diagnostics: Diagnostic[]
  svg: string | null
  vectorData: Uint8Array | null
  pageDimensions: PageDimension[]
  speakerNotes: InlineSpeakerNote[]
  totalPages: number
  errorCount: number
  warningCount: number
  compileTime: number
  compiledMainPath: string | null
  setStatus: (status: CompileStatus) => void
  setCompilerReady: (ready: boolean) => void
  setDiagnostics: (diagnostics: Diagnostic[]) => void
  setSvgResult: (
    svg: string,
    vectorData: Uint8Array,
    pageDimensions: PageDimension[],
    speakerNotes?: InlineSpeakerNote[],
  ) => void
  setCompileTime: (ms: number) => void
  setCompiledMainPath: (path: string | null) => void
}

export const useCompileStore = create<CompileState>((set) => ({
  status: 'idle',
  compilerReady: false,
  diagnostics: [],
  svg: null,
  vectorData: null,
  pageDimensions: [],
  speakerNotes: [],
  totalPages: 0,
  errorCount: 0,
  warningCount: 0,
  compileTime: 0,
  compiledMainPath: null,
  setStatus: (status) => set({ status }),
  setCompilerReady: (compilerReady) => set({ compilerReady }),
  setDiagnostics: (diagnostics) => set({
    diagnostics,
    errorCount: diagnostics.reduce((count, diag) => count + (diag.severity === 'error' ? 1 : 0), 0),
    warningCount: diagnostics.reduce((count, diag) => count + (diag.severity === 'warning' ? 1 : 0), 0),
  }),
  setSvgResult: (svg, vectorData, pageDimensions, speakerNotes = []) => set({
    svg,
    vectorData,
    pageDimensions,
    speakerNotes,
    totalPages: Math.max(pageDimensions.length, 1),
  }),
  setCompileTime: (compileTime) => set({ compileTime }),
  setCompiledMainPath: (compiledMainPath) => set({ compiledMainPath }),
}))
