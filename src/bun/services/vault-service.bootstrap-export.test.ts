import { beforeEach, describe, expect, it, vi } from 'vitest'
import fs from 'node:fs/promises'
import os from 'node:os'
import path from 'node:path'

const appStateLoadMock = vi.fn()
const appStateUpdateMock = vi.fn()
const appStateUpsertMock = vi.fn()
const appStatePersistLastFileMock = vi.fn()
const appStateClearReopenMock = vi.fn()
const getIndexMock = vi.fn()
const invalidateMock = vi.fn()

class MockAppStateService {
  load = appStateLoadMock
  update = appStateUpdateMock
  upsertRecentVault = appStateUpsertMock
  persistLastFile = appStatePersistLastFileMock
  clearReopenLastVaultPathLocally = appStateClearReopenMock
  save = vi.fn(async (metadata: unknown) => metadata)
  removeRecentVault = vi.fn()
  toggleFavoriteVault = vi.fn()
  setWindowState = vi.fn()
}

class MockBackgroundTaskQueue {
  drain = vi.fn().mockResolvedValue(undefined)
  enqueue = vi.fn(async (task: () => Promise<void>) => {
    await task()
  })
}

class MockVaultIndexService {
  getIndex = getIndexMock
  invalidate = invalidateMock
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

function baseMetadata(overrides: Record<string, unknown> = {}) {
  return {
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
        fileCount: 1,
      },
    ],
    reopenLastVaultPath: null,
    windowState: null,
    ...overrides,
  }
}

describe('VaultService.getBootstrapState / getVaultExportBundle', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    appStateLoadMock.mockResolvedValue(baseMetadata())
    appStateUpdateMock.mockImplementation(async (recipe: (current: unknown) => unknown) => {
      const next = recipe(baseMetadata())
      return next
    })
    appStateUpsertMock.mockResolvedValue(baseMetadata())
    appStatePersistLastFileMock.mockResolvedValue(baseMetadata())
  })

  it('does not clear startupVaultOverride during restoreActive=false refresh', async () => {
    getIndexMock.mockResolvedValue({
      entries: [
        {
          path: 'notes.typ',
          name: 'notes.typ',
          kind: 'file',
          parentPath: null,
          extension: '.typ',
          isHidden: false,
          isBinary: false,
          lastModified: 1,
          sizeBytes: 4,
        },
      ],
      truncated: false,
      scannedAt: Date.now(),
      includeHidden: false,
    })

    const { VaultService } = await loadModule()
    const service = new VaultService()

    service.setStartupVaultOverride('/cli-vault', 'notes.typ')
    const metadataOnly = await service.getBootstrapState({} as never, { restoreActive: false })
    expect(metadataOnly.activeVault).toBeNull()

    const openVault = vi.spyOn(
      service as unknown as { openVault: (...args: unknown[]) => Promise<unknown> },
      'openVault',
    ).mockResolvedValue({
      id: '/cli-vault',
      rootPath: '/cli-vault',
      name: 'cli-vault',
      files: [],
      mainFile: 'notes.typ',
      createdAt: 1,
      updatedAt: 1,
    })

    const restored = await service.getBootstrapState({} as never, { restoreActive: true })
    expect(openVault).toHaveBeenCalledWith(
      '/cli-vault',
      expect.anything(),
      'notes.typ',
      expect.objectContaining({ removeRecentOnFailure: false }),
    )
    expect(restored.activeVault?.rootPath).toBe('/cli-vault')
  })

  it('flushes pending writes, rejects truncated indexes, and blocks path escape', async () => {
    const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), 'typsmthng-export-'))
    await fs.writeFile(path.join(tempRoot, 'main.typ'), '= Hello\n', 'utf8')

    appStateLoadMock.mockResolvedValue(baseMetadata({
      recentVaults: [
        {
          id: tempRoot,
          rootPath: tempRoot,
          name: path.basename(tempRoot),
          favorite: false,
          hiddenFilesVisible: false,
          lastOpenedAt: 1,
          lastFilePath: 'main.typ',
          recentDocuments: [],
          fileCount: 1,
        },
      ],
    }))

    getIndexMock.mockResolvedValue({
      entries: [
        {
          path: 'main.typ',
          name: 'main.typ',
          kind: 'file',
          parentPath: null,
          extension: '.typ',
          isHidden: false,
          isBinary: false,
          lastModified: 1,
          sizeBytes: 8,
        },
      ],
      truncated: false,
      scannedAt: Date.now(),
      includeHidden: false,
    })

    const { VaultService } = await loadModule()
    const service = new VaultService()
    const flushSpy = vi.spyOn(service, 'flushWrites')

    await service.stageFileWrite(tempRoot, 'main.typ', '= Flushed\n')
    const bundle = await service.getVaultExportBundle(tempRoot)
    expect(flushSpy).toHaveBeenCalledWith({ rootPath: tempRoot })
    expect(bundle?.files).toEqual([
      { path: 'main.typ', isBinary: false, content: '= Flushed\n' },
    ])

    getIndexMock.mockResolvedValue({
      entries: [
        {
          path: 'main.typ',
          name: 'main.typ',
          kind: 'file',
          parentPath: null,
          extension: '.typ',
          isHidden: false,
          isBinary: false,
          lastModified: 1,
          sizeBytes: 8,
        },
      ],
      truncated: true,
      scannedAt: Date.now(),
      includeHidden: false,
    })
    await expect(service.getVaultExportBundle(tempRoot)).rejects.toThrow(/too many files/)

    getIndexMock.mockResolvedValue({
      entries: [
        {
          path: '../evil.typ',
          name: 'evil.typ',
          kind: 'file',
          parentPath: null,
          extension: '.typ',
          isHidden: false,
          isBinary: false,
          lastModified: 1,
          sizeBytes: 8,
        },
      ],
      truncated: false,
      scannedAt: Date.now(),
      includeHidden: false,
    })
    await expect(service.getVaultExportBundle(tempRoot)).rejects.toThrow(/escapes vault root/)

    await fs.rm(tempRoot, { recursive: true, force: true })
  })
})
