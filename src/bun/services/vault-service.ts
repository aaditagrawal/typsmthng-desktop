import fs from "node:fs/promises";
import path from "node:path";

import { BrowserView, BrowserWindow, Utils } from "electrobun/bun";
import chokidar, { type FSWatcher } from "chokidar";

import {
  type AppMetadata,
  type BootstrapState,
  type CompileBundle,
  type DesktopRPC,
  type ExternalVaultEvent,
  type PathSearchResult,
  type ProjectScaffold,
  type RecentVaultRecord,
  type TextSearchResult,
  type VaultFileEntry,
  type VaultRecord,
} from "../../shared/rpc";
import { isKnownTextPath, normalizeExtension } from "../../mainview/lib/file-classification";
import { SAMPLE_DOCUMENT } from "../../mainview/lib/sample-document";
import { createBuiltInTemplateScaffold, getBuiltInTemplate } from "../../mainview/lib/builtin-templates";
import { AppStateService } from "./app-state";
import { BackgroundTaskQueue } from "./background-task-queue";
import { FullTextSearchService } from "./full-text-search";
import { VaultIndexService } from "./vault-index";

const WRITE_DEBOUNCE_MS = 450;
const SUPPRESSED_WATCH_EVENT_MS = 1_250;
const MAX_EAGER_TEXT_BYTES = 512 * 1024;
const MAX_EAGER_BINARY_BYTES = 8 * 1024 * 1024;

interface PendingWrite {
  rootPath: string;
  filePath: string;
  content: string;
  queuedAt: number;
  timer: ReturnType<typeof setTimeout>;
}

interface CachedFile {
  entry: VaultFileEntry;
  mtimeMs: number;
}

const IMAGE_EXTENSION_SET = new Set([
  ".png",
  ".jpg",
  ".jpeg",
  ".gif",
  ".webp",
  ".svg",
  ".bmp",
  ".avif",
  ".tiff",
]);

type DesktopBunRPC = ReturnType<typeof BrowserView.defineRPC<DesktopRPC>>;
type DesktopWindow = BrowserWindow<DesktopBunRPC>;

function toPosixPath(input: string): string {
  return input.split(path.sep).join("/");
}

function parentPathOf(input: string): string | null {
  const separatorIndex = input.lastIndexOf("/");
  if (separatorIndex === -1) return null;
  return input.slice(0, separatorIndex) || null;
}

function basenameOf(input: string): string {
  const separatorIndex = input.lastIndexOf("/");
  return separatorIndex === -1 ? input : input.slice(separatorIndex + 1);
}

function normalizeRelativePath(rootPath: string, absolutePath: string): string | null {
  const relative = toPosixPath(path.relative(rootPath, absolutePath));
  if (!relative || relative === "." || relative.startsWith("../") || relative === "..") {
    return null;
  }
  return relative;
}

function sanitizeRelativePath(input: string): string {
  return input.replace(/^[/\\]+/, "");
}

function toWorkspacePath(input: string): string {
  const sanitized = sanitizeRelativePath(toPosixPath(input));
  return sanitized ? `/${sanitized}` : "/";
}

function isHiddenPath(relativePath: string): boolean {
  return relativePath.split("/").some((segment) => segment.startsWith("."));
}

function shouldLoadBinary(relativePath: string, sizeBytes: number): boolean {
  return IMAGE_EXTENSION_SET.has(normalizeExtension(relativePath)) && sizeBytes <= MAX_EAGER_BINARY_BYTES;
}

function defaultMainFile(files: VaultFileEntry[], recent?: RecentVaultRecord): string {
  const preferred = recent?.lastFilePath && files.some((file) => file.path === recent.lastFilePath)
    ? recent.lastFilePath
    : null;
  if (preferred) return preferred;
  const mainTyp = files.find((file) => file.path === "main.typ");
  if (mainTyp) return mainTyp.path;
  const firstTyp = files.find((file) => file.kind === "file" && file.extension === ".typ");
  if (firstTyp) return firstTyp.path;
  const firstText = files.find((file) => file.kind === "file" && !file.isBinary);
  if (firstText) return firstText.path;
  return files.find((file) => file.kind === "file")?.path ?? "main.typ";
}

function countVisibleFiles(entries: Array<{ kind: "file" | "directory"; path: string }>): number {
  return entries.filter(
    (entry) =>
      entry.kind === "file"
      && entry.path !== ".folder"
      && !entry.path.endsWith("/.folder")
      && !entry.path.startsWith(".typsmthng/"),
  ).length;
}

export class VaultService {
  private readonly indexService = new VaultIndexService();
  private readonly searchService = new FullTextSearchService(this.indexService);
  private readonly appState = new AppStateService();
  private readonly writeQueue = new BackgroundTaskQueue();
  private readonly pendingWrites = new Map<string, PendingWrite>();
  private readonly contentCache = new Map<string, Map<string, CachedFile>>();
  private readonly suppressedWatchPaths = new Map<string, number>();

  private watcher: FSWatcher | null = null;
  private activeVaultRoot: string | null = null;
  private activeWindow: DesktopWindow | null = null;
  private watcherBatch: ExternalVaultEvent[] = [];
  private watcherFlushTimer: ReturnType<typeof setTimeout> | null = null;

  async waitUntilReady(): Promise<{ ready: true }> {
    await this.appState.load();
    return { ready: true as const };
  }

  async getBootstrapState(): Promise<BootstrapState> {
    const metadata = await this.hydrateRecentVaultMetadata(await this.appState.load());
    return {
      metadata,
      activeVault: null,
    };
  }

  async openVaultDialog(window: DesktopWindow): Promise<VaultRecord | null> {
    const [selectedPath] = await Utils.openFileDialog({
      startingFolder: Utils.paths.documents,
      allowedFileTypes: "*",
      canChooseFiles: false,
      canChooseDirectory: true,
      allowsMultipleSelection: false,
    });

    if (!selectedPath) return null;
    return this.openVault(selectedPath, window);
  }

  async openRecentVault(rootPath: string, window: DesktopWindow): Promise<VaultRecord | null> {
    return this.openVault(rootPath, window);
  }

  async createVault(
    params: { name: string; scaffold?: ProjectScaffold },
    window: DesktopWindow,
  ): Promise<VaultRecord | null> {
    const name = params.name.trim();
    if (!name) return null;

    const [selectedParent] = await Utils.openFileDialog({
      startingFolder: Utils.paths.documents,
      allowedFileTypes: "*",
      canChooseFiles: false,
      canChooseDirectory: true,
      allowsMultipleSelection: false,
    });

    if (!selectedParent) return null;

    const rootPath = path.join(selectedParent, name);
    await fs.mkdir(rootPath, { recursive: true });

    const scaffold = params.scaffold ?? this.createBlankScaffold(name);
    for (const file of scaffold.files) {
      const absolutePath = path.join(rootPath, sanitizeRelativePath(file.path));
      await fs.mkdir(path.dirname(absolutePath), { recursive: true });
      if (file.isBinary && file.binaryData) {
        await fs.writeFile(absolutePath, file.binaryData);
      } else {
        await fs.writeFile(absolutePath, file.content, "utf8");
      }
    }

    return this.openVault(rootPath, window);
  }

  async closeVault(): Promise<{ ok: true }> {
    this.activeWindow?.webview.rpc?.send.activeVaultClosed();
    await this.stopWatcher();
    this.activeVaultRoot = null;
    this.activeWindow = null;
    return { ok: true };
  }

  async readFile(rootPath: string, filePath: string): Promise<VaultFileEntry | null> {
    return this.readFileEntry(rootPath, filePath, true);
  }

  async stageFileWrite(rootPath: string, filePath: string, content: string): Promise<{ queuedAt: number }> {
    const key = `${rootPath}::${filePath}`;
    const queuedAt = Date.now();
    const existing = this.pendingWrites.get(key);
    if (existing) {
      clearTimeout(existing.timer);
    }

    const timer = setTimeout(() => {
      void this.flushWrite(key);
    }, WRITE_DEBOUNCE_MS);

    this.pendingWrites.set(key, {
      rootPath,
      filePath,
      content,
      queuedAt,
      timer,
    });

    return { queuedAt };
  }

  async flushWrites(input: { rootPath?: string; path?: string }): Promise<{ ok: true }> {
    const pendingKeys = [...this.pendingWrites.keys()].filter((key) => {
      const pending = this.pendingWrites.get(key);
      if (!pending) return false;
      if (input.rootPath && pending.rootPath !== input.rootPath) return false;
      if (input.path && pending.filePath !== input.path) return false;
      return true;
    });

    await Promise.all(pendingKeys.map((key) => this.flushWrite(key)));
    await this.writeQueue.drain();
    return { ok: true };
  }

  async createFile(rootPath: string, filePath: string, content = ""): Promise<VaultFileEntry | null> {
    const absolutePath = path.join(rootPath, filePath);
    await fs.mkdir(path.dirname(absolutePath), { recursive: true });
    await fs.writeFile(absolutePath, content, "utf8");
    this.indexService.invalidate(rootPath);
    this.contentCache.get(rootPath)?.delete(filePath);
    return this.readFileEntry(rootPath, filePath, true);
  }

  async createFilesBatch(
    rootPath: string,
    entries: Array<{ path: string; content: string }>,
  ): Promise<{ ok: true }> {
    for (const entry of entries) {
      await this.createFile(rootPath, entry.path, entry.content);
    }
    return { ok: true };
  }

  async addBinaryFilesBatch(
    rootPath: string,
    entries: Array<{ path: string; data: Uint8Array }>,
  ): Promise<{ ok: true }> {
    for (const entry of entries) {
      const absolutePath = path.join(rootPath, entry.path);
      await fs.mkdir(path.dirname(absolutePath), { recursive: true });
      await fs.writeFile(absolutePath, entry.data);
      this.contentCache.get(rootPath)?.delete(entry.path);
    }
    this.indexService.invalidate(rootPath);
    return { ok: true };
  }

  async createFolder(rootPath: string, folderPath: string): Promise<{ ok: true }> {
    await fs.mkdir(path.join(rootPath, folderPath), { recursive: true });
    this.indexService.invalidate(rootPath);
    return { ok: true };
  }

  async renamePath(rootPath: string, oldPath: string, newPath: string): Promise<{ ok: true }> {
    await fs.mkdir(path.dirname(path.join(rootPath, newPath)), { recursive: true });
    await fs.rename(path.join(rootPath, oldPath), path.join(rootPath, newPath));
    this.indexService.invalidate(rootPath);
    const cache = this.contentCache.get(rootPath);
    if (cache) {
      const cached = cache.get(oldPath);
      if (cached) {
        cache.delete(oldPath);
        cache.set(newPath, {
          ...cached,
          entry: { ...cached.entry, path: newPath, name: basenameOf(newPath), parentPath: parentPathOf(newPath) },
        });
      }
    }
    return { ok: true };
  }

  async deletePath(rootPath: string, filePath: string): Promise<{ ok: true }> {
    Utils.moveToTrash(path.join(rootPath, filePath));
    this.indexService.invalidate(rootPath);
    this.contentCache.get(rootPath)?.delete(filePath);
    return { ok: true };
  }

  async revealInFinder(absolutePath: string): Promise<{ ok: boolean }> {
    Utils.showItemInFolder(absolutePath);
    return { ok: true };
  }

  async openPath(absolutePath: string): Promise<{ ok: boolean }> {
    return { ok: Utils.openPath(absolutePath) };
  }

  async searchVaultPaths(
    rootPath: string,
    query: string,
    limit: number,
    includeHidden: boolean,
  ): Promise<{ results: PathSearchResult[]; truncated: boolean }> {
    return this.indexService.search(rootPath, query, limit, includeHidden);
  }

  async searchVaultText(
    rootPath: string,
    query: string,
    limit: number,
    includeHidden: boolean,
  ): Promise<{ results: TextSearchResult[]; truncated: boolean }> {
    return this.searchService.search(rootPath, query, limit, includeHidden);
  }

  async setHiddenFilesVisible(
    rootPath: string,
    value: boolean,
    window: DesktopWindow,
  ): Promise<{ metadata: AppMetadata; vault: VaultRecord | null }> {
    const metadata = await this.appState.setHiddenFilesVisible(rootPath, value);
    const vault = this.activeVaultRoot === rootPath ? await this.loadVaultSnapshot(rootPath, metadata) : null;
    if (vault && window === this.activeWindow) {
      this.activeWindow = window;
    }
    return { metadata, vault };
  }

  async toggleFavoriteVault(rootPath: string): Promise<AppMetadata> {
    return this.appState.toggleFavoriteVault(rootPath);
  }

  async removeRecentVault(rootPath: string): Promise<AppMetadata> {
    return this.appState.removeRecentVault(rootPath);
  }

  async persistLastFile(rootPath: string, filePath: string | null): Promise<AppMetadata> {
    return this.appState.persistLastFile(rootPath, filePath);
  }

  async getCompileBundle(
    rootPath: string,
    currentFilePath: string | null,
    liveSource: string,
  ): Promise<CompileBundle> {
    const metadata = await this.appState.load();
    const recent = metadata.recentVaults.find((vault) => vault.rootPath === rootPath);
    const includeHidden = recent?.hiddenFilesVisible ?? false;
    const index = await this.indexService.getIndex(rootPath, includeHidden);

    const fileEntries = index.entries.filter((entry) => entry.kind === "file");
    const mainPath = currentFilePath
      ?? defaultMainFile(
        fileEntries.map((entry) => ({
          ...entry,
          loaded: false,
          content: "",
        })),
        recent,
      );
    const normalizedMainPath = toWorkspacePath(mainPath);

    const textFiles = await Promise.all(
      fileEntries
        .filter((entry) => !entry.isBinary)
        .map((entry) => this.readFileEntry(rootPath, entry.path, false)),
    );
    const binaryFiles = await Promise.all(
      fileEntries
        .filter((entry) => entry.isBinary && shouldLoadBinary(entry.path, entry.sizeBytes))
        .map((entry) => this.readFileEntry(rootPath, entry.path, false)),
    );

    const resolvedTextFiles = textFiles.filter(
      (file): file is VaultFileEntry => file !== null,
    );
    const extraFiles = resolvedTextFiles
      .map((file) => ({
        path: toWorkspacePath(file.path),
        content: file.path === mainPath ? liveSource : file.content,
      }))
      .filter((file) => file.path !== normalizedMainPath);

    const mainEntry = resolvedTextFiles.find((file) => file.path === mainPath);
    const mainSource = currentFilePath === mainPath ? liveSource : mainEntry?.content ?? liveSource;
    const resolvedBinaryFiles = binaryFiles.filter(
      (file): file is VaultFileEntry & { binaryData: Uint8Array } => Boolean(file?.binaryData),
    );
    const extraBinaryFiles = resolvedBinaryFiles
      .filter((file) => file.path !== mainPath)
      .map((file) => ({
        path: toWorkspacePath(file.path),
        data: file.binaryData,
      }));

    return {
      mainPath: normalizedMainPath,
      mainSource,
      extraFiles,
      extraBinaryFiles,
    };
  }

  async getVaultStats(
    rootPath: string,
    includeHidden: boolean,
  ): Promise<{ fileCount: number }> {
    const index = await this.indexService.getIndex(rootPath, includeHidden);
    return { fileCount: countVisibleFiles(index.entries) };
  }

  async persistWindowState(frame: { width: number; height: number; x?: number; y?: number }): Promise<void> {
    await this.appState.setWindowState({
      width: frame.width,
      height: frame.height,
      x: frame.x,
      y: frame.y,
    });
  }

  async getStoredWindowState() {
    const metadata = await this.appState.load();
    return metadata.windowState;
  }

  private createBlankScaffold(name: string): ProjectScaffold {
    return {
      files: [
        {
          path: "main.typ",
          content: `// ${name}\n\n= ${name}\n\n${SAMPLE_DOCUMENT}`,
          isBinary: false,
        },
      ],
      mainFile: "main.typ",
    };
  }

  async createVaultFromTemplate(
    params: { name: string; templateId?: string | null },
    window: DesktopWindow,
  ): Promise<VaultRecord | null> {
    const template = params.templateId ? getBuiltInTemplate(params.templateId) : undefined;
    const scaffold = template ? createBuiltInTemplateScaffold(template.id) : undefined;
    return this.createVault({ name: params.name, scaffold }, window);
  }

  private async openVault(rootPath: string, window: DesktopWindow): Promise<VaultRecord | null> {
    try {
      const metadata = await this.appState.load();
      const snapshot = await this.loadVaultSnapshot(rootPath, metadata);
      await this.stopWatcher();

      this.activeVaultRoot = rootPath;
      this.activeWindow = window;

      const nextMetadata = await this.appState.upsertRecentVault({
        rootPath,
        name: snapshot.name,
        fileCount: countVisibleFiles(snapshot.files),
        lastFilePath: snapshot.mainFile,
      });

      await this.startWatcher(rootPath, window);
      window.webview.rpc?.send.metadataUpdated(nextMetadata);
      return snapshot;
    } catch (error) {
      console.error("Failed to open vault", error);
      await this.appState.removeRecentVault(rootPath);
      return null;
    }
  }

  private async loadVaultSnapshot(rootPath: string, metadata: AppMetadata): Promise<VaultRecord> {
    const recent = metadata.recentVaults.find((vault) => vault.rootPath === rootPath);
    const includeHidden = recent?.hiddenFilesVisible ?? false;
    const index = await this.indexService.getIndex(rootPath, includeHidden);
    const baseEntries: VaultFileEntry[] = index.entries.map((entry) => ({
      ...entry,
      loaded: false,
      content: "",
    }));

    const mainFile = defaultMainFile(baseEntries, recent);
    const hydratedFiles = await Promise.all(
      baseEntries.map(async (entry) => {
        if (entry.kind !== "file") return entry;
        if (entry.path !== mainFile) return entry;
        return (await this.readFileEntry(rootPath, entry.path, true)) ?? entry;
      }),
    );

    const now = Date.now();
    return {
      id: rootPath,
      rootPath,
      name: path.basename(rootPath),
      files: hydratedFiles,
      mainFile,
      createdAt: now,
      updatedAt: now,
    };
  }

  private async hydrateRecentVaultMetadata(metadata: AppMetadata): Promise<AppMetadata> {
    const nextRecentVaults = await Promise.all(
      metadata.recentVaults.map(async (vault) => {
        try {
          const { fileCount } = await this.getVaultStats(vault.rootPath, vault.hiddenFilesVisible);
          if (vault.fileCount === fileCount) return vault;
          return { ...vault, fileCount };
        } catch {
          if (vault.fileCount === 0) return vault;
          return { ...vault, fileCount: 0 };
        }
      }),
    );

    const changed = nextRecentVaults.some((vault, index) => vault !== metadata.recentVaults[index]);
    if (!changed) return metadata;
    return this.appState.save({ ...metadata, recentVaults: nextRecentVaults });
  }

  private async readFileEntry(
    rootPath: string,
    filePath: string,
    hydrateContent: boolean,
  ): Promise<VaultFileEntry | null> {
    const absolutePath = path.join(rootPath, filePath);

    try {
      const stat = await fs.stat(absolutePath);
      const isBinary = !isKnownTextPath(filePath);
      const cacheForVault = this.contentCache.get(rootPath) ?? new Map<string, CachedFile>();
      this.contentCache.set(rootPath, cacheForVault);

      const cached = cacheForVault.get(filePath);
      if (cached && cached.mtimeMs === stat.mtimeMs) {
        if (!hydrateContent && cached.entry.isBinary) {
          return { ...cached.entry, binaryData: undefined, loaded: false };
        }
        return cached.entry;
      }

      const baseEntry: VaultFileEntry = {
        path: filePath,
        name: basenameOf(filePath),
        kind: "file",
        parentPath: parentPathOf(filePath),
        extension: normalizeExtension(filePath),
        isHidden: isHiddenPath(filePath),
        isBinary,
        lastModified: stat.mtimeMs,
        sizeBytes: stat.size,
        loaded: false,
        content: "",
      };

      if (!isBinary && (hydrateContent || stat.size <= MAX_EAGER_TEXT_BYTES)) {
        baseEntry.content = await fs.readFile(absolutePath, "utf8");
        baseEntry.loaded = true;
      } else if (isBinary && hydrateContent && shouldLoadBinary(filePath, stat.size)) {
        const buffer = await fs.readFile(absolutePath);
        baseEntry.binaryData = new Uint8Array(buffer);
        baseEntry.loaded = true;
      }

      cacheForVault.set(filePath, { entry: baseEntry, mtimeMs: stat.mtimeMs });
      return baseEntry;
    } catch {
      return null;
    }
  }

  private async flushWrite(key: string): Promise<void> {
    const pending = this.pendingWrites.get(key);
    if (!pending) return;

    this.pendingWrites.delete(key);
    clearTimeout(pending.timer);

    await this.writeQueue.enqueue(async () => {
      const absolutePath = path.join(pending.rootPath, pending.filePath);
      await fs.mkdir(path.dirname(absolutePath), { recursive: true });
      await fs.writeFile(absolutePath, pending.content, "utf8");

      const stat = await fs.stat(absolutePath);
      const cacheForVault = this.contentCache.get(pending.rootPath) ?? new Map<string, CachedFile>();
      this.contentCache.set(pending.rootPath, cacheForVault);
      cacheForVault.set(pending.filePath, {
        mtimeMs: stat.mtimeMs,
        entry: {
          path: pending.filePath,
          name: basenameOf(pending.filePath),
          kind: "file",
          parentPath: parentPathOf(pending.filePath),
          extension: normalizeExtension(pending.filePath),
          isHidden: isHiddenPath(pending.filePath),
          isBinary: false,
          lastModified: stat.mtimeMs,
          sizeBytes: stat.size,
          loaded: true,
          content: pending.content,
        },
      });

      this.suppressedWatchPaths.set(
        `${pending.rootPath}::${pending.filePath}`,
        Date.now() + SUPPRESSED_WATCH_EVENT_MS,
      );
      this.indexService.invalidate(pending.rootPath);
    });
  }

  private async stopWatcher(): Promise<void> {
    if (this.watcherFlushTimer) {
      clearTimeout(this.watcherFlushTimer);
      this.watcherFlushTimer = null;
    }

    const watcher = this.watcher;
    this.watcher = null;
    if (watcher) {
      await watcher.close();
    }
  }

  private async startWatcher(rootPath: string, window: DesktopWindow): Promise<void> {
    const watcher = chokidar.watch(rootPath, {
      ignoreInitial: true,
      awaitWriteFinish: {
        stabilityThreshold: 150,
        pollInterval: 50,
      },
      ignored: (absolutePath) => absolutePath.split(path.sep).some((segment) => segment && segment !== "." && IGNORED_WATCH_SEGMENTS.has(segment)),
    });

    watcher.on("all", async (kind, absolutePath) => {
      const relativePath = normalizeRelativePath(rootPath, absolutePath);
      if (!relativePath) return;

      const suppressKey = `${rootPath}::${relativePath}`;
      const suppressedUntil = this.suppressedWatchPaths.get(suppressKey);
      if (suppressedUntil && suppressedUntil > Date.now()) {
        return;
      }
      if (suppressedUntil && suppressedUntil <= Date.now()) {
        this.suppressedWatchPaths.delete(suppressKey);
      }

      this.indexService.invalidate(rootPath);
      this.contentCache.get(rootPath)?.delete(relativePath);

      const isDirectory = kind === "addDir" || kind === "unlinkDir";
      let sizeBytes = 0;
      let lastModified = Date.now();
      let isBinary: boolean | undefined;

      if (kind !== "unlink" && kind !== "unlinkDir") {
        try {
          const stat = await fs.stat(absolutePath);
          sizeBytes = stat.size;
          lastModified = stat.mtimeMs;
          isBinary = !isKnownTextPath(relativePath);
        } catch {
          // File may no longer exist.
        }
      }

      this.queueWatcherEvent(window, {
        kind: kind as ExternalVaultEvent["kind"],
        path: relativePath,
        isDirectory,
        sizeBytes,
        lastModified,
        isBinary,
      });
    });

    this.watcher = watcher;
  }

  private queueWatcherEvent(window: DesktopWindow, event: ExternalVaultEvent): void {
    this.watcherBatch.push(event);
    if (this.watcherFlushTimer) return;

    this.watcherFlushTimer = setTimeout(() => {
      this.watcherFlushTimer = null;
      if (this.watcherBatch.length === 0 || !this.activeVaultRoot) return;

      const events = [...this.watcherBatch];
      this.watcherBatch = [];
      window.webview.rpc?.send.externalVaultEvents({
        rootPath: this.activeVaultRoot,
        events,
      });
    }, 64);
  }
}

const IGNORED_WATCH_SEGMENTS = new Set([
  ".git",
  "node_modules",
  ".next",
  ".turbo",
  "dist",
  "build",
  "out",
  ".cache",
]);
