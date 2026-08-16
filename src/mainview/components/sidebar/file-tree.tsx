import { useEffect, useMemo, useRef, useState } from 'react'
import {
  ChevronDown,
  ChevronRight,
  Copy,
  ExternalLink,
  FileCode2,
  FileImage,
  FilePlus2,
  FileText,
  FileType,
  Folder,
  FolderOpen,
  FolderPlus,
  FolderSearch,
  Pencil,
  Trash2,
  Upload,
} from 'lucide-react'

import type { LucideIcon } from 'lucide-react'
import { ContextMenu, type ContextMenuAction } from '@/components/ui/context-menu'
import { isLatexPath, isPreviewableImagePath, shouldTreatUploadAsText } from '@/lib/file-classification'
import { convertLatexToTypst } from '@/lib/latex-converter'
import { revealLabel } from '@/lib/platform'
import { normalizeImportEntryPath } from '@/lib/project-io'
import { useProjectStore, type ProjectFile } from '@/stores/project-store'
import { useUIStore } from '@/stores/ui-store'

function fileIcon(name: string): LucideIcon {
  const ext = name.lastIndexOf('.') !== -1 ? name.slice(name.lastIndexOf('.')).toLowerCase() : ''
  switch (ext) {
    case '.typ': return FileType
    case '.pdf': return FileText
    case '.png': case '.jpg': case '.jpeg': case '.gif': case '.svg': case '.webp': return FileImage
    case '.js': case '.ts': case '.tsx': case '.jsx': case '.json': case '.yaml': case '.yml': case '.toml': return FileCode2
    default: return FileText
  }
}

interface TreeNode {
  path: string
  name: string
  kind: 'file' | 'directory'
  children: TreeNode[]
}

interface FlatNode {
  node: TreeNode
  depth: number
}

const ROW_HEIGHT = 34
const OVERSCAN = 10
const VIRTUALIZE_THRESHOLD = 220

const INTERNAL_DND_TYPE = 'application/x-typsmthng-node'

const INDENT_BASE = 10
const INDENT_STEP = 16
const BORDER_LEFT = 2

function IndentGuides({ depth }: { depth: number }) {
  if (depth <= 0) return null
  return (
    <>
      {Array.from({ length: depth }, (_, i) => (
        <span
          key={i}
          style={{
            position: 'absolute',
            left: `${BORDER_LEFT + INDENT_BASE + i * INDENT_STEP + 6}px`,
            top: 0,
            bottom: 0,
            width: '1px',
            background: 'var(--indent-guide)',
            pointerEvents: 'none',
          }}
        />
      ))}
    </>
  )
}

function normalizePath(path: string): string {
  return path.replace(/^\/+/, '').replace(/\\/g, '/')
}

function basename(path: string): string {
  const normalized = normalizePath(path)
  const index = normalized.lastIndexOf('/')
  return index === -1 ? normalized : normalized.slice(index + 1)
}

function parentPath(path: string): string | null {
  const normalized = normalizePath(path)
  const index = normalized.lastIndexOf('/')
  if (index <= 0) return null
  return normalized.slice(0, index)
}

function ensureDirectory(nodeMap: Map<string, TreeNode>, path: string): TreeNode {
  const normalized = normalizePath(path)
  const existing = nodeMap.get(normalized)
  if (existing) return existing

  const node: TreeNode = {
    path: normalized,
    name: basename(normalized),
    kind: 'directory',
    children: [],
  }
  nodeMap.set(normalized, node)

  const parent = parentPath(normalized)
  if (parent) {
    ensureDirectory(nodeMap, parent).children.push(node)
  }

  return node
}

function buildTree(files: ProjectFile[]): TreeNode[] {
  const nodeMap = new Map<string, TreeNode>()
  const roots: TreeNode[] = []

  for (const file of files) {
    const normalized = normalizePath(file.path)
    if (!normalized || normalized.startsWith('.typsmthng/')) continue

    const kind = (file.kind ?? 'file') as 'file' | 'directory'
    if (kind === 'directory') {
      const directory = ensureDirectory(nodeMap, normalized)
      if (!parentPath(normalized) && !roots.some((entry) => entry.path === directory.path)) {
        roots.push(directory)
      }
      continue
    }

    const node: TreeNode = {
      path: normalized,
      name: basename(normalized),
      kind: 'file',
      children: [],
    }
    nodeMap.set(normalized, node)

    const parent = parentPath(normalized)
    if (parent) {
      ensureDirectory(nodeMap, parent).children.push(node)
    } else {
      roots.push(node)
    }
  }

  const sortNode = (node: TreeNode) => {
    node.children = [...new Map(node.children.map((child) => [child.path, child])).values()].sort((left, right) => {
      if (left.kind !== right.kind) return left.kind === 'directory' ? -1 : 1
      return left.name.localeCompare(right.name)
    })
    node.children.forEach(sortNode)
  }

  const uniqueRoots = [...new Map(roots.map((node) => [node.path, node])).values()].sort((left, right) => {
    if (left.kind !== right.kind) return left.kind === 'directory' ? -1 : 1
    return left.name.localeCompare(right.name)
  })

  uniqueRoots.forEach(sortNode)
  return uniqueRoots
}

function structureSignature(files: ProjectFile[]): string {
  let signature = ''
  for (const file of files) {
    const normalized = normalizePath(file.path)
    if (!normalized || normalized.startsWith('.typsmthng/')) continue
    signature += `${normalized}:${file.kind ?? 'file'}:${file.name ?? ''}|`
  }
  return signature
}

function flattenTree(nodes: TreeNode[], expanded: Set<string>, depth = 0): FlatNode[] {
  const items: FlatNode[] = []
  for (const node of nodes) {
    items.push({ node, depth })
    if (node.kind === 'directory' && expanded.has(node.path)) {
      items.push(...flattenTree(node.children, expanded, depth + 1))
    }
  }
  return items
}

function suggestUntitledTypstPath(existingPaths: Iterable<string>, directory: string): string {
  const taken = new Set([...existingPaths].map((entry) => entry.replace(/^\/+/, '')))
  const folder = directory.replace(/^\/+/, '').replace(/\/+$/, '')
  const prefix = folder ? `${folder}/untitled` : 'untitled'
  if (!taken.has(`${prefix}.typ`)) return `${prefix}.typ`
  let n = 2
  while (taken.has(`${prefix}-${n}.typ`)) n += 1
  return `${prefix}-${n}.typ`
}

function buildDuplicatePath(existingPaths: Iterable<string>, path: string): string {
  const taken = new Set(existingPaths)
  const directory = parentPath(path)
  const fileName = basename(path)
  const dotIndex = fileName.lastIndexOf('.')
  const hasExtension = dotIndex > 0
  const baseName = hasExtension ? fileName.slice(0, dotIndex) : fileName
  const extension = hasExtension ? fileName.slice(dotIndex) : ''

  let attempt = 0
  while (true) {
    const suffix = attempt === 0 ? ' copy' : ` copy ${attempt + 1}`
    const candidateName = `${baseName}${suffix}${extension}`
    const candidatePath = directory ? `${directory}/${candidateName}` : candidateName
    if (!taken.has(candidatePath)) {
      return candidatePath
    }
    attempt += 1
  }
}

function resolveImportTargetPath(basePath: string, rawRelativePath: string): string | null {
  try {
    const relativeName = normalizeImportEntryPath(rawRelativePath)
    if (!relativeName) return null
    const combined = basePath ? `${normalizePath(basePath)}/${relativeName}` : relativeName
    return normalizeImportEntryPath(combined)
  } catch (err) {
    console.warn(`Skipping unsafe import path "${rawRelativePath}":`, err)
    return null
  }
}

async function processImportedFiles(files: FileList | File[], basePath: string): Promise<void> {
  const textEntries: Array<{ path: string; content: string }> = []
  const binaryEntries: Array<{ path: string; data: Uint8Array }> = []
  const texPathsToReplace: string[] = []
  const skipped: string[] = []
  const existingPaths = new Set(
    (useProjectStore.getState().getCurrentProject()?.files ?? []).map((entry) =>
      normalizePath(entry.path),
    ),
  )

  for (const file of Array.from(files)) {
    const fileWithRelativePath = file as File & { webkitRelativePath?: string }
    const rawRelative = fileWithRelativePath.webkitRelativePath || file.name
    const targetPath = resolveImportTargetPath(basePath, rawRelative)
    if (!targetPath) {
      skipped.push(file.name)
      continue
    }

    if (shouldTreatUploadAsText(file)) {
      let content = await file.text()
      let path = targetPath

      if (isLatexPath(file.name)) {
        try {
          const result = await convertLatexToTypst(content)
          content = result.typst
          path = targetPath.replace(/\.(tex|ltx)$/i, '.typ')
        } catch (err) {
          console.warn(`LaTeX conversion failed for "${file.name}":`, err)
          path = targetPath.replace(/\.(tex|ltx)$/i, '.typ')
          content = `// LaTeX conversion failed for this file.\n// Original .tex content preserved below:\n\n/* ${content.replace(/\*\//g, '* /')} */\n`
        }
        if (path !== targetPath && existingPaths.has(targetPath)) {
          texPathsToReplace.push(targetPath)
        }
      }

      textEntries.push({ path, content })
    } else {
      binaryEntries.push({
        path: targetPath,
        data: new Uint8Array(await file.arrayBuffer()),
      })
    }
  }

  const plannedPaths = [
    ...textEntries.map((entry) => entry.path),
    ...binaryEntries.map((entry) => entry.path),
  ]
  const overwriting = plannedPaths.filter((path) => existingPaths.has(path))
  if (overwriting.length > 0) {
    const preview = overwriting.slice(0, 5).join('\n')
    const ok = window.confirm(
      `Replace ${overwriting.length} existing file${overwriting.length === 1 ? '' : 's'}?\n\n${preview}${
        overwriting.length > 5 ? '\n…' : ''
      }`,
    )
    if (!ok) return
  }

  try {
    if (textEntries.length > 0) {
      await useProjectStore.getState().createFilesBatch(textEntries)
    }
    if (binaryEntries.length > 0) {
      await useProjectStore.getState().addBinaryFilesBatch(binaryEntries)
    }
    // Dropping .tex writes a converted .typ; remove any leftover original .tex.
    for (const texPath of texPathsToReplace) {
      try {
        await useProjectStore.getState().deleteFile(texPath)
      } catch (err) {
        console.warn(`Failed to remove replaced LaTeX source "${texPath}":`, err)
      }
    }
  } catch (err) {
    const message = err instanceof Error ? err.message : 'Import failed'
    window.alert(`Could not import files: ${message}`)
    return
  }

  if (skipped.length > 0) {
    window.alert(
      `Skipped ${skipped.length} unsafe or invalid path${skipped.length === 1 ? '' : 's'} during import.`,
    )
  }
}

export function FileTree() {
  const currentProject = useProjectStore((s) => s.getCurrentProject())
  const currentFilePath = useProjectStore((s) => s.currentFilePath)
  const selectFile = useProjectStore((s) => s.selectFile)
  const setImagePreviewPath = useUIStore((s) => s.setImagePreviewPath)
  const createFile = useProjectStore((s) => s.createFile)
  const createFolder = useProjectStore((s) => s.createFolder)
  const duplicateFile = useProjectStore((s) => s.duplicateFile)
  const deleteFile = useProjectStore((s) => s.deleteFile)
  const deleteFolder = useProjectStore((s) => s.deleteFolder)
  const renameFile = useProjectStore((s) => s.renameFile)
  const renameFolder = useProjectStore((s) => s.renameFolder)
  const moveFile = useProjectStore((s) => s.moveFile)
  const revealPathInFinder = useProjectStore((s) => s.revealPathInFinder)
  const openPathInDefaultApp = useProjectStore((s) => s.openPathInDefaultApp)

  const fileInputRef = useRef<HTMLInputElement>(null)
  const listRef = useRef<HTMLDivElement>(null)

  const [expanded, setExpanded] = useState<Set<string>>(new Set())
  const [contextMenu, setContextMenu] = useState<{ x: number; y: number; node: TreeNode } | null>(null)
  const [scrollTop, setScrollTop] = useState(0)
  const [viewportHeight, setViewportHeight] = useState(480)
  const [dragActive, setDragActive] = useState(false)
  const [dropTargetDir, setDropTargetDir] = useState<string | null>(null)

  const includeHidden = useProjectStore((s) => (
    s.metadata?.recentVaults.find((vault) => vault.rootPath === s.currentProjectId)?.hiddenFilesVisible ?? false
  ))
  const treeSignature = useMemo(() => structureSignature(currentProject?.files ?? []), [currentProject?.files])
  const visibleFiles = useMemo(() => {
    const files = currentProject?.files ?? []
    if (includeHidden) return files
    return files.filter((file) => !file.isHidden && !normalizePath(file.path).split('/').some((segment) => segment.startsWith('.')))
  }, [currentProject?.files, includeHidden])
  const tree = useMemo(() => buildTree(visibleFiles), [visibleFiles, treeSignature])
  const rows = useMemo(() => flattenTree(tree, expanded), [expanded, tree])

  useEffect(() => {
    setExpanded(() => {
      const next = new Set<string>()
      for (const node of tree) {
        if (node.kind === 'directory') next.add(node.path)
      }
      return next
    })
  }, [currentProject?.id])

  useEffect(() => {
    if (!currentFilePath) return
    setExpanded((prev) => {
      const next = new Set(prev)
      const segments = normalizePath(currentFilePath).split('/')
      for (let index = 1; index < segments.length; index += 1) {
        next.add(segments.slice(0, index).join('/'))
      }
      return next
    })
  }, [currentFilePath])

  useEffect(() => {
    const list = listRef.current
    if (!list) return
    const updateViewport = () => setViewportHeight(list.clientHeight)
    updateViewport()

    if (typeof ResizeObserver === 'undefined') return
    const observer = new ResizeObserver(updateViewport)
    observer.observe(list)
    return () => observer.disconnect()
  }, [])

  const activeDirectory = useMemo(() => {
    if (!currentFilePath) return ''
    const current = currentProject?.files.find((entry) => entry.path === currentFilePath)
    if ((current?.kind ?? 'file') === 'directory') return currentFilePath
    return parentPath(currentFilePath) ?? ''
  }, [currentFilePath, currentProject?.files])

  const promptForFile = async (baseDirectory = activeDirectory) => {
    const existing = (currentProject?.files ?? [])
      .filter((entry) => (entry.kind ?? 'file') === 'file')
      .map((entry) => entry.path)
    const suggestion = suggestUntitledTypstPath(existing, baseDirectory)
    const nextPath = window.prompt('New file path', suggestion)?.trim()
    if (nextPath) {
      try {
        await createFile(nextPath)
      } catch (error) {
        window.alert(error instanceof Error ? error.message : 'Failed to create file.')
      }
    }
  }

  const promptForFolder = async (baseDirectory = activeDirectory) => {
    const suggestion = baseDirectory ? `${baseDirectory}/notes` : 'notes'
    const nextPath = window.prompt('New folder path', suggestion)?.trim()
    if (nextPath) {
      try {
        await createFolder(nextPath)
      } catch (error) {
        window.alert(error instanceof Error ? error.message : 'Failed to create folder.')
      }
    }
  }

  const renameNode = async (node: TreeNode) => {
    const currentName = basename(node.path)
    const nextName = window.prompt('Rename', currentName)?.trim()
    if (!nextName || nextName === currentName) return

    const parent = parentPath(node.path)
    const nextPath = parent ? `${parent}/${nextName}` : nextName
    if (node.kind === 'directory') {
      try {
        await renameFolder(node.path, nextPath)
      } catch (error) {
        window.alert(error instanceof Error ? error.message : 'Failed to rename folder.')
      }
    } else {
      try {
        await renameFile(node.path, nextPath)
      } catch (error) {
        window.alert(error instanceof Error ? error.message : 'Failed to rename file.')
      }
    }
  }

  const duplicateNode = async (node: TreeNode) => {
    if (node.kind !== 'file' || !currentProject) return
    const nextPath = buildDuplicatePath(
      currentProject.files
        .filter((entry) => (entry.kind ?? 'file') === 'file')
        .map((entry) => entry.path),
      node.path,
    )
    try {
      await duplicateFile(node.path, nextPath)
    } catch (error) {
      window.alert(error instanceof Error ? error.message : 'Failed to duplicate file.')
    }
  }

  const moveNodeTo = async (sourcePath: string, sourceKind: 'file' | 'directory', targetDir: string) => {
    const source = normalizePath(sourcePath)
    const destinationDir = normalizePath(targetDir)
    if (!source) return
    if (destinationDir === (parentPath(source) ?? '')) return
    if (sourceKind === 'directory' && (destinationDir === source || destinationDir.startsWith(`${source}/`))) {
      return
    }

    const nextPath = destinationDir ? `${destinationDir}/${basename(source)}` : basename(source)
    const existingPaths = new Set(
      (currentProject?.files ?? []).map((entry) => normalizePath(entry.path)),
    )
    if (existingPaths.has(nextPath)) {
      window.alert(`"${nextPath}" already exists.`)
      return
    }

    try {
      if (sourceKind === 'directory') await renameFolder(source, nextPath)
      else await moveFile(source, nextPath)
    } catch (error) {
      window.alert(error instanceof Error ? error.message : 'Failed to move.')
    }
  }

  const readInternalDrag = (dataTransfer: DataTransfer): { path: string; kind: 'file' | 'directory' } | null => {
    const raw = dataTransfer.getData(INTERNAL_DND_TYPE)
    if (!raw) return null
    try {
      const parsed = JSON.parse(raw) as { path?: unknown; kind?: unknown }
      if (typeof parsed.path !== 'string') return null
      return { path: parsed.path, kind: parsed.kind === 'directory' ? 'directory' : 'file' }
    } catch {
      return null
    }
  }

  const actionsForNode = (node: TreeNode): ContextMenuAction[] => [
    {
      label: revealLabel,
      icon: <FolderSearch size={12} />,
      onClick: () => {
        void revealPathInFinder(node.path)
      },
    },
    ...(node.kind === 'file'
      ? [
          {
            label: 'Open in default app',
            icon: <ExternalLink size={12} />,
            onClick: () => {
              void openPathInDefaultApp(node.path)
            },
          },
        ]
      : []),
    {
      label: 'Rename',
      icon: <Pencil size={12} />,
      onClick: () => {
        void renameNode(node)
      },
    },
    ...(node.kind === 'file'
      ? [
          {
            label: 'Duplicate',
            icon: <Copy size={12} />,
            onClick: () => {
              void duplicateNode(node)
            },
          },
        ]
      : []),
    ...(node.kind === 'directory'
      ? [
          {
            label: 'New file here',
            icon: <FilePlus2 size={12} />,
            onClick: () => {
              void promptForFile(node.path)
            },
          },
          {
            label: 'New folder here',
            icon: <FolderPlus size={12} />,
            onClick: () => {
              void promptForFolder(node.path)
            },
          },
        ]
      : []),
    {
      label: 'Delete',
      icon: <Trash2 size={12} />,
      danger: true,
      onClick: () => {
        if (!window.confirm(`Delete ${node.name}?`)) return
        void (async () => {
          try {
            if (node.kind === 'directory') await deleteFolder(node.path)
            else await deleteFile(node.path)
          } catch (error) {
            window.alert(error instanceof Error ? error.message : 'Failed to delete.')
          }
        })()
      },
    },
  ]

  const shouldVirtualize = rows.length > VIRTUALIZE_THRESHOLD
  const totalHeight = rows.length * ROW_HEIGHT
  const windowStart = shouldVirtualize ? Math.max(0, Math.floor(scrollTop / ROW_HEIGHT) - OVERSCAN) : 0
  const windowEnd = shouldVirtualize
    ? Math.min(rows.length, Math.ceil((scrollTop + viewportHeight) / ROW_HEIGHT) + OVERSCAN)
    : rows.length
  const visibleRows = rows.slice(windowStart, windowEnd)

  if (!currentProject) {
    return (
      <div className="flex h-full items-center justify-center px-6 text-center" style={{ color: 'var(--text-secondary)', fontSize: '13px' }}>
        Open a project to browse files.
      </div>
    )
  }

  return (
    <div
      className="relative flex h-full flex-col"
      onDragOver={(event) => {
        event.preventDefault()
        if (event.dataTransfer.types.includes(INTERNAL_DND_TYPE)) {
          // Rows stop propagation, so reaching here means the drag is over empty space → vault root.
          setDropTargetDir('')
          return
        }
        setDragActive(true)
      }}
      onDragLeave={(event) => {
        if (event.currentTarget.contains(event.relatedTarget as Node)) return
        setDragActive(false)
        setDropTargetDir(null)
      }}
      onDrop={(event) => {
        event.preventDefault()
        setDragActive(false)
        const internal = readInternalDrag(event.dataTransfer)
        const targetDir = dropTargetDir ?? ''
        setDropTargetDir(null)
        if (internal) {
          void moveNodeTo(internal.path, internal.kind, targetDir)
          return
        }
        if (event.dataTransfer.files.length === 0) return
        void processImportedFiles(event.dataTransfer.files, activeDirectory)
      }}
    >
      {/* Project info bar */}
      <div
        className="shrink-0"
        style={{ borderBottom: '1px solid var(--border-subtle)', padding: '14px 20px' }}
      >
        <div
          className="truncate"
          style={{ color: 'var(--text-primary)', fontSize: '13px', fontWeight: 600 }}
          title={currentProject.name}
        >
          {currentProject.name}
        </div>
        <div
          className="truncate"
          style={{ color: 'var(--text-tertiary)', fontSize: '10px', fontFamily: 'var(--font-mono)', marginTop: '3px' }}
          title={currentProject.rootPath}
        >
          {currentProject.rootPath}
        </div>
      </div>

      {/* File actions bar */}
      <div
        className="shrink-0 flex items-center gap-1 px-3 py-1.5"
        style={{ borderBottom: '1px solid var(--border-subtle)' }}
      >
        <button className="toolbar-button" style={{ width: '26px', height: '26px' }} title="New file" onClick={() => void promptForFile()}>
          <FilePlus2 size={13} />
        </button>
        <button className="toolbar-button" style={{ width: '26px', height: '26px' }} title="New folder" onClick={() => void promptForFolder()}>
          <FolderPlus size={13} />
        </button>
        <button className="toolbar-button" style={{ width: '26px', height: '26px' }} title="Import files" onClick={() => fileInputRef.current?.click()}>
          <Upload size={13} />
        </button>
        <button
          className="toolbar-button"
          style={{ width: '26px', height: '26px' }}
          title={revealLabel}
          onClick={() => void useProjectStore.getState().revealCurrentProjectInFinder()}
        >
          <FolderSearch size={13} />
        </button>
      </div>

      <input
        ref={fileInputRef}
        type="file"
        multiple
        className="hidden"
        onChange={(event) => {
          const files = event.target.files
          if (files?.length) void processImportedFiles(files, activeDirectory)
          event.currentTarget.value = ''
        }}
      />

      <div ref={listRef} className="min-h-0 flex-1 overflow-y-auto py-3" onScroll={(event) => setScrollTop(event.currentTarget.scrollTop)}>
        {(dragActive || dropTargetDir === '') && (
          <div
            className="pointer-events-none absolute inset-3 z-20 rounded-[20px] border-2 border-dashed"
            style={{
              borderColor: 'color-mix(in srgb, var(--accent) 70%, transparent)',
              background: 'color-mix(in srgb, var(--accent) 10%, transparent)',
            }}
          />
        )}

        <div style={{ height: shouldVirtualize ? `${totalHeight}px` : 'auto', position: 'relative' }}>
          {visibleRows.map((row, visibleIndex) => {
            const index = windowStart + visibleIndex
            const isDirectory = row.node.kind === 'directory'
            const isExpanded = expanded.has(row.node.path)
            const isActive = currentFilePath === row.node.path
            const rowDropDir = isDirectory ? row.node.path : (parentPath(row.node.path) ?? '')
            const isDropTarget = dropTargetDir !== null && dropTargetDir !== '' && dropTargetDir === rowDropDir && isDirectory
            const Icon = isDirectory ? (isExpanded ? FolderOpen : Folder) : fileIcon(row.node.name)

            return (
              <button
                key={row.node.path}
                type="button"
                className="file-tree-row flex w-full items-center gap-2 px-3 text-left transition"
                data-active={isActive || undefined}
                draggable
                onDragStart={(event) => {
                  event.dataTransfer.effectAllowed = 'move'
                  event.dataTransfer.setData(
                    INTERNAL_DND_TYPE,
                    JSON.stringify({ path: row.node.path, kind: row.node.kind }),
                  )
                }}
                onDragEnd={() => {
                  setDragActive(false)
                  setDropTargetDir(null)
                }}
                onDragOver={(event) => {
                  if (!event.dataTransfer.types.includes(INTERNAL_DND_TYPE)) return
                  event.preventDefault()
                  event.stopPropagation()
                  event.dataTransfer.dropEffect = 'move'
                  setDropTargetDir(rowDropDir)
                }}
                style={{
                  position: shouldVirtualize ? 'absolute' : 'relative',
                  left: 0,
                  right: 0,
                  top: shouldVirtualize ? `${index * ROW_HEIGHT}px` : undefined,
                  height: `${ROW_HEIGHT}px`,
                  paddingLeft: `${INDENT_BASE + row.depth * INDENT_STEP}px`,
                  paddingRight: '10px',
                  background: isActive
                    ? 'color-mix(in srgb, var(--accent) 12%, transparent)'
                    : isDropTarget
                      ? 'color-mix(in srgb, var(--accent) 18%, transparent)'
                      : 'transparent',
                  borderLeft: `${BORDER_LEFT}px solid ${isActive ? 'var(--accent)' : 'transparent'}`,
                  color: isActive ? 'var(--text-primary)' : 'var(--text-secondary)',
                }}
                onContextMenu={(event) => {
                  event.preventDefault()
                  setContextMenu({ x: event.clientX, y: event.clientY, node: row.node })
                }}
                onClick={() => {
                  if (isDirectory) {
                    setExpanded((current) => {
                      const next = new Set(current)
                      if (next.has(row.node.path)) next.delete(row.node.path)
                      else next.add(row.node.path)
                      return next
                    })
                    return
                  }
                  if (isPreviewableImagePath(row.node.path)) {
                    setImagePreviewPath(row.node.path)
                    return
                  }
                  selectFile(row.node.path)
                }}
              >
                <IndentGuides depth={row.depth} />
                {isDirectory ? (
                  isExpanded ? <ChevronDown size={13} /> : <ChevronRight size={13} />
                ) : (
                  <span style={{ width: '13px' }} />
                )}
                <Icon size={14} style={{ color: isActive ? 'var(--accent)' : 'var(--text-tertiary)', flexShrink: 0 }} />
                <span className="truncate" style={{ fontSize: '13px' }}>
                  {row.node.name}
                </span>
              </button>
            )
          })}
        </div>
      </div>

      {contextMenu && (
        <ContextMenu
          x={contextMenu.x}
          y={contextMenu.y}
          actions={actionsForNode(contextMenu.node)}
          onClose={() => setContextMenu(null)}
        />
      )}
    </div>
  )
}
