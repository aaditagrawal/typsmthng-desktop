import type { Project } from '@/stores/project-store'

export interface CompileTextFile {
  path: string
  content: string
}

export interface CompileBinaryFile {
  path: string
  data: Uint8Array
}

export interface CompileInputs {
  mainPath: string
  mainSource: string
  textFiles: CompileTextFile[]
  extraFiles: CompileTextFile[]
  extraBinaryFiles: CompileBinaryFile[]
}

interface BuildCompileInputsOptions {
  project?: Project
  currentFilePath?: string | null
  liveSource: string
  defaultMainPath?: string
  transformText?: (path: string, content: string) => string
}

function toWorkspacePath(input: string): string {
  const normalized = input.replace(/\\/g, '/').replace(/^\/+/, '')
  return normalized ? `/${normalized}` : '/main.typ'
}

export function buildCompileInputs({
  project,
  currentFilePath,
  liveSource,
  defaultMainPath = '/main.typ',
  transformText,
}: BuildCompileInputsOptions): CompileInputs {
  const transform = transformText ?? ((_path: string, content: string) => content)

  if (!project) {
    const mainSource = transform(defaultMainPath, liveSource)
    return {
      mainPath: defaultMainPath,
      mainSource,
      textFiles: [{ path: defaultMainPath, content: mainSource }],
      extraFiles: [],
      extraBinaryFiles: [],
    }
  }

  const mainPath = toWorkspacePath(project.mainFile ?? defaultMainPath)
  const textFiles: CompileTextFile[] = []
  const extraFiles: CompileTextFile[] = []
  const extraBinaryFiles: CompileBinaryFile[] = []

  let mainSource = transform(mainPath, liveSource)

  for (const file of project.files) {
    if ((file.kind ?? 'file') !== 'file') {
      continue
    }

    if (!file.isBinary) {
      const content = currentFilePath && file.path === currentFilePath
        ? liveSource
        : file.content
      const normalized = transform(file.path, content)
      const entry: CompileTextFile = { path: toWorkspacePath(file.path), content: normalized }
      textFiles.push(entry)

      if (entry.path === mainPath) {
        mainSource = normalized
      } else {
        extraFiles.push(entry)
      }
      continue
    }

    const binaryPath = toWorkspacePath(file.path)
    if (binaryPath !== mainPath && file.binaryData) {
      extraBinaryFiles.push({ path: binaryPath, data: file.binaryData })
    }
  }

  if (!textFiles.some((file) => file.path === mainPath)) {
    const fallback = { path: mainPath, content: mainSource }
    textFiles.push(fallback)
  }

  return {
    mainPath,
    mainSource,
    textFiles,
    extraFiles,
    extraBinaryFiles,
  }
}
