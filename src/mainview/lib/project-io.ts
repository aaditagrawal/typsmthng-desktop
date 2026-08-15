import { zipSync, unzipSync, strToU8, strFromU8 } from 'fflate'
import { useProjectStore, type Project, type ProjectFile, type ProjectScaffold } from '@/stores/project-store'
import { isKnownTextPath, isLatexPath, shouldTreatUploadAsText } from '@/lib/file-classification'
import { convertLatexToTypst, type ConversionResult, type ConversionWarning } from '@/lib/latex-converter'
import { downloadBlob } from '@/lib/download-blob'
import { desktopRpc } from '@/lib/desktop-rpc'

function latexConversionFallback(source: string): string {
  return `// LaTeX conversion failed for this file.\n// Original .tex content preserved below:\n\n/* ${source.replace(/\*\//g, '* /')} */\n`
}

export interface LatexImportResult {
  projectName: string
  fileCount: number
  texFilesConverted: number
  warnings: ConversionWarning[]
  metadata: ConversionResult['metadata']
}

export function resolveImportedMainFile(
  projectFiles: Array<{ path: string; content?: string }>,
  options?: { documentPaths?: Iterable<string> },
): string {
  const typFiles = projectFiles.filter((file) => file.path.toLowerCase().endsWith('.typ'))
  const mainFiles = typFiles.filter((file) => {
    const base = basenamePath(file.path).toLowerCase()
    return base === 'main.typ'
  })
  if (mainFiles.length > 0) {
    mainFiles.sort((left, right) => {
      const leftDepth = left.path.split('/').length
      const rightDepth = right.path.split('/').length
      return leftDepth - rightDepth
    })
    return mainFiles[0].path
  }

  const documentSet = new Set(
    [...(options?.documentPaths ?? [])].map((path) => (path.startsWith('/') ? path : `/${path}`)),
  )
  const documentFiles = typFiles.filter((file) => {
    const normalized = file.path.startsWith('/') ? file.path : `/${file.path}`
    return documentSet.has(normalized) || looksLikeConvertedDocument(file.content ?? '')
  })
  if (documentFiles.length === 1) return documentFiles[0].path
  if (documentFiles.length > 1) {
    documentFiles.sort((left, right) => (right.content?.length ?? 0) - (left.content?.length ?? 0))
    return documentFiles[0].path
  }

  const bySize = [...typFiles].sort(
    (left, right) => (right.content?.length ?? 0) - (left.content?.length ?? 0),
  )
  if ((bySize[0]?.content?.length ?? 0) > 0) return bySize[0].path

  const rootTyp = typFiles.find((file) => !file.path.replace(/^\/+/, '').includes('/'))
  return rootTyp?.path || typFiles[0]?.path || projectFiles[0]?.path || '/main.typ'
}

function looksLikeConvertedDocument(content: string): boolean {
  return /#set document\(/.test(content) || /\\begin\s*\{document\}/.test(content)
}

function latexToTypstPath(input: string): string {
  return input.replace(/\.(tex|ltx)$/i, '.typ')
}

function isLatexDocumentSource(source: string): boolean {
  return /\\begin\s*\{document\}/i.test(source) || /\\documentclass/i.test(source)
}

function parentOsDirectory(rootPath: string): string | undefined {
  const trimmed = rootPath.replace(/[\\/]+$/, '')
  const idx = Math.max(trimmed.lastIndexOf('/'), trimmed.lastIndexOf('\\'))
  if (idx <= 0) return undefined
  return trimmed.slice(0, idx)
}

/** Strip a single shared top-level folder from zip paths (Overleaf-style archives). */
export function stripZipCommonRootPrefix(paths: string[]): string {
  const normalized = paths
    .map((path) => path.replace(/^\/+/, ''))
    .filter((path) => path.length > 0 && !path.endsWith('/'))
  if (normalized.length === 0) return ''

  const firstSegments = normalized.map((path) => path.split('/')[0] ?? '')
  const candidate = firstSegments[0]
  if (!candidate) return ''
  if (!firstSegments.every((segment) => segment === candidate)) return ''
  // Only strip when every entry lives under that folder (has a nested path).
  if (!normalized.every((path) => path.includes('/'))) return ''
  return candidate
}

export function applyZipCommonRootStrip(path: string, rootPrefix: string): string {
  if (!rootPrefix) return path.startsWith('/') ? path : `/${path}`
  const withoutSlash = path.replace(/^\/+/, '')
  const stripped = withoutSlash.startsWith(`${rootPrefix}/`)
    ? withoutSlash.slice(rootPrefix.length + 1)
    : withoutSlash
  return stripped.startsWith('/') ? stripped : `/${stripped}`
}

function basenamePath(input: string): string {
  const normalized = input.replace(/\\/g, '/')
  const separatorIndex = normalized.lastIndexOf('/')
  return separatorIndex === -1 ? normalized : normalized.slice(separatorIndex + 1)
}

function isSkippedZipMetaPath(path: string): boolean {
  return path.includes('__MACOSX') || path.includes('.DS_Store')
}

function isFolderMarkerPath(path: string): boolean {
  return path === '.folder' || path.endsWith('/.folder')
}

/** Normalize zip/folder entry paths; reject traversal and skip archive junk. */
export function normalizeImportEntryPath(path: string): string | null {
  const posix = path.replace(/\\/g, '/')
  if (!posix || posix.endsWith('/')) return null
  if (isSkippedZipMetaPath(posix)) return null

  const withoutSlash = posix.replace(/^\/+/, '')
  if (!withoutSlash || isFolderMarkerPath(withoutSlash)) return null

  const resolved: string[] = []
  for (const segment of withoutSlash.split('/')) {
    if (!segment || segment === '.') continue
    if (segment === '..') {
      if (resolved.length === 0) {
        throw new Error(`Archive path escapes project root: ${path}`)
      }
      resolved.pop()
      continue
    }
    if (/^[a-zA-Z]:$/.test(segment)) {
      throw new Error(`Archive path escapes project root: ${path}`)
    }
    resolved.push(segment)
  }
  if (resolved.length === 0) return null
  return resolved.join('/')
}

/** Prefer converted .tex→.typ over a same-named shipped .typ when both exist. */
function setImportedTextFile(
  filesByPath: Map<string, ProjectFile>,
  convertedFromTex: Set<string>,
  filePath: string,
  content: string,
  fromTex: boolean,
): void {
  const existing = filesByPath.get(filePath)
  if (existing && !fromTex && convertedFromTex.has(filePath)) {
    // Keep the converted .typ when the archive also shipped a same-named .typ.
    return
  }

  filesByPath.set(filePath, {
    path: filePath,
    content,
    isBinary: false,
    lastModified: Date.now(),
  })
  if (fromTex) convertedFromTex.add(filePath)
}

async function createImportedProject(
  projectName: string,
  projectFiles: ProjectFile[],
  options?: { documentPaths?: Iterable<string>; parentPath?: string },
): Promise<string> {
  const scaffold: ProjectScaffold = {
    files: projectFiles.map((file) => ({
      path: file.path,
      content: file.content,
      isBinary: file.isBinary,
      binaryData: file.binaryData,
    })),
    mainFile: resolveImportedMainFile(projectFiles, { documentPaths: options?.documentPaths }),
  }

  const createOptions: { ifExists: 'fail'; select: false; parentPath?: string } = {
    ifExists: 'fail',
    select: false,
  }
  if (options?.parentPath) createOptions.parentPath = options.parentPath

  const id = await useProjectStore.getState().createProject(projectName, scaffold, createOptions)
  if (!id) {
    throw new Error(
      'Project creation was cancelled or a project with that name already exists in the chosen folder.',
    )
  }
  return id
}

async function collectProjectExportFiles(project: Project): Promise<Record<string, Uint8Array>> {
  const files: Record<string, Uint8Array> = {}

  // Always export from disk. Vault snapshots often keep only the main file hydrated,
  // so serializing project.files would produce empty companion files/binaries.
  // Fail closed: never silently zip stale/partial disk state after a flush error.
  await desktopRpc.request.flushWrites({ rootPath: project.rootPath })

  let bundle: Awaited<ReturnType<typeof desktopRpc.request.getVaultExportBundle>>
  try {
    bundle = await desktopRpc.request.getVaultExportBundle({ rootPath: project.rootPath })
  } catch (err) {
    throw new Error(
      err instanceof Error
        ? err.message
        : `Could not read files for project "${project.name}".`,
    )
  }
  if (!bundle) {
    throw new Error(`Could not read files for project "${project.name}".`)
  }
  for (const file of bundle.files) {
    const zipPath = file.path.startsWith('/') ? file.path.slice(1) : file.path
    if (!zipPath || zipPath === '.folder' || zipPath.endsWith('/.folder')) continue
    if (file.isBinary) {
      if (!file.binaryData) {
        throw new Error(
          `Missing binary data for "${zipPath}" while exporting "${project.name}".`,
        )
      }
      files[zipPath] = file.binaryData
    } else {
      files[zipPath] = strToU8(file.content ?? '')
    }
  }
  return files
}

export function uniqueExportFolderName(project: Project, usedNames: Set<string>): string {
  const sanitized = project.name.replace(/[/\\:*?"<>|]/g, '_').trim() || 'project'
  if (!usedNames.has(sanitized)) {
    usedNames.add(sanitized)
    return sanitized
  }

  const rootSegment = basenamePath(project.rootPath || project.id)
    .replace(/[/\\:*?"<>|]/g, '_')
    .trim()
  const suffix = rootSegment && rootSegment !== sanitized
    ? rootSegment
    : basenamePath(project.id).slice(-8) || 'dup'
  let candidate = `${sanitized}-${suffix}`
  if (usedNames.has(candidate)) {
    let n = 2
    while (usedNames.has(`${candidate}-${n}`)) n++
    candidate = `${candidate}-${n}`
  }
  usedNames.add(candidate)
  return candidate
}

export function sanitizeImportedProjectName(input: string | undefined, fallback: string): string {
  const sanitized = (input ?? '').replace(/[/\\:*?"<>|]/g, '_').trim()
  return sanitized || fallback
}

const IMAGE_EXTENSIONS = ['.png', '.jpg', '.jpeg', '.gif', '.svg', '.pdf', '.webp']

function rewriteImportedTypstAssets(
  content: string,
  availablePaths: string[],
): string {
  const normalizedAvailable = availablePaths.map((path) => path.replace(/^\/+/, ''))
  return content.replace(/(\bimage\(")([^"]+)("\))/g, (full, prefix, rawPath, suffix) => {
    const requested = rawPath.replace(/\\/g, '/').replace(/^\/+/, '')
    if (!requested) return full
    const candidates = [
      requested,
      ...IMAGE_EXTENSIONS.map((ext) => `${requested}${ext}`),
    ]
    for (const candidate of candidates) {
      const match = normalizedAvailable.find((path) => path === candidate || path.endsWith(`/${candidate}`))
      if (match) return `${prefix}/${match}${suffix}`
    }
    return `${prefix}${requested}${suffix}`
  })
}

function applyAssetRewrites(files: ProjectFile[]): ProjectFile[] {
  const available = files.map((file) => file.path)
  return files.map((file) => {
    if (file.isBinary || !file.path.toLowerCase().endsWith('.typ')) return file
    return { ...file, content: rewriteImportedTypstAssets(file.content, available) }
  })
}

function unzipEntries(data: Uint8Array): Array<[string, Uint8Array]> {
  const unzipped = unzipSync(data)
  return Object.entries(unzipped).flatMap(([rawPath, bytes]) => {
    const normalized = normalizeImportEntryPath(rawPath)
    return normalized ? [[normalized, bytes] as const] : []
  })
}

function maybeUnwrapNestedZip(entries: Array<readonly [string, Uint8Array]>): Array<readonly [string, Uint8Array]> {
  const hasSource = entries.some(([path]) => isLatexPath(path) || path.toLowerCase().endsWith('.typ'))
  if (hasSource) return entries
  const innerZips = entries.filter(([path]) => path.toLowerCase().endsWith('.zip'))
  if (innerZips.length !== 1) return entries
  try {
    return unzipEntries(innerZips[0][1])
  } catch {
    return entries
  }
}

function unwrapImportAllWrapper(
  projectFolders: Map<string, Array<{ path: string; data: Uint8Array }>>,
): Map<string, Array<{ path: string; data: Uint8Array }>> {
  if (projectFolders.size !== 1) return projectFolders
  const [wrapperName, entries] = [...projectFolders.entries()][0]
  const childFolders = new Map<string, Array<{ path: string; data: Uint8Array }>>()
  let hasRootFiles = false
  for (const entry of entries) {
    const relative = entry.path.replace(/^\/+/, '')
    const slashIndex = relative.indexOf('/')
    if (slashIndex < 0) {
      hasRootFiles = true
      break
    }
    const child = relative.slice(0, slashIndex)
    const rest = relative.slice(slashIndex)
    if (!childFolders.has(child)) childFolders.set(child, [])
    childFolders.get(child)!.push({ path: rest, data: entry.data })
  }
  if (hasRootFiles || childFolders.size < 2) return projectFolders
  void wrapperName
  return childFolders
}

export async function exportProject(projectId?: string): Promise<void> {
  const state = useProjectStore.getState()
  const project = projectId
    ? state.projects.find((entry) => entry.id === projectId)
    : state.getCurrentProject()
  if (!project) {
    throw new Error(projectId ? 'Selected project was not found.' : 'No project is open to export.')
  }

  const files = await collectProjectExportFiles(project)
  if (Object.keys(files).length === 0) {
    throw new Error(`Project "${project.name}" has no exportable files.`)
  }

  let zipped: Uint8Array
  try {
    zipped = zipSync(files)
  } catch (err) {
    console.error('Failed to export project:', err)
    throw new Error(`Failed to zip project "${project.name}".`)
  }
  await downloadBlob(`${project.name}.zip`, new Blob([zipped as BlobPart], { type: 'application/zip' }))
}

async function exportProjectsAsFolderArchive(
  projectIds: string[],
  downloadName: string,
): Promise<void> {
  const uniqueIds = [...new Set(projectIds)]
  if (uniqueIds.length === 0) {
    throw new Error('No projects selected to export.')
  }

  const state = useProjectStore.getState()
  const files: Record<string, Uint8Array> = {}
  const usedFolderNames = new Set<string>()
  let exported = 0

  for (const projectId of uniqueIds) {
    const project = state.projects.find((entry) => entry.id === projectId)
    if (!project) {
      throw new Error('One of the selected projects was not found.')
    }
    const projectFiles = await collectProjectExportFiles(project)
    if (Object.keys(projectFiles).length === 0) {
      throw new Error(`Project "${project.name}" has no exportable files.`)
    }
    const folderName = uniqueExportFolderName(project, usedFolderNames)
    for (const [filePath, data] of Object.entries(projectFiles)) {
      files[`${folderName}/${filePath}`] = data
    }
    exported++
  }

  if (exported !== uniqueIds.length) {
    throw new Error('Could not export every selected project.')
  }

  let zipped: Uint8Array
  try {
    zipped = zipSync(files)
  } catch (err) {
    console.error('Failed to export selected projects:', err)
    throw new Error('Failed to zip the selected projects.')
  }
  await downloadBlob(downloadName, new Blob([zipped as BlobPart], { type: 'application/zip' }))
}

export async function exportProjects(projectIds: string[]): Promise<void> {
  const uniqueIds = [...new Set(projectIds)]
  if (uniqueIds.length === 0) {
    throw new Error('No projects selected to export.')
  }
  // Single-project export stays flat for Import project; multi stays nested for Import all.
  if (uniqueIds.length === 1) {
    await exportProject(uniqueIds[0])
    return
  }
  await exportProjectsAsFolderArchive(uniqueIds, 'typsmthng-selected-projects.zip')
}

export async function exportAllProjects(): Promise<void> {
  const projects = useProjectStore.getState().projects
  if (projects.length === 0) {
    throw new Error('There are no projects to export.')
  }
  // Always nest under project folders so Import all can round-trip, even for one project.
  await exportProjectsAsFolderArchive(
    projects.map((project) => project.id),
    'typsmthng-all-projects.zip',
  )
}

export async function importAllProjects(file: File): Promise<number> {
  const buffer = await file.arrayBuffer()
  let unzipped: ReturnType<typeof unzipSync>
  try {
    unzipped = unzipSync(new Uint8Array(buffer))
  } catch {
    throw new Error('The file does not appear to be a valid zip archive.')
  }

  const rawEntries = Object.entries(unzipped).flatMap(([rawPath, data]) => {
    const normalized = normalizeImportEntryPath(rawPath)
    return normalized ? [[normalized, data] as const] : []
  })
  const unwrapped = maybeUnwrapNestedZip(rawEntries)

  // Group files by top-level folder (each folder = one project)
  const projectFolders = new Map<string, Array<{ path: string; data: Uint8Array }>>()

  for (const [normalized, data] of unwrapped) {
    const slashIndex = normalized.indexOf('/')
    if (slashIndex < 0) continue // skip files not in a folder
    const folderName = normalized.slice(0, slashIndex)
    const filePath = normalized.slice(slashIndex) // keeps leading slash
    if (!projectFolders.has(folderName)) {
      projectFolders.set(folderName, [])
    }
    projectFolders.get(folderName)!.push({ path: filePath, data })
  }

  const grouped = unwrapImportAllWrapper(projectFolders)

  if (grouped.size === 0) {
    throw new Error('No project folders found in the archive.')
  }

  const store = useProjectStore.getState()
  let imported = 0
  let parentPath: string | undefined
  const failed: string[] = []

  for (const [folderName, entries] of grouped) {
    const commonRoot = stripZipCommonRootPrefix(entries.map((entry) => entry.path))
    const filesByPath = new Map<string, ProjectFile>()
    const convertedFromTex = new Set<string>()
    const documentPaths = new Set<string>()

    for (const { path, data } of entries) {
      const fullPath = applyZipCommonRootStrip(path, commonRoot)
      const isText = isKnownTextPath(path)
      if (isText) {
        let content = strFromU8(data)
        let filePath = fullPath
        let fromTex = false

        if (isLatexPath(path)) {
          fromTex = true
          const source = content
          if (isLatexDocumentSource(source)) {
            documentPaths.add(latexToTypstPath(fullPath))
          }
          try {
            const result = await convertLatexToTypst(content)
            content = result.typst
            filePath = latexToTypstPath(fullPath)
          } catch (err) {
            console.warn(`LaTeX conversion failed for "${path}":`, err)
            filePath = latexToTypstPath(fullPath)
            content = latexConversionFallback(content)
          }
        }

        setImportedTextFile(filesByPath, convertedFromTex, filePath, content, fromTex)
      } else {
        if (filesByPath.has(fullPath)) continue
        filesByPath.set(fullPath, {
          path: fullPath,
          content: '',
          isBinary: true,
          binaryData: data,
          lastModified: Date.now(),
        })
      }
    }

    const projectFiles = applyAssetRewrites([...filesByPath.values()])
    if (projectFiles.length === 0) continue

    const createOptions: { ifExists: 'fail'; select: false; parentPath?: string } = {
      ifExists: 'fail',
      select: false,
    }
    if (parentPath) createOptions.parentPath = parentPath

    const id = await store.createProject(folderName, {
      files: projectFiles.map((projectFile) => ({
        path: projectFile.path,
        content: projectFile.content,
        isBinary: projectFile.isBinary,
        binaryData: projectFile.binaryData,
      })),
      mainFile: resolveImportedMainFile(projectFiles, { documentPaths }),
    }, createOptions)
    if (!id) {
      failed.push(folderName)
      continue
    }
    parentPath = parentOsDirectory(id) ?? parentPath
    imported++
  }

  if (imported === 0 && failed.length > 0) {
    throw new Error(
      `Could not import project "${failed[0]}" (cancelled or a project with that name already exists).`,
    )
  }

  // Go back to home after import
  useProjectStore.setState({ hasSelectedProject: false, currentProjectId: null, currentFilePath: null })

  return imported
}

export async function importProject(file: File): Promise<void> {
  const buffer = await file.arrayBuffer()
  let unzipped: ReturnType<typeof unzipSync>
  try {
    unzipped = unzipSync(new Uint8Array(buffer))
  } catch (err) {
    console.error('Import failed:', err)
    throw new Error('Failed to import project: the file does not appear to be a valid zip archive.')
  }

  // Determine project name from zip filename
  const projectName = file.name.replace(/\.zip$/i, '')

  const filesByPath = new Map<string, ProjectFile>()
  const convertedFromTex = new Set<string>()
  const zipEntries = maybeUnwrapNestedZip(Object.entries(unzipped).flatMap(([rawPath, data]) => {
    const normalized = normalizeImportEntryPath(rawPath)
    return normalized ? [[normalized, data] as const] : []
  }))
  const commonRoot = stripZipCommonRootPrefix(zipEntries.map(([path]) => path))

  for (const [path, data] of zipEntries) {
    const fullPath = applyZipCommonRootStrip(path, commonRoot)

    // Detect if file is text or binary
    const isText = isKnownTextPath(path)

    if (isText) {
      let content = strFromU8(data)
      let filePath = fullPath
      let fromTex = false

      if (isLatexPath(path)) {
        fromTex = true
        try {
          const result = await convertLatexToTypst(content)
          content = result.typst
          filePath = latexToTypstPath(fullPath)
        } catch (err) {
          console.warn(`LaTeX conversion failed for "${path}":`, err)
          filePath = latexToTypstPath(fullPath)
          content = latexConversionFallback(content)
        }
      }

      setImportedTextFile(filesByPath, convertedFromTex, filePath, content, fromTex)
    } else if (!filesByPath.has(fullPath)) {
      filesByPath.set(fullPath, {
        path: fullPath,
        content: '',
        isBinary: true,
        binaryData: data,
        lastModified: Date.now(),
      })
    }
  }

  const projectFiles = applyAssetRewrites([...filesByPath.values()])
  if (projectFiles.length === 0) {
    throw new Error('The zip archive contains no importable files.')
  }

  await createImportedProject(projectName, projectFiles)
}

/** Import a LaTeX project from .tex files, a .zip, or a folder of files.
 *  .tex files are converted to .typ; other files are passed through. */
export async function importLatexProject(
  files: Array<{ relativePath: string; file: File }>,
): Promise<LatexImportResult> {
  const allWarnings: ConversionWarning[] = []
  const filesByPath = new Map<string, ProjectFile>()
  const convertedFromTex = new Set<string>()
  const documentPaths = new Set<string>()
  const metaByTypPath = new Map<string, ConversionResult['metadata']>()
  let texCount = 0

  const normalizedInputs = files.map((entry) => {
    const normalized = normalizeImportEntryPath(entry.relativePath)
    if (!normalized) return null
    return { relativePath: normalized, file: entry.file }
  }).filter((entry): entry is { relativePath: string; file: File } => entry !== null)

  const commonRoot = stripZipCommonRootPrefix(normalizedInputs.map((entry) => entry.relativePath))

  for (const { relativePath, file } of normalizedInputs) {
    const path = applyZipCommonRootStrip(relativePath, commonRoot)

    if (isLatexPath(file.name) || isLatexPath(path)) {
      const source = await file.text()
      const typPath = latexToTypstPath(path)
      let content: string
      try {
        const result = await convertLatexToTypst(source)
        content = result.typst
        allWarnings.push(...result.warnings)
        metaByTypPath.set(typPath, result.metadata)
      } catch (err) {
        console.warn(`LaTeX conversion failed for "${file.name}":`, err)
        allWarnings.push({
          message: `Conversion failed for ${file.name}: ${err instanceof Error ? err.message : 'unknown error'}`,
          construct: file.name,
        })
        content = latexConversionFallback(source)
      }
      if (isLatexDocumentSource(source)) documentPaths.add(typPath)
      setImportedTextFile(filesByPath, convertedFromTex, typPath, content, true)
      texCount++
    } else if (shouldTreatUploadAsText(file)) {
      const content = await file.text()
      setImportedTextFile(filesByPath, convertedFromTex, path, content, false)
    } else if (!filesByPath.has(path)) {
      const buffer = await file.arrayBuffer()
      filesByPath.set(path, {
        path,
        content: '',
        isBinary: true,
        binaryData: new Uint8Array(buffer),
        lastModified: Date.now(),
      })
    }
  }

  const projectFiles = applyAssetRewrites([...filesByPath.values()])
  if (projectFiles.length === 0) {
    throw new Error('No files found to import')
  }

  const mainFile = resolveImportedMainFile(projectFiles, { documentPaths })
  const lastMeta = metaByTypPath.get(mainFile)
    ?? [...metaByTypPath.values()].find((meta) => meta.title)
    ?? { packages: [], graphicspath: [] }

  const latexInput = normalizedInputs.find((entry) => (
    isLatexPath(entry.file.name) || isLatexPath(entry.relativePath)
  ))
  const fallbackName = texCount === 1 && latexInput
    ? latexInput.file.name.replace(/\.(tex|ltx)$/i, '')
    : `LaTeX Import (${texCount} files)`
  const projectName = sanitizeImportedProjectName(lastMeta.title, fallbackName)

  await createImportedProject(projectName, projectFiles, { documentPaths })

  return {
    projectName,
    fileCount: projectFiles.length,
    texFilesConverted: texCount,
    warnings: allWarnings,
    metadata: lastMeta,
  }
}

/** Import a LaTeX project from a .zip file containing .tex files. */
export async function importLatexZip(file: File): Promise<LatexImportResult> {
  const buffer = await file.arrayBuffer()
  let unzipped: ReturnType<typeof unzipSync>
  try {
    unzipped = unzipSync(new Uint8Array(buffer))
  } catch {
    throw new Error('The file does not appear to be a valid zip archive.')
  }

  const allWarnings: ConversionWarning[] = []
  const filesByPath = new Map<string, ProjectFile>()
  const convertedFromTex = new Set<string>()
  const documentPaths = new Set<string>()
  const metaByTypPath = new Map<string, ConversionResult['metadata']>()
  let texCount = 0

  const zipEntries = maybeUnwrapNestedZip(Object.entries(unzipped).flatMap(([rawPath, data]) => {
    const normalized = normalizeImportEntryPath(rawPath)
    return normalized ? [[normalized, data] as const] : []
  }))
  const commonRoot = stripZipCommonRootPrefix(zipEntries.map(([path]) => path))

  for (const [path, data] of zipEntries) {
    const fullPath = applyZipCommonRootStrip(path, commonRoot)
    const isText = isKnownTextPath(path)

    if (isText) {
      let content = strFromU8(data)
      let filePath = fullPath
      let fromTex = false

      if (isLatexPath(path)) {
        fromTex = true
        const source = content
        try {
          const result = await convertLatexToTypst(content)
          content = result.typst
          filePath = latexToTypstPath(fullPath)
          allWarnings.push(...result.warnings)
          metaByTypPath.set(filePath, result.metadata)
        } catch (err) {
          console.warn(`LaTeX conversion failed for "${path}":`, err)
          allWarnings.push({
            message: `Conversion failed for ${path}: ${err instanceof Error ? err.message : 'unknown error'}`,
            construct: path,
          })
          filePath = latexToTypstPath(fullPath)
          content = latexConversionFallback(content)
        }
        if (isLatexDocumentSource(source)) documentPaths.add(filePath)
        texCount++
      }

      setImportedTextFile(filesByPath, convertedFromTex, filePath, content, fromTex)
    } else if (!filesByPath.has(fullPath)) {
      filesByPath.set(fullPath, {
        path: fullPath,
        content: '',
        isBinary: true,
        binaryData: data,
        lastModified: Date.now(),
      })
    }
  }

  const projectFiles = applyAssetRewrites([...filesByPath.values()])
  if (projectFiles.length === 0) {
    throw new Error('The zip archive contains no importable files.')
  }
  if (texCount === 0) {
    throw new Error('No .tex files found in the archive.')
  }

  const mainFile = resolveImportedMainFile(projectFiles, { documentPaths })
  const lastMeta = metaByTypPath.get(mainFile)
    ?? [...metaByTypPath.values()].find((meta) => meta.title)
    ?? { packages: [], graphicspath: [] }

  const projectName = sanitizeImportedProjectName(lastMeta.title, file.name.replace(/\.zip$/i, ''))

  await createImportedProject(projectName, projectFiles, { documentPaths })

  return {
    projectName,
    fileCount: projectFiles.length,
    texFilesConverted: texCount,
    warnings: allWarnings,
    metadata: lastMeta,
  }
}
