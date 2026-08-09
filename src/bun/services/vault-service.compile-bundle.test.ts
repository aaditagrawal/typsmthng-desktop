import { beforeEach, describe, expect, it, vi } from 'vitest'
import fs from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'

const appStateLoadMock = vi.fn()
const getIndexMock = vi.fn()
const moveToTrashMock = vi.fn()

class MockAppStateService {
  load = appStateLoadMock
}

class MockBackgroundTaskQueue {
  drain = vi.fn().mockResolvedValue(undefined)
  enqueue = vi.fn(async (task: () => Promise<void>) => {
    await task()
  })
}

class MockVaultIndexService {
  getIndex = getIndexMock
  invalidate = vi.fn()
}

class MockFullTextSearchService {
  constructor(_indexService: unknown) {}
}

vi.mock('electrobun/bun', () => ({
  BrowserView: {
    defineRPC: vi.fn(),
  },
  BrowserWindow: class {},
  Utils: {
    openFileDialog: vi.fn(),
    moveToTrash: (...args: unknown[]) => moveToTrashMock(...args),
    paths: {
      documents: '/tmp',
      userData: '/tmp',
    },
  },
}))

vi.mock('./app-state', () => ({
  AppStateService: MockAppStateService,
}))

vi.mock('./background-task-queue', () => ({
  BackgroundTaskQueue: MockBackgroundTaskQueue,
}))

vi.mock('./vault-index', () => ({
  VaultIndexService: MockVaultIndexService,
}))

vi.mock('./full-text-search', () => ({
  FullTextSearchService: MockFullTextSearchService,
}))

vi.mock('chokidar', () => ({
  default: {
    watch: vi.fn(),
  },
}))

async function loadModule() {
  vi.resetModules()
  return import('./vault-service')
}

function fileEntry(path: string, partial: Partial<Record<string, unknown>> = {}) {
  return {
    path,
    name: path.split('/').pop() ?? path,
    kind: 'file',
    parentPath: path.includes('/') ? path.slice(0, path.lastIndexOf('/')) : null,
    extension: path.includes('.') ? path.slice(path.lastIndexOf('.')).toLowerCase() : '',
    isHidden: false,
    isBinary: false,
    lastModified: 1,
    sizeBytes: 32,
    loaded: true,
    content: '',
    ...partial,
  }
}

type PendingWriteMap = Map<string, {
  rootPath: string
  filePath: string
  content: string
  queuedAt: number
  timer: ReturnType<typeof setTimeout>
}>

describe('VaultService.getCompileBundle', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    appStateLoadMock.mockResolvedValue({
      version: 1,
      recentVaults: [
        {
          id: '/vault',
          rootPath: '/vault',
          name: 'vault',
          favorite: false,
          hiddenFilesVisible: false,
          lastOpenedAt: 1,
          lastFilePath: null,
          recentDocuments: [],
        },
      ],
      reopenLastVaultPath: null,
      windowState: null,
    })
  })

  it('uses the live source as the main entry and excludes it from extra files', async () => {
    getIndexMock.mockResolvedValue({
      entries: [
        fileEntry('main.typ'),
        fileEntry('chapter.typ'),
        fileEntry('assets/logo.png', { isBinary: true, extension: '.png', sizeBytes: 128 }),
      ],
    })

    const { VaultService } = await loadModule()
    const service = new VaultService() as unknown as {
      getCompileBundle: (rootPath: string, currentFilePath: string | null, liveSource: string) => Promise<unknown>
      readFileEntry: ReturnType<typeof vi.fn>
    }

    service.readFileEntry = vi.fn(async (_rootPath: string, path: string) => {
      if (path === 'main.typ') return fileEntry('main.typ', { content: 'stale main' })
      if (path === 'chapter.typ') return fileEntry('chapter.typ', { content: 'chapter body' })
      if (path === 'assets/logo.png') {
        return fileEntry('assets/logo.png', {
          isBinary: true,
          extension: '.png',
          binaryData: new Uint8Array([1, 2, 3]),
        })
      }
      return null
    })

    const bundle = await service.getCompileBundle('/vault', 'main.typ', 'live main source') as {
      mainPath: string
      mainSource: string
      extraFiles: Array<{ path: string; content: string }>
      extraBinaryFiles: Array<{ path: string; data: Uint8Array }>
    }

    expect(getIndexMock).toHaveBeenCalledWith('/vault', false)
    expect(service.readFileEntry).toHaveBeenCalledWith('/vault', 'main.typ', true)
    expect(service.readFileEntry).toHaveBeenCalledWith('/vault', 'chapter.typ', true)
    expect(service.readFileEntry).toHaveBeenCalledWith('/vault', 'assets/logo.png', true)
    expect(bundle.mainPath).toBe('/main.typ')
    expect(bundle.mainSource).toBe('live main source')
    expect(bundle.extraFiles).toEqual([{ path: '/chapter.typ', content: 'chapter body' }])
    expect(bundle.extraBinaryFiles).toEqual([{ path: '/assets/logo.png', data: new Uint8Array([1, 2, 3]) }])
  })

  it('prefers pendingWrites content for non-main files', async () => {
    getIndexMock.mockResolvedValue({
      entries: [
        fileEntry('main.typ'),
        fileEntry('chapter.typ'),
      ],
    })

    const { VaultService } = await loadModule()
    const service = new VaultService() as unknown as {
      getCompileBundle: (rootPath: string, currentFilePath: string | null, liveSource: string) => Promise<unknown>
      readFileEntry: ReturnType<typeof vi.fn>
      stageFileWrite: (rootPath: string, filePath: string, content: string) => Promise<{ queuedAt: number }>
      pendingWrites: PendingWriteMap
    }

    service.readFileEntry = vi.fn(async (_rootPath: string, filePath: string) => {
      if (filePath === 'main.typ') return fileEntry('main.typ', { content: 'stale main' })
      if (filePath === 'chapter.typ') return fileEntry('chapter.typ', { content: 'disk chapter' })
      return null
    })

    await service.stageFileWrite('/vault', 'chapter.typ', 'pending chapter')

    const bundle = await service.getCompileBundle('/vault', 'main.typ', 'live main source') as {
      extraFiles: Array<{ path: string; content: string }>
    }

    expect(bundle.extraFiles).toEqual([{ path: '/chapter.typ', content: 'pending chapter' }])

    for (const pending of service.pendingWrites.values()) {
      clearTimeout(pending.timer)
    }
    service.pendingWrites.clear()
  })

  it('uses the persisted recent file when no explicit current file is provided', async () => {
    appStateLoadMock.mockResolvedValue({
      version: 1,
      recentVaults: [
        {
          id: '/vault',
          rootPath: '/vault',
          name: 'vault',
          favorite: false,
          hiddenFilesVisible: true,
          lastOpenedAt: 1,
          lastFilePath: 'notes/intro.typ',
          recentDocuments: [],
        },
      ],
      reopenLastVaultPath: null,
      windowState: null,
    })
    getIndexMock.mockResolvedValue({
      entries: [
        fileEntry('notes/intro.typ'),
        fileEntry('main.typ'),
      ],
    })

    const { VaultService } = await loadModule()
    const service = new VaultService() as unknown as {
      getCompileBundle: (rootPath: string, currentFilePath: string | null, liveSource: string) => Promise<unknown>
      readFileEntry: ReturnType<typeof vi.fn>
    }

    service.readFileEntry = vi.fn(async (_rootPath: string, path: string) => {
      if (path === 'notes/intro.typ') return fileEntry('notes/intro.typ', { content: 'recent file body' })
      if (path === 'main.typ') return fileEntry('main.typ', { content: 'main body' })
      return null
    })

    const bundle = await service.getCompileBundle('/vault', null, 'fallback live source') as {
      mainPath: string
      mainSource: string
      extraFiles: Array<{ path: string; content: string }>
    }

    expect(getIndexMock).toHaveBeenCalledWith('/vault', true)
    expect(bundle.mainPath).toBe('/notes/intro.typ')
    expect(bundle.mainSource).toBe('recent file body')
    expect(bundle.extraFiles).toEqual([{ path: '/main.typ', content: 'main body' }])
  })
})

describe('VaultService.readFileEntry / pendingWrites path ops', () => {
  beforeEach(() => {
    vi.clearAllMocks()
  })

  it('hydrates text from disk instead of returning unloaded cache entries', async () => {
    const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), 'typsmthng-compile-'))
    const filePath = 'chapter.typ'
    const absolutePath = path.join(tempRoot, filePath)
    await fs.writeFile(absolutePath, 'hydrated body', 'utf8')
    const stat = await fs.stat(absolutePath)

    const { VaultService } = await loadModule()
    const service = new VaultService() as unknown as {
      contentCache: Map<string, Map<string, { entry: Record<string, unknown>; mtimeMs: number }>>
      readFileEntry: (rootPath: string, filePath: string, hydrateContent: boolean) => Promise<{
        content: string
        loaded: boolean
      } | null>
    }

    service.contentCache.set(tempRoot, new Map([
      [filePath, {
        mtimeMs: stat.mtimeMs,
        entry: {
          path: filePath,
          name: 'chapter.typ',
          kind: 'file',
          parentPath: null,
          extension: '.typ',
          isHidden: false,
          isBinary: false,
          lastModified: stat.mtimeMs,
          sizeBytes: stat.size,
          loaded: false,
          content: '',
        },
      }],
    ]))

    const unloaded = await service.readFileEntry(tempRoot, filePath, false)
    expect(unloaded).toMatchObject({ loaded: false, content: '' })

    const hydrated = await service.readFileEntry(tempRoot, filePath, true)
    expect(hydrated).toMatchObject({ loaded: true, content: 'hydrated body' })

    await fs.rm(tempRoot, { recursive: true, force: true })
  })

  it('clears pendingWrites on deletePath and migrates them on renamePath', async () => {
    const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), 'typsmthng-pending-'))
    await fs.mkdir(path.join(tempRoot, 'notes'), { recursive: true })
    await fs.writeFile(path.join(tempRoot, 'notes', 'a.typ'), 'a', 'utf8')
    await fs.writeFile(path.join(tempRoot, 'notes', 'b.typ'), 'b', 'utf8')

    const { VaultService } = await loadModule()
    const service = new VaultService() as unknown as {
      stageFileWrite: (rootPath: string, filePath: string, content: string) => Promise<{ queuedAt: number }>
      deletePath: (rootPath: string, filePath: string) => Promise<{ ok: true }>
      renamePath: (rootPath: string, oldPath: string, newPath: string) => Promise<{ ok: true }>
      pendingWrites: PendingWriteMap
    }

    await service.stageFileWrite(tempRoot, 'notes/a.typ', 'pending-a')
    await service.stageFileWrite(tempRoot, 'notes/b.typ', 'pending-b')

    await service.deletePath(tempRoot, 'notes/a.typ')
    expect(service.pendingWrites.has(`${tempRoot}::notes/a.typ`)).toBe(false)
    expect(service.pendingWrites.has(`${tempRoot}::notes/b.typ`)).toBe(true)

    await service.renamePath(tempRoot, 'notes/b.typ', 'notes/renamed.typ')
    expect(service.pendingWrites.has(`${tempRoot}::notes/b.typ`)).toBe(false)
    expect(service.pendingWrites.get(`${tempRoot}::notes/renamed.typ`)?.content).toBe('pending-b')

    await service.stageFileWrite(tempRoot, 'notes/renamed.typ', 'pending-folder-child')
    await service.renamePath(tempRoot, 'notes', 'chapters')
    expect(service.pendingWrites.has(`${tempRoot}::notes/renamed.typ`)).toBe(false)
    expect(service.pendingWrites.get(`${tempRoot}::chapters/renamed.typ`)?.content).toBe('pending-folder-child')

    for (const pending of service.pendingWrites.values()) {
      clearTimeout(pending.timer)
    }
    service.pendingWrites.clear()
    await fs.rm(tempRoot, { recursive: true, force: true })
  })

  it('keeps pendingWrites on flush failure and retries successfully', async () => {
    const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), 'typsmthng-flush-'))
    const filePath = 'draft.typ'
    const absolutePath = path.join(tempRoot, filePath)

    const { VaultService } = await loadModule()
    const service = new VaultService() as unknown as {
      stageFileWrite: (rootPath: string, filePath: string, content: string) => Promise<{ queuedAt: number }>
      flushWrites: (input: { rootPath?: string; path?: string }) => Promise<{ ok: true }>
      pendingWrites: PendingWriteMap
    }

    await service.stageFileWrite(tempRoot, filePath, 'first body')
    const key = `${tempRoot}::${filePath}`
    expect(service.pendingWrites.has(key)).toBe(true)

    const writeSpy = vi.spyOn(fs, 'writeFile')
      .mockRejectedValueOnce(new Error('disk busy'))
      .mockImplementation(async (target, content, encoding) => {
        writeSpy.mockRestore()
        await fs.writeFile(target, content, encoding as BufferEncoding)
      })

    await expect(service.flushWrites({ rootPath: tempRoot, path: filePath })).rejects.toThrow('disk busy')
    expect(service.pendingWrites.has(key)).toBe(true)
    expect(service.pendingWrites.get(key)?.content).toBe('first body')

    await service.flushWrites({ rootPath: tempRoot, path: filePath })
    expect(service.pendingWrites.has(key)).toBe(false)
    expect(await fs.readFile(absolutePath, 'utf8')).toBe('first body')

    await fs.rm(tempRoot, { recursive: true, force: true })
  })
})
