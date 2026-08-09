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

export function resolveImportedMainFile(projectFiles: Array<{ path: string }>): string {
  return projectFiles.find((f) => f.path === '/main.typ')?.path
    || projectFiles.find((f) => f.path.endsWith('.typ'))?.path
    || projectFiles[0]?.path
    || '/main.typ'
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

async function createImportedProject(projectName: string, projectFiles: ProjectFile[]): Promise<string> {
  const scaffold: ProjectScaffold = {
    files: projectFiles.map((file) => ({
      path: file.path,
      content: file.content,
      isBinary: file.isBinary,
      binaryData: file.binaryData,
    })),
    mainFile: resolveImportedMainFile(projectFiles),
  }

  const id = await useProjectStore.getState().createProject(projectName, scaffold, {
    ifExists: 'fail',
    select: false,
  })
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
    if (zipPath.endsWith('.folder')) continue
    if (file.isBinary && file.binaryData) {
      files[zipPath] = file.binaryData
    } else if (!file.isBinary) {
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
  downloadBlob(`${project.name}.zip`, new Blob([zipped as BlobPart], { type: 'application/zip' }))
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
  downloadBlob(downloadName, new Blob([zipped as BlobPart], { type: 'application/zip' }))
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

  // Group files by top-level folder (each folder = one project)
  const projectFolders = new Map<string, Array<{ path: string; data: Uint8Array }>>()

  for (const [path, data] of Object.entries(unzipped)) {
    if (path.endsWith('/') || path.includes('__MACOSX') || path.includes('.DS_Store')) continue
    const slashIndex = path.indexOf('/')
    if (slashIndex < 0) continue // skip files not in a folder
    const folderName = path.slice(0, slashIndex)
    const filePath = path.slice(slashIndex) // keeps leading slash
    if (!projectFolders.has(folderName)) {
      projectFolders.set(folderName, [])
    }
    projectFolders.get(folderName)!.push({ path: filePath, data })
  }

  if (projectFolders.size === 0) {
    throw new Error('No project folders found in the archive.')
  }

  const store = useProjectStore.getState()
  let imported = 0

  for (const [folderName, entries] of projectFolders) {
    const commonRoot = stripZipCommonRootPrefix(entries.map((entry) => entry.path))
    const filesByPath = new Map<string, ProjectFile>()
    const convertedFromTex = new Set<string>()

    for (const { path, data } of entries) {
      if (path.endsWith('.folder')) continue
      const fullPath = applyZipCommonRootStrip(path, commonRoot)
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
            filePath = fullPath.replace(/\.tex$/i, '.typ')
          } catch (err) {
            console.warn(`LaTeX conversion failed for "${path}":`, err)
            filePath = fullPath.replace(/\.tex$/i, '.typ')
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

    const projectFiles = [...filesByPath.values()]
    if (projectFiles.length === 0) continue

    const id = await store.createProject(folderName, {
      files: projectFiles.map((projectFile) => ({
        path: projectFile.path,
        content: projectFile.content,
        isBinary: projectFile.isBinary,
        binaryData: projectFile.binaryData,
      })),
      mainFile: resolveImportedMainFile(projectFiles),
    }, { ifExists: 'fail', select: false })
    if (id) {
      imported++
    }
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
  const zipEntries = Object.entries(unzipped).filter(([path]) => (
    !path.endsWith('/')
    && !path.includes('__MACOSX')
    && !path.includes('.DS_Store')
  ))
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
          filePath = fullPath.replace(/\.tex$/i, '.typ')
        } catch (err) {
          console.warn(`LaTeX conversion failed for "${path}":`, err)
          filePath = fullPath.replace(/\.tex$/i, '.typ')
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

  const projectFiles = [...filesByPath.values()]
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
  let texCount = 0
  let lastMeta: ConversionResult['metadata'] = { packages: [] }

  const commonRoot = stripZipCommonRootPrefix(files.map((entry) => entry.relativePath))

  for (const { relativePath, file } of files) {
    const path = applyZipCommonRootStrip(relativePath, commonRoot)

    if (isLatexPath(file.name)) {
      const source = await file.text()
      const typPath = path.replace(/\.tex$/i, '.typ')
      let content: string
      try {
        const result = await convertLatexToTypst(source)
        content = result.typst
        allWarnings.push(...result.warnings)
        if (result.metadata.title || result.metadata.author) lastMeta = result.metadata
      } catch (err) {
        console.warn(`LaTeX conversion failed for "${file.name}":`, err)
        allWarnings.push({
          message: `Conversion failed for ${file.name}: ${err instanceof Error ? err.message : 'unknown error'}`,
          construct: file.name,
        })
        content = latexConversionFallback(source)
      }
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

  const projectFiles = [...filesByPath.values()]
  if (projectFiles.length === 0) {
    throw new Error('No files found to import')
  }

  // Determine project name: metadata title > first .tex filename > generic
  const projectName = lastMeta.title
    || (texCount === 1
      ? files.find((f) => isLatexPath(f.file.name))!.file.name.replace(/\.tex$/i, '')
      : `LaTeX Import (${texCount} files)`)

  await createImportedProject(projectName, projectFiles)

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
  let texCount = 0
  let lastMeta: ConversionResult['metadata'] = { packages: [] }

  const zipEntries = Object.entries(unzipped).filter(([path]) => (
    !path.endsWith('/')
    && !path.includes('__MACOSX')
    && !path.includes('.DS_Store')
  ))
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
        try {
          const result = await convertLatexToTypst(content)
          content = result.typst
          filePath = fullPath.replace(/\.tex$/i, '.typ')
          allWarnings.push(...result.warnings)
          if (result.metadata.title || result.metadata.author) lastMeta = result.metadata
          texCount++
        } catch (err) {
          console.warn(`LaTeX conversion failed for "${path}":`, err)
          allWarnings.push({
            message: `Conversion failed for ${path}: ${err instanceof Error ? err.message : 'unknown error'}`,
            construct: path,
          })
          filePath = fullPath.replace(/\.tex$/i, '.typ')
          content = latexConversionFallback(content)
          texCount++
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

  const projectFiles = [...filesByPath.values()]
  if (projectFiles.length === 0) {
    throw new Error('The zip archive contains no importable files.')
  }

  const projectName = lastMeta.title
    || file.name.replace(/\.zip$/i, '')

  await createImportedProject(projectName, projectFiles)

  return {
    projectName,
    fileCount: projectFiles.length,
    texFilesConverted: texCount,
    warnings: allWarnings,
    metadata: lastMeta,
  }
}
