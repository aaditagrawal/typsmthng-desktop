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

  it('clears activeVaultRoot when openVault fails after activation', async () => {
    const tempRoot = await fs.mkdtemp(path.join(os.tmpdir(), 'typsmthng-open-fail-'))
    await fs.writeFile(path.join(tempRoot, 'main.typ'), '= Hello\n', 'utf8')

    const vaultMeta = {
      id: tempRoot,
      rootPath: tempRoot,
      name: path.basename(tempRoot),
      favorite: false,
      hiddenFilesVisible: false,
      lastOpenedAt: 1,
      lastFilePath: 'main.typ',
      recentDocuments: [],
      fileCount: 1,
    }
    appStateLoadMock.mockResolvedValue(baseMetadata({
      recentVaults: [vaultMeta],
      reopenLastVaultPath: null,
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
    // Fail after openVault assigns activeVaultRoot (post loadVaultSnapshot / stopWatcher).
    appStateUpsertMock.mockRejectedValueOnce(new Error('upsert failed after activation'))

    const window = {
      webview: {
        rpc: {
          send: {
            metadataUpdated: vi.fn(),
            activeVaultOpened: vi.fn(),
            activeVaultClosed: vi.fn(),
          },
        },
      },
    }

    await expect(service.openRecentVault(tempRoot, window as never)).resolves.toBeNull()

    // Zombie active root would make bootstrap return that vault instead of null.
    const bootstrap = await service.getBootstrapState(window as never, { restoreActive: true })
    expect(bootstrap.activeVault).toBeNull()
    expect(
      (service as unknown as { activeVaultRoot: string | null }).activeVaultRoot,
    ).toBeNull()

    await fs.rm(tempRoot, { recursive: true, force: true })
  })

  it('getBootstrapState tears down when openVault returns null with leftover active root', async () => {
    const { VaultService } = await loadModule()
    const service = new VaultService()
    const closeVault = vi.spyOn(service, 'closeVault').mockResolvedValue({ ok: true as const })
    appStateLoadMock.mockResolvedValue(
      baseMetadata({ reopenLastVaultPath: '/partial-vault' }),
    )
    appStateUpdateMock.mockImplementation(async (mutator: (current: unknown) => unknown) => {
      const next = mutator(baseMetadata({ reopenLastVaultPath: '/partial-vault' }))
      return next
    })

    service.setStartupVaultOverride('/partial-vault', null)
    vi.spyOn(
      service as unknown as { openVault: (...args: unknown[]) => Promise<unknown> },
      'openVault',
    ).mockImplementation(async () => {
      ;(service as unknown as { activeVaultRoot: string | null }).activeVaultRoot = '/partial-vault'
      return null
    })

    const bootstrap = await service.getBootstrapState({} as never, { restoreActive: true })
    expect(bootstrap.activeVault).toBeNull()
    expect(closeVault).toHaveBeenCalledWith({ rootPath: '/partial-vault' })
    expect(appStateUpdateMock).toHaveBeenCalled()
    expect(bootstrap.metadata.reopenLastVaultPath).toBeNull()
  })

  it('getBootstrapState clears reopen when override openVault fails after catch cleared active', async () => {
    const { VaultService } = await loadModule()
    const service = new VaultService()
    const closeVault = vi.spyOn(service, 'closeVault').mockResolvedValue({ ok: true as const })
    appStateLoadMock.mockResolvedValue(
      baseMetadata({ reopenLastVaultPath: '/failed-override' }),
    )
    appStateUpdateMock.mockImplementation(async (mutator: (current: unknown) => unknown) => {
      const next = mutator(baseMetadata({ reopenLastVaultPath: '/failed-override' }))
      return next
    })

    service.setStartupVaultOverride('/failed-override', null)
    vi.spyOn(
      service as unknown as { openVault: (...args: unknown[]) => Promise<unknown> },
      'openVault',
    ).mockResolvedValue(null)

    const bootstrap = await service.getBootstrapState({} as never, { restoreActive: true })
    expect(bootstrap.activeVault).toBeNull()
    expect(closeVault).not.toHaveBeenCalled()
    expect(bootstrap.metadata.reopenLastVaultPath).toBeNull()
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

describe('VaultService.createVault rollback', () => {
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

  it('removes a new folder when a scaffold write fails', async () => {
    const parent = await fs.mkdtemp(path.join(os.tmpdir(), 'typsmthng-create-parent-'))
    const { Utils } = await import('electrobun/bun')
    vi.mocked(Utils.openFileDialog).mockResolvedValue([parent] as never)

    const { VaultService } = await loadModule()
    const service = new VaultService()

    await expect(
      service.createVault(
        {
          name: 'Paper',
          activate: false,
          ifExists: 'fail',
          scaffold: {
            files: [
              { path: 'main.typ', content: '= Hi\n', isBinary: false },
              { path: '../evil.typ', content: 'nope', isBinary: false },
            ],
            mainFile: 'main.typ',
          },
        },
        {} as never,
      ),
    ).rejects.toThrow(/escapes vault root/)

    await expect(fs.access(path.join(parent, 'Paper'))).rejects.toMatchObject({ code: 'ENOENT' })
    await fs.rm(parent, { recursive: true, force: true })
  })

  it('removes a new folder when register fails so retry with ifExists fail can succeed', async () => {
    const parent = await fs.mkdtemp(path.join(os.tmpdir(), 'typsmthng-create-reg-'))
    const { Utils } = await import('electrobun/bun')
    vi.mocked(Utils.openFileDialog).mockResolvedValue([parent] as never)
    getIndexMock.mockRejectedValue(new Error('index failed'))

    const { VaultService } = await loadModule()
    const service = new VaultService()

    await expect(
      service.createVault(
        {
          name: 'Paper',
          activate: false,
          ifExists: 'fail',
          scaffold: {
            files: [{ path: 'main.typ', content: '= Hi\n', isBinary: false }],
            mainFile: 'main.typ',
          },
        },
        {} as never,
      ),
    ).rejects.toThrow(/Failed to register the new project|index failed/)

    await expect(fs.access(path.join(parent, 'Paper'))).rejects.toMatchObject({ code: 'ENOENT' })

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
          sizeBytes: 6,
        },
      ],
      truncated: false,
      scannedAt: Date.now(),
      includeHidden: false,
    })

    const created = await service.createVault(
      {
        name: 'Paper',
        activate: false,
        ifExists: 'fail',
        parentPath: parent,
        scaffold: {
          files: [{ path: 'main.typ', content: '= Hi\n', isBinary: false }],
          mainFile: 'main.typ',
        },
      },
      {} as never,
    )
    expect(created?.rootPath).toBe(path.join(parent, 'Paper'))
    expect(await fs.readFile(path.join(parent, 'Paper', 'main.typ'), 'utf8')).toBe('= Hi\n')

    await fs.rm(parent, { recursive: true, force: true })
  })
})
