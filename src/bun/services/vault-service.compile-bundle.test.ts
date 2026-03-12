import { beforeEach, describe, expect, it, vi } from 'vitest'

const appStateLoadMock = vi.fn()
const getIndexMock = vi.fn()

class MockAppStateService {
  load = appStateLoadMock
}

class MockBackgroundTaskQueue {
  drain = vi.fn().mockResolvedValue(undefined)
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
    expect(bundle.mainPath).toBe('/main.typ')
    expect(bundle.mainSource).toBe('live main source')
    expect(bundle.extraFiles).toEqual([{ path: '/chapter.typ', content: 'chapter body' }])
    expect(bundle.extraBinaryFiles).toEqual([{ path: '/assets/logo.png', data: new Uint8Array([1, 2, 3]) }])
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
