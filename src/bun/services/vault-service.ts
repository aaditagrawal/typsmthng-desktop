import fs from "node:fs/promises";
import { writeFileSync } from "node:fs";
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
  type ProjectTemplateMeta,
  type RecentVaultRecord,
  type TextSearchResult,
  type VaultExportBundle,
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
import { resolveCompileMainFile, resolveVaultMainFile } from "./vault-main-file";
import { shouldReopenVault } from "./vault-reopen";

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

const COMPILE_BINARY_EXTENSION_SET = new Set([
  ...IMAGE_EXTENSION_SET,
  ".pdf",
  ".ttf",
  ".otf",
  ".ttc",
  ".woff",
  ".woff2",
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
  return toPosixPath(input).replace(/^[/\\]+/, "");
}

/** Reject empty / traversal segments so scaffold writes and export reads stay inside the vault. */
function assertSafeVaultRelativePath(input: string): string {
  const sanitized = sanitizeRelativePath(input).replace(/\\/g, "/");
  if (!sanitized || sanitized.includes("\0")) {
    throw new Error("Invalid empty vault path.");
  }
  const normalized = path.posix.normalize(sanitized);
  if (
    !normalized
    || normalized === ".."
    || normalized.startsWith("../")
    || path.posix.isAbsolute(normalized)
    || normalized.startsWith("//")
    || /^[a-zA-Z]:/.test(normalized)
  ) {
    throw new Error(`Path escapes vault root: ${input}`);
  }
  const segments = normalized.split("/");
  if (segments.some((segment) => segment === ".." || segment.includes("\0"))) {
    throw new Error(`Path escapes vault root: ${input}`);
  }
  return normalized;
}

function isEnoent(error: unknown): boolean {
  return Boolean(error && typeof error === "object" && "code" in error && error.code === "ENOENT");
}

function isAlreadyExists(error: unknown): boolean {
  return Boolean(error && typeof error === "object" && "code" in error && error.code === "EEXIST");
}

/** Single-segment project folder name; strips parents and rejects empty / `.` / `..`. */
function sanitizeCreateVaultFolderName(input: string): string | null {
  const name = path.basename(input.trim()).replace(/[/\\:*?"<>|]/g, "_").trim();
  if (!name || name === "." || name === "..") return null;
  return name;
}

function toWorkspacePath(input: string): string {
  const sanitized = sanitizeRelativePath(input);
  return sanitized ? `/${sanitized}` : "/";
}

function isHiddenPath(relativePath: string): boolean {
  return relativePath.split("/").some((segment) => segment.startsWith("."));
}

function shouldLoadBinary(relativePath: string, sizeBytes: number): boolean {
  return COMPILE_BINARY_EXTENSION_SET.has(normalizeExtension(relativePath)) && sizeBytes <= MAX_EAGER_BINARY_BYTES;
}

function defaultMainFile(
  files: VaultFileEntry[],
  recent?: RecentVaultRecord,
  preferredMainFile?: string | null,
): string {
  return resolveVaultMainFile(files, { recent, preferredMainFile });
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
  /** Content of the last successful flush per `${rootPath}::${path}`, to drop own-write watcher echoes. */
  private readonly lastFlushedContent = new Map<string, string>();
  /** Serializes openVault calls: overlapping opens leaked chokidar watchers. */
  private openVaultChain: Promise<unknown> = Promise.resolve();

  private watcher: FSWatcher | null = null;
  private activeVaultRoot: string | null = null;
  private activeWindow: DesktopWindow | null = null;
  private watcherBatch: ExternalVaultEvent[] = [];
  private watcherFlushTimer: ReturnType<typeof setTimeout> | null = null;
  /** Bumped in stopWatcher so in-flight handlers from a prior vault are ignored. */
  private watcherGeneration = 0;
  /** Prefer this vault on the first bootstrap (CLI / OS file open). */
  private startupVaultOverride: { rootPath: string; selectFile: string | null } | null = null;

  async waitUntilReady(): Promise<{ ready: true }> {
    await this.appState.load();
    return { ready: true as const };
  }

  /**
   * RPC handlers take `rootPath` verbatim from the renderer. Relative paths are
   * strictly validated elsewhere; this closes the remaining hole by refusing
   * roots the app has never registered (defense-in-depth against a compromised
   * renderer turning vault RPCs into arbitrary filesystem access).
   */
  private async assertKnownVaultRoot(rootPath: string): Promise<void> {
    if (rootPath === this.activeVaultRoot) return;
    const metadata = await this.appState.load();
    if (metadata.recentVaults.some((vault) => vault.rootPath === rootPath)) return;
    throw new Error(`Unknown vault root: "${rootPath}"`);
  }

  /** True when the absolute path lives inside a registered vault root. */
  private async isPathInKnownVault(absolutePath: string): Promise<boolean> {
    const roots = new Set<string>();
    if (this.activeVaultRoot) roots.add(this.activeVaultRoot);
    const metadata = await this.appState.load();
    for (const vault of metadata.recentVaults) roots.add(vault.rootPath);
    const resolved = path.resolve(absolutePath);
    for (const root of roots) {
      const resolvedRoot = path.resolve(root);
      if (resolved === resolvedRoot || resolved.startsWith(resolvedRoot + path.sep)) return true;
    }
    return false;
  }

  /** Drop per-vault caches when a vault leaves scope (close or switch). */
  private evictVaultCaches(rootPath: string): void {
    this.contentCache.delete(rootPath);
    const prefix = `${rootPath}::`;
    for (const key of this.lastFlushedContent.keys()) {
      if (key.startsWith(prefix)) this.lastFlushedContent.delete(key);
    }
    for (const key of this.suppressedWatchPaths.keys()) {
      if (key.startsWith(prefix)) this.suppressedWatchPaths.delete(key);
    }
  }

  setStartupVaultOverride(rootPath: string | null, selectFile: string | null = null): void {
    this.startupVaultOverride = rootPath
      ? { rootPath, selectFile }
      : null;
  }

  async getBootstrapState(
    window: DesktopWindow,
    options?: { restoreActive?: boolean },
  ): Promise<BootstrapState> {
    let metadata = await this.hydrateRecentVaultMetadata(await this.appState.load());
    const restoreActive = options?.restoreActive !== false;

    // Metadata-only refresh (e.g. after home import) must not open/select a vault.
    // Keep startupVaultOverride intact so a pending CLI/OS open is not dropped if this
    // refresh races ahead of the first restoring bootstrap.
    if (!restoreActive) {
      return { metadata, activeVault: null };
    }

    // If a vault is already open (CLI race, prior bootstrap), return it instead of
    // forcing reopenLastVaultPath again and yanking the user off home/CLI target.
    if (this.activeVaultRoot) {
      const pendingOverride = this.startupVaultOverride;
      // Drop leftover override so later loadProjects calls do not reopen it.
      this.startupVaultOverride = null;
      try {
        if (
          pendingOverride?.selectFile
          && pendingOverride.rootPath === this.activeVaultRoot
        ) {
          try {
            await this.appState.persistLastFile(
              pendingOverride.rootPath,
              pendingOverride.selectFile,
            );
            metadata = await this.appState.load();
          } catch {}
        }
        const preferredMainFile =
          pendingOverride?.rootPath === this.activeVaultRoot
            ? pendingOverride.selectFile
            : null;
        const activeVault = await this.loadVaultSnapshot(
          this.activeVaultRoot,
          metadata,
          preferredMainFile,
        );
        return { metadata, activeVault };
      } catch (error) {
        console.error("Failed to snapshot active vault during bootstrap", error);
        // Snapshot failed while backend still thinks a vault is open — tear it down so
        // renderer/backend stay aligned instead of falling through into reopen logic.
        await this.closeVault();
        metadata = await this.appState.load();
        return { metadata, activeVault: null };
      }
    }

    const override = this.startupVaultOverride;
    this.startupVaultOverride = null;
    if (override) {
      const activeVault = await this.openVault(override.rootPath, window, override.selectFile, {
        removeRecentOnFailure: false,
      });
      if (!activeVault) {
        // openVault may have partially activated before failing — ensure teardown.
        if (this.activeVaultRoot === override.rootPath) {
          await this.closeVault({ rootPath: override.rootPath });
        }
        // upsertRecentVault sets reopen; clear it so the next launch does not
        // auto-open a vault that just failed to activate (openVault catch may
        // already have cleared activeVaultRoot, skipping closeVault above).
        metadata = await this.appState.update((current) => ({
          ...current,
          reopenLastVaultPath:
            current.reopenLastVaultPath === override.rootPath
              ? null
              : current.reopenLastVaultPath,
        }));
        return { metadata, activeVault: null };
      }
      if (override.selectFile) {
        try {
          await this.appState.persistLastFile(override.rootPath, override.selectFile);
        } catch {}
      }
      metadata = await this.appState.load();
      return { metadata, activeVault };
    }

    const reopenPath = metadata.reopenLastVaultPath;
    if (!reopenPath) {
      return { metadata, activeVault: null };
    }

    const pathExists = await this.directoryExists(reopenPath);
    if (!shouldReopenVault(metadata, pathExists)) {
      // Stale reopen path (directory gone) — clear so next launch stays on home.
      metadata = await this.appState.update((current) => ({
        ...current,
        reopenLastVaultPath: null,
      }));
      return { metadata, activeVault: null };
    }

    const activeVault = await this.openVault(reopenPath, window, null, {
      removeRecentOnFailure: false,
    });
    if (!activeVault) {
      // Soft-fail restore: keep the project in recents, just skip auto-open.
      if (this.activeVaultRoot === reopenPath) {
        await this.closeVault({ rootPath: reopenPath });
      }
      metadata = await this.appState.update((current) => ({
        ...current,
        reopenLastVaultPath: null,
      }));
      return { metadata, activeVault: null };
    }

    metadata = await this.appState.load();
    return { metadata, activeVault };
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

  async openRecentVault(
    rootPath: string,
    window: DesktopWindow,
    preferredMainFile?: string | null,
  ): Promise<VaultRecord | null> {
    return this.openVault(rootPath, window, preferredMainFile);
  }

  async createVault(
    params: {
      name: string;
      scaffold?: ProjectScaffold;
      ifExists?: "open" | "fail";
      activate?: boolean;
      parentPath?: string;
    },
    window: DesktopWindow,
  ): Promise<VaultRecord | null> {
    const name = sanitizeCreateVaultFolderName(params.name);
    if (!name) return null;

    let selectedParent = params.parentPath?.trim() || "";
    if (!selectedParent) {
      const [picked] = await Utils.openFileDialog({
        startingFolder: Utils.paths.documents,
        allowedFileTypes: "*",
        canChooseFiles: false,
        canChooseDirectory: true,
        allowsMultipleSelection: false,
      });
      if (!picked) return null;
      selectedParent = picked;
    }

    const rootPath = path.join(selectedParent, name);
    const ifExists = params.ifExists ?? "open";
    const activate = params.activate !== false;

    // Check if directory exists and already has content — open as existing vault
    // instead of overwriting to prevent data loss (GitHub issue #8).
    // Imports pass ifExists: "fail" so a collision is not reported as a successful import.
    // Only roll back by deleting the directory when WE created it — a
    // pre-existing (empty) directory belongs to the user.
    let rollbackOnFailure = false;
    try {
      const entries = await fs.readdir(rootPath);
      if (entries.length > 0) {
        if (ifExists === "fail") return null;
        if (!activate) {
          return this.registerVaultWithoutActivating(rootPath, null);
        }
        return this.openVault(rootPath, window);
      }
    } catch (error) {
      if (!isEnoent(error)) throw error;
      rollbackOnFailure = true;
    }

    try {
      await fs.mkdir(rootPath, { recursive: true });

      const scaffold = params.scaffold ?? this.createBlankScaffold(name);
      for (const file of scaffold.files) {
        const relativePath = assertSafeVaultRelativePath(file.path);
        const absolutePath = path.join(rootPath, relativePath);
        await fs.mkdir(path.dirname(absolutePath), { recursive: true });
        if (file.isBinary && file.binaryData) {
          await fs.writeFile(absolutePath, file.binaryData);
        } else if (file.isBinary) {
          throw new Error(`Scaffold binary "${file.path}" is missing file data.`);
        } else {
          await fs.writeFile(absolutePath, file.content, "utf8");
        }
      }

      if (!activate) {
        const record = await this.registerVaultWithoutActivating(rootPath, scaffold.mainFile);
        if (!record) {
          throw new Error("Failed to register the new project.");
        }
        return record;
      }
      const opened = await this.openVault(rootPath, window, scaffold.mainFile);
      if (!opened) {
        throw new Error("Failed to open the new project.");
      }
      return opened;
    } catch (error) {
      if (rollbackOnFailure) {
        try {
          await fs.rm(rootPath, { recursive: true, force: true });
        } catch (cleanupError) {
          console.error("Failed to roll back createVault directory", cleanupError);
        }
      }
      throw error;
    }
  }

  /** Upsert recents + return a snapshot without watcher/activeVaultOpened (import/bulk). */
  private async registerVaultWithoutActivating(
    rootPath: string,
    preferredMainFile?: string | null,
  ): Promise<VaultRecord | null> {
    try {
      const metadata = await this.appState.load();
      const snapshot = await this.loadVaultSnapshot(rootPath, metadata, preferredMainFile);
      const nextMetadata = await this.appState.upsertRecentVault({
        rootPath,
        name: snapshot.name,
        fileCount: countVisibleFiles(snapshot.files),
        lastFilePath: snapshot.mainFile,
        setAsReopen: false,
      });
      this.activeWindow?.webview.rpc?.send.metadataUpdated(nextMetadata);
      return snapshot;
    } catch (error) {
      console.error("Failed to register vault without activating", error);
      return null;
    }
  }

  async closeVault(input: { rootPath?: string } = {}): Promise<{ ok: true }> {
    // Bound close for a vault that is no longer active: flush that vault's pending
    // writes and clear reopen only if it still points there. Do not tear down the
    // current active vault or wipe a CLI startup override.
    if (input.rootPath && this.activeVaultRoot !== input.rootPath) {
      try {
        await this.flushWrites({ rootPath: input.rootPath });
      } catch (error) {
        console.error("Failed to flush pending writes during stale closeVault", error);
      }
      await this.appState.update((current) => ({
        ...current,
        reopenLastVaultPath:
          current.reopenLastVaultPath === input.rootPath
            ? null
            : current.reopenLastVaultPath,
      }));
      this.evictVaultCaches(input.rootPath);
      return { ok: true };
    }

    // Unbound close with nothing open: do not clear startupVaultOverride / reopen.
    if (!this.activeVaultRoot) {
      return { ok: true };
    }

    // Clear active root + reopen path synchronously so a concurrent getBootstrapState
    // during goHome cannot restore the vault the user is leaving. Flush after the clear
    // so a slow write cannot delay activeVaultClosed / leave a stale activeVaultRoot.
    const rootPath = this.activeVaultRoot;
    const window = this.activeWindow;
    this.activeVaultRoot = null;
    this.activeWindow = null;
    this.startupVaultOverride = null;
    if (rootPath) {
      this.appState.clearReopenLastVaultPathLocally();
    }
    window?.webview.rpc?.send.activeVaultClosed();
    await this.stopWatcher();

    if (rootPath) {
      try {
        await this.flushWrites({ rootPath });
      } catch (error) {
        console.error("Failed to flush pending writes during closeVault", error);
      }
      this.evictVaultCaches(rootPath);
    }

    // Only clear reopen when it still points at the vault we closed — a concurrent
    // open during flush may have already set reopen to a different path.
    await this.appState.update((current) => ({
      ...current,
      reopenLastVaultPath:
        rootPath && current.reopenLastVaultPath === rootPath
          ? null
          : current.reopenLastVaultPath,
    }));
    return { ok: true };
  }

  async readFile(rootPath: string, filePath: string): Promise<VaultFileEntry | null> {
    await this.assertKnownVaultRoot(rootPath);
    return this.readFileEntry(rootPath, assertSafeVaultRelativePath(filePath), true);
  }

  async stageFileWrite(rootPath: string, filePath: string, content: string): Promise<{ queuedAt: number }> {
    await this.assertKnownVaultRoot(rootPath);
    const safePath = assertSafeVaultRelativePath(filePath);
    const key = `${rootPath}::${safePath}`;
    const queuedAt = Date.now();
    const existing = this.pendingWrites.get(key);
    if (existing) {
      clearTimeout(existing.timer);
    }

    const timer = setTimeout(() => {
      void this.flushWrite(key).catch((error) => {
        console.error(`Debounced flush failed for ${key}:`, error);
      });
    }, WRITE_DEBOUNCE_MS);

    this.pendingWrites.set(key, {
      rootPath,
      filePath: safePath,
      content,
      queuedAt,
      timer,
    });

    return { queuedAt };
  }

  async flushWrites(input: { rootPath?: string; path?: string }): Promise<{ ok: true }> {
    // Re-scan after each pass so content staged during an in-flight flush is not skipped.
    for (let pass = 0; pass < 8; pass += 1) {
      const pendingKeys = [...this.pendingWrites.keys()].filter((key) => {
        const pending = this.pendingWrites.get(key);
        if (!pending) return false;
        if (input.rootPath && pending.rootPath !== input.rootPath) return false;
        if (input.path && pending.filePath !== input.path) return false;
        return true;
      });
      if (pendingKeys.length === 0) return { ok: true };
      await Promise.all(pendingKeys.map((key) => this.flushWrite(key)));
      await this.writeQueue.drain();
    }

    const remaining = [...this.pendingWrites.values()].some((pending) => {
      if (input.rootPath && pending.rootPath !== input.rootPath) return false;
      if (input.path && pending.filePath !== input.path) return false;
      return true;
    });
    if (remaining) {
      throw new Error("Timed out flushing pending vault writes");
    }
    return { ok: true };
  }

  /** Best-effort disk write for pending buffers when the process is about to die. */
  flushWritesSync(): void {
    for (const pending of this.pendingWrites.values()) {
      try {
        if (pending.timer) clearTimeout(pending.timer);
        const safePath = assertSafeVaultRelativePath(pending.filePath);
        writeFileSync(path.join(pending.rootPath, safePath), pending.content, "utf8");
      } catch (error) {
        console.error("Failed to flush write on exit", error);
      }
    }
    // Keep the entries: an async write already in flight on the serial queue
    // would otherwise land AFTER these sync writes and roll files back to
    // stale content. Callers pair this with an async flushWrites(), whose
    // queued re-write of the latest content is ordered after any in-flight
    // task; the sync writes above only cover an immediate process death.
  }

  async createFile(rootPath: string, filePath: string, content = ""): Promise<VaultFileEntry | null> {
    return this.writeTextFile(rootPath, filePath, content, { exclusive: true });
  }

  async createFilesBatch(
    rootPath: string,
    entries: Array<{ path: string; content: string }>,
  ): Promise<{ ok: true }> {
    for (const entry of entries) {
      await this.writeTextFile(rootPath, entry.path, entry.content, { exclusive: false });
    }
    return { ok: true };
  }

  private async writeTextFile(
    rootPath: string,
    filePath: string,
    content: string,
    options: { exclusive: boolean },
  ): Promise<VaultFileEntry | null> {
    await this.assertKnownVaultRoot(rootPath);
    const safePath = assertSafeVaultRelativePath(filePath);
    const absolutePath = path.join(rootPath, safePath);
    await fs.mkdir(path.dirname(absolutePath), { recursive: true });
    try {
      await fs.writeFile(
        absolutePath,
        content,
        options.exclusive ? { encoding: "utf8", flag: "wx" } : "utf8",
      );
    } catch (error) {
      if (options.exclusive && isAlreadyExists(error)) {
        throw new Error(`A file already exists at "${safePath}".`);
      }
      throw error;
    }
    this.indexService.invalidate(rootPath);
    this.contentCache.get(rootPath)?.delete(safePath);
    return this.readFileEntry(rootPath, safePath, true);
  }

  async addBinaryFilesBatch(
    rootPath: string,
    entries: Array<{ path: string; data: Uint8Array }>,
  ): Promise<{ ok: true }> {
    await this.assertKnownVaultRoot(rootPath);
    for (const entry of entries) {
      const safePath = assertSafeVaultRelativePath(entry.path);
      const absolutePath = path.join(rootPath, safePath);
      await fs.mkdir(path.dirname(absolutePath), { recursive: true });
      await fs.writeFile(absolutePath, entry.data);
      this.contentCache.get(rootPath)?.delete(safePath);
    }
    this.indexService.invalidate(rootPath);
    return { ok: true };
  }

  async createFolder(rootPath: string, folderPath: string): Promise<{ ok: true }> {
    await this.assertKnownVaultRoot(rootPath);
    const safePath = assertSafeVaultRelativePath(folderPath);
    await fs.mkdir(path.join(rootPath, safePath), { recursive: true });
    this.indexService.invalidate(rootPath);
    return { ok: true };
  }

  async duplicateFile(
    rootPath: string,
    sourcePath: string,
    targetPath: string,
  ): Promise<VaultFileEntry | null> {
    await this.assertKnownVaultRoot(rootPath);
    const safeSource = assertSafeVaultRelativePath(sourcePath);
    const safeTarget = assertSafeVaultRelativePath(targetPath);
    await fs.mkdir(path.dirname(path.join(rootPath, safeTarget)), { recursive: true });
    await fs.copyFile(path.join(rootPath, safeSource), path.join(rootPath, safeTarget));
    this.indexService.invalidate(rootPath);
    this.contentCache.get(rootPath)?.delete(safeTarget);
    return this.readFileEntry(rootPath, safeTarget, true);
  }

  async renamePath(rootPath: string, oldPath: string, newPath: string): Promise<{ ok: true }> {
    await this.assertKnownVaultRoot(rootPath);
    const safeOld = assertSafeVaultRelativePath(oldPath);
    const safeNew = assertSafeVaultRelativePath(newPath);
    if (safeOld === safeNew) return { ok: true };

    const dest = path.join(rootPath, safeNew);
    try {
      const destStat = await fs.lstat(dest);
      // On case-insensitive filesystems (macOS/Windows defaults) a case-only
      // rename resolves dest to the source itself — that's not a collision.
      let sameNode = false;
      try {
        const srcStat = await fs.lstat(path.join(rootPath, safeOld));
        sameNode = srcStat.ino === destStat.ino && srcStat.dev === destStat.dev;
      } catch {}
      if (!sameNode) {
        throw new Error(`A file or folder already exists at "${safeNew}".`);
      }
    } catch (error) {
      if (!isEnoent(error)) throw error;
    }

    await fs.mkdir(path.dirname(dest), { recursive: true });
    await fs.rename(path.join(rootPath, safeOld), dest);
    this.indexService.invalidate(rootPath);
    this.migratePendingWrites(rootPath, safeOld, safeNew);
    this.lastFlushedContent.delete(`${rootPath}::${safeOld}`);
    const cache = this.contentCache.get(rootPath);
    if (cache) {
      const cached = cache.get(safeOld);
      if (cached) {
        cache.delete(safeOld);
        cache.set(safeNew, {
          ...cached,
          entry: { ...cached.entry, path: safeNew, name: basenameOf(safeNew), parentPath: parentPathOf(safeNew) },
        });
      }
    }
    return { ok: true };
  }

  async deletePath(rootPath: string, filePath: string): Promise<{ ok: true }> {
    await this.assertKnownVaultRoot(rootPath);
    const safePath = assertSafeVaultRelativePath(filePath);
    if (!Utils.moveToTrash(path.join(rootPath, safePath))) {
      // Surface the failure instead of letting the file tree and disk diverge.
      throw new Error(`Could not move "${safePath}" to the trash.`);
    }
    this.indexService.invalidate(rootPath);
    this.clearPendingWrites(rootPath, safePath);
    this.contentCache.get(rootPath)?.delete(safePath);
    this.lastFlushedContent.delete(`${rootPath}::${safePath}`);
    return { ok: true };
  }

  async revealInFinder(absolutePath: string): Promise<{ ok: boolean }> {
    if (!(await this.isPathInKnownVault(absolutePath))) {
      return { ok: false };
    }
    Utils.showItemInFolder(absolutePath);
    return { ok: true };
  }

  async openPath(absolutePath: string): Promise<{ ok: boolean }> {
    if (!(await this.isPathInKnownVault(absolutePath))) {
      return { ok: false };
    }
    return { ok: Utils.openPath(absolutePath) };
  }

  async searchVaultPaths(
    rootPath: string,
    query: string,
    limit: number,
    includeHidden: boolean,
  ): Promise<{ results: PathSearchResult[]; truncated: boolean }> {
    await this.assertKnownVaultRoot(rootPath);
    return this.indexService.search(rootPath, query, limit, includeHidden);
  }

  async searchVaultText(
    rootPath: string,
    query: string,
    limit: number,
    includeHidden: boolean,
  ): Promise<{ results: TextSearchResult[]; truncated: boolean }> {
    await this.assertKnownVaultRoot(rootPath);
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

  /** Push the current active vault snapshot to the renderer (e.g. after CLI open + persistLastFile). */
  async resendActiveVault(window: DesktopWindow): Promise<VaultRecord | null> {
    const rootPath = this.activeVaultRoot;
    if (!rootPath) return null;
    try {
      const metadata = await this.appState.load();
      const snapshot = await this.loadVaultSnapshot(rootPath, metadata);
      this.activeWindow = window;
      window.webview.rpc?.send.activeVaultOpened(snapshot);
      return snapshot;
    } catch (error) {
      console.error("Failed to resend active vault", error);
      return null;
    }
  }

  async getCompileBundle(
    rootPath: string,
    currentFilePath: string | null,
    liveSource: string,
  ): Promise<CompileBundle> {
    await this.assertKnownVaultRoot(rootPath);
    const metadata = await this.appState.load();
    const recent = metadata.recentVaults.find((vault) => vault.rootPath === rootPath);
    const includeHidden = recent?.hiddenFilesVisible ?? false;
    const index = await this.indexService.getIndex(rootPath, includeHidden);

    const fileEntries = index.entries.filter((entry) => entry.kind === "file");
    const compileMain = resolveCompileMainFile(fileEntries, currentFilePath);
    const normalizedMainPath = toWorkspacePath(compileMain);
    const currentRel = currentFilePath ? sanitizeRelativePath(currentFilePath) : null;

    const textFiles = await Promise.all(
      fileEntries
        .filter((entry) => !entry.isBinary)
        .map((entry) => this.readFileEntry(rootPath, entry.path, true)),
    );
    const binaryFiles = await Promise.all(
      fileEntries
        .filter((entry) => entry.isBinary && shouldLoadBinary(entry.path, entry.sizeBytes))
        .map((entry) => this.readFileEntry(rootPath, entry.path, true)),
    );

    const resolvedTextFiles = textFiles.filter(
      (file): file is VaultFileEntry => file !== null,
    );
    const extraFiles = resolvedTextFiles
      .map((file) => {
        const pending = this.pendingWrites.get(`${rootPath}::${file.path}`);
        const isCurrent = currentRel !== null && file.path === currentRel;
        return {
          path: toWorkspacePath(file.path),
          content: isCurrent ? liveSource : (pending?.content ?? file.content),
        };
      })
      .filter((file) => file.path !== normalizedMainPath);

    const mainEntry = resolvedTextFiles.find((file) => file.path === compileMain);
    const mainPending = this.pendingWrites.get(`${rootPath}::${compileMain}`);
    const mainSource = currentRel === compileMain
      ? liveSource
      : (mainPending?.content ?? mainEntry?.content ?? liveSource);
    const resolvedBinaryFiles = binaryFiles.filter(
      (file): file is VaultFileEntry & { binaryData: Uint8Array } => Boolean(file?.binaryData),
    );
    const extraBinaryFiles = resolvedBinaryFiles
      .filter((file) => file.path !== compileMain)
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
    await this.assertKnownVaultRoot(rootPath);
    const index = await this.indexService.getIndex(rootPath, includeHidden);
    return { fileCount: countVisibleFiles(index.entries) };
  }

  /** Read all vault files from disk for zip export (works for unloaded recent projects). */
  async getVaultExportBundle(rootPath: string): Promise<VaultExportBundle | null> {
    try {
      await this.assertKnownVaultRoot(rootPath);
      // Defense in depth: exporters also flush, but direct RPC callers must not zip stale disk.
      await this.flushWrites({ rootPath });

      const metadata = await this.appState.load();
      const recent = metadata.recentVaults.find((vault) => vault.rootPath === rootPath);
      const includeHidden = recent?.hiddenFilesVisible ?? false;
      const index = await this.indexService.getIndex(rootPath, includeHidden);
      if (index.truncated) {
        throw new Error(
          `Project "${path.basename(rootPath)}" has too many files to export completely.`,
        );
      }
      const fileEntries = index.entries.filter(
        (entry) =>
          entry.kind === "file"
          && entry.path !== ".folder"
          && !entry.path.endsWith("/.folder")
          && (entry.path === ".typsmthng/template.json" || !entry.path.startsWith(".typsmthng/")),
      );

      const files = await Promise.all(
        fileEntries.map(async (entry) => {
          const relativePath = assertSafeVaultRelativePath(entry.path);
          const absolutePath = path.join(rootPath, relativePath);
          try {
            if (entry.isBinary) {
              const buffer = await fs.readFile(absolutePath);
              return {
                path: relativePath,
                isBinary: true,
                binaryData: new Uint8Array(buffer),
              };
            }
            const content = await fs.readFile(absolutePath, "utf8");
            return {
              path: relativePath,
              isBinary: false,
              content,
            };
          } catch (error) {
            throw new Error(
              `Failed to read "${relativePath}" while exporting ${path.basename(rootPath)}: ${
                error instanceof Error ? error.message : "unknown error"
              }`,
            );
          }
        }),
      );

      const exportFiles = [...files];
      const hasTemplateMeta = exportFiles.some((file) => file.path === ".typsmthng/template.json");
      if (!hasTemplateMeta) {
        try {
          const content = await fs.readFile(path.join(rootPath, ".typsmthng", "template.json"), "utf8");
          exportFiles.push({
            path: ".typsmthng/template.json",
            isBinary: false,
            content,
          });
        } catch {
          // Optional template metadata.
        }
      }

      return {
        name: path.basename(rootPath),
        files: exportFiles,
      };
    } catch (error) {
      console.error("Failed to build vault export bundle", error);
      // Preserve the specific per-file error for the renderer instead of a generic null.
      throw error instanceof Error
        ? error
        : new Error("Failed to build vault export bundle.");
    }
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

  private async directoryExists(rootPath: string): Promise<boolean> {
    try {
      const stat = await fs.stat(rootPath);
      return stat.isDirectory();
    } catch {
      return false;
    }
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

  private async openVault(
    rootPath: string,
    window: DesktopWindow,
    preferredMainFile?: string | null,
    options?: { removeRecentOnFailure?: boolean },
  ): Promise<VaultRecord | null> {
    // Serialize opens: overlapping openVault calls interleave at the awaits
    // and can leak a chokidar watcher (startWatcher overwrites this.watcher
    // without closing the previous one).
    const run = this.openVaultChain.then(() =>
      this.openVaultSerialized(rootPath, window, preferredMainFile, options),
    );
    this.openVaultChain = run.catch(() => {});
    return run;
  }

  private async openVaultSerialized(
    rootPath: string,
    window: DesktopWindow,
    preferredMainFile?: string | null,
    options?: { removeRecentOnFailure?: boolean },
  ): Promise<VaultRecord | null> {
    const removeRecentOnFailure = options?.removeRecentOnFailure ?? true;
    try {
      const metadata = await this.appState.load();
      const snapshot = await this.loadVaultSnapshot(rootPath, metadata, preferredMainFile);
      await this.stopWatcher();

      const previousRoot = this.activeVaultRoot;
      if (previousRoot && previousRoot !== rootPath) {
        this.evictVaultCaches(previousRoot);
      }
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
      window.webview.rpc?.send.activeVaultOpened(snapshot);
      return snapshot;
    } catch (error) {
      console.error("Failed to open vault", error);
      // Undo partial activation so bootstrap/CLI callers do not see a zombie active vault.
      if (this.activeVaultRoot === rootPath) {
        this.activeVaultRoot = null;
        this.activeWindow = null;
        try {
          await this.stopWatcher();
        } catch (stopError) {
          console.error("Failed to stop watcher after openVault failure", stopError);
        }
      }
      if (removeRecentOnFailure) {
        await this.appState.removeRecentVault(rootPath);
      }
      return null;
    }
  }

  private async loadVaultSnapshot(
    rootPath: string,
    metadata: AppMetadata,
    preferredMainFile?: string | null,
  ): Promise<VaultRecord> {
    const recent = metadata.recentVaults.find((vault) => vault.rootPath === rootPath);
    const includeHidden = recent?.hiddenFilesVisible ?? false;
    const index = await this.indexService.getIndex(rootPath, includeHidden);
    const baseEntries: VaultFileEntry[] = index.entries.map((entry) => ({
      ...entry,
      loaded: false,
      content: "",
    }));

    const mainFile = defaultMainFile(baseEntries, recent, preferredMainFile);
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
      templateMeta: await this.readTemplateMeta(rootPath),
    };
  }

  private async readTemplateMeta(rootPath: string): Promise<ProjectTemplateMeta | undefined> {
    try {
      const raw = await fs.readFile(path.join(rootPath, ".typsmthng", "template.json"), "utf8");
      const parsed = JSON.parse(raw) as ProjectTemplateMeta;
      if (!parsed || typeof parsed !== "object") return undefined;
      return parsed;
    } catch {
      return undefined;
    }
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
    const safePath = assertSafeVaultRelativePath(filePath);
    const absolutePath = path.join(rootPath, safePath);

    try {
      const stat = await fs.stat(absolutePath);
      const isBinary = !isKnownTextPath(safePath);
      const cacheForVault = this.contentCache.get(rootPath) ?? new Map<string, CachedFile>();
      this.contentCache.set(rootPath, cacheForVault);

      const cached = cacheForVault.get(safePath);
      if (cached && cached.mtimeMs === stat.mtimeMs) {
        const needsTextHydration =
          hydrateContent && !cached.entry.isBinary && !cached.entry.loaded;
        if (!needsTextHydration) {
          if (!hydrateContent && cached.entry.isBinary) {
            return { ...cached.entry, binaryData: undefined, loaded: false };
          }
          return cached.entry;
        }
      }

      const baseEntry: VaultFileEntry = {
        path: safePath,
        name: basenameOf(safePath),
        kind: "file",
        parentPath: parentPathOf(safePath),
        extension: normalizeExtension(safePath),
        isHidden: isHiddenPath(safePath),
        isBinary,
        lastModified: stat.mtimeMs,
        sizeBytes: stat.size,
        loaded: false,
        content: "",
      };

      if (!isBinary && (hydrateContent || stat.size <= MAX_EAGER_TEXT_BYTES)) {
        baseEntry.content = await fs.readFile(absolutePath, "utf8");
        baseEntry.loaded = true;
      } else if (isBinary && hydrateContent && shouldLoadBinary(safePath, stat.size)) {
        const buffer = await fs.readFile(absolutePath);
        baseEntry.binaryData = new Uint8Array(buffer);
        baseEntry.loaded = true;
      }

      cacheForVault.set(safePath, { entry: baseEntry, mtimeMs: stat.mtimeMs });
      return baseEntry;
    } catch {
      return null;
    }
  }

  private clearPendingWrites(rootPath: string, filePath: string): void {
    const childPrefix = `${filePath}/`;
    for (const [key, pending] of this.pendingWrites) {
      if (pending.rootPath !== rootPath) continue;
      if (pending.filePath !== filePath && !pending.filePath.startsWith(childPrefix)) continue;
      clearTimeout(pending.timer);
      this.pendingWrites.delete(key);
    }
  }

  private migratePendingWrites(rootPath: string, oldPath: string, newPath: string): void {
    const childPrefix = `${oldPath}/`;
    const migrations: Array<{ key: string; pending: PendingWrite; nextPath: string }> = [];

    for (const [key, pending] of this.pendingWrites) {
      if (pending.rootPath !== rootPath) continue;
      let nextPath: string | null = null;
      if (pending.filePath === oldPath) {
        nextPath = newPath;
      } else if (pending.filePath.startsWith(childPrefix)) {
        nextPath = `${newPath}/${pending.filePath.slice(childPrefix.length)}`;
      }
      if (!nextPath) continue;
      migrations.push({ key, pending, nextPath });
    }

    for (const { key, pending, nextPath } of migrations) {
      clearTimeout(pending.timer);
      this.pendingWrites.delete(key);

      const nextKey = `${rootPath}::${nextPath}`;
      const existing = this.pendingWrites.get(nextKey);
      if (existing) {
        clearTimeout(existing.timer);
      }

      const timer = setTimeout(() => {
        void this.flushWrite(nextKey).catch((error) => {
          console.error(`Debounced flush failed for ${nextKey}:`, error);
        });
      }, WRITE_DEBOUNCE_MS);

      this.pendingWrites.set(nextKey, {
        rootPath,
        filePath: nextPath,
        content: pending.content,
        queuedAt: pending.queuedAt,
        timer,
      });
    }
  }

  private async flushWrite(key: string): Promise<void> {
    const pending = this.pendingWrites.get(key);
    if (!pending) return;

    // Keep the entry until the write succeeds so a failed flush can retry and a
    // newer stageFileWrite during the queue wait still wins via queuedAt.
    clearTimeout(pending.timer);

    await this.writeQueue.enqueue(async () => {
      const latest = this.pendingWrites.get(key);
      if (!latest) return;

      try {
        const absolutePath = path.join(latest.rootPath, latest.filePath);
        await fs.mkdir(path.dirname(absolutePath), { recursive: true });
        await fs.writeFile(absolutePath, latest.content, "utf8");

        const stat = await fs.stat(absolutePath);
        const cacheForVault = this.contentCache.get(latest.rootPath) ?? new Map<string, CachedFile>();
        this.contentCache.set(latest.rootPath, cacheForVault);
        cacheForVault.set(latest.filePath, {
          mtimeMs: stat.mtimeMs,
          entry: {
            path: latest.filePath,
            name: basenameOf(latest.filePath),
            kind: "file",
            parentPath: parentPathOf(latest.filePath),
            extension: normalizeExtension(latest.filePath),
            isHidden: isHiddenPath(latest.filePath),
            isBinary: false,
            lastModified: stat.mtimeMs,
            sizeBytes: stat.size,
            loaded: true,
            content: latest.content,
          },
        });

        // Sweep expired entries so the map doesn't grow one entry per
        // file ever saved for the whole process lifetime.
        const now = Date.now();
        for (const [suppressedKey, until] of this.suppressedWatchPaths) {
          if (until <= now) this.suppressedWatchPaths.delete(suppressedKey);
        }
        this.suppressedWatchPaths.set(
          `${latest.rootPath}::${latest.filePath}`,
          now + SUPPRESSED_WATCH_EVENT_MS,
        );
        this.lastFlushedContent.set(`${latest.rootPath}::${latest.filePath}`, latest.content);
        this.indexService.invalidate(latest.rootPath);

        const current = this.pendingWrites.get(key);
        if (current && current.queuedAt === latest.queuedAt) {
          clearTimeout(current.timer);
          this.pendingWrites.delete(key);
        }
      } catch (error) {
        console.error(`Failed to flush vault write for ${latest.filePath}:`, error);
        const current = this.pendingWrites.get(key);
        if (current) {
          clearTimeout(current.timer);
          const timer = setTimeout(() => {
            void this.flushWrite(key).catch((retryError) => {
              console.error(`Retry flush failed for ${key}:`, retryError);
            });
          }, WRITE_DEBOUNCE_MS);
          this.pendingWrites.set(key, {
            ...current,
            timer,
          });
        }
        throw error;
      }
    });
  }

  private async stopWatcher(): Promise<void> {
    if (this.watcherFlushTimer) {
      clearTimeout(this.watcherFlushTimer);
      this.watcherFlushTimer = null;
    }
    this.watcherBatch = [];
    this.watcherGeneration += 1;

    const watcher = this.watcher;
    this.watcher = null;
    if (watcher) {
      await watcher.close();
    }
  }

  private async startWatcher(rootPath: string, window: DesktopWindow): Promise<void> {
    const generation = this.watcherGeneration;
    const watcher = chokidar.watch(rootPath, {
      ignoreInitial: true,
      awaitWriteFinish: {
        stabilityThreshold: 150,
        pollInterval: 50,
      },
      ignored: (absolutePath) => absolutePath.split(path.sep).some((segment) => segment && segment !== "." && IGNORED_WATCH_SEGMENTS.has(segment)),
    });

    watcher.on("all", async (kind, absolutePath) => {
      if (generation !== this.watcherGeneration || this.activeVaultRoot !== rootPath) {
        return;
      }

      const relativePath = normalizeRelativePath(rootPath, absolutePath);
      if (!relativePath) return;

      const suppressKey = `${rootPath}::${relativePath}`;
      const suppressedUntil = this.suppressedWatchPaths.get(suppressKey);
      if (suppressedUntil !== undefined && suppressedUntil <= Date.now()) {
        this.suppressedWatchPaths.delete(suppressKey);
      }

      // For file writes, decide by content instead of timing alone: our own
      // flush echoing back late must not raise a conflict (the banner would
      // invite the user to discard their newer buffer), and a genuinely
      // different external write must not be swallowed just because it landed
      // inside the suppression window.
      if (kind === "change" || kind === "add") {
        const lastWritten = this.lastFlushedContent.get(suppressKey);
        if (lastWritten !== undefined) {
          try {
            const currentContent = await fs.readFile(absolutePath, "utf8");
            if (currentContent === lastWritten) return;
          } catch {
            // Unreadable/gone — fall through to normal handling.
          }
          // Content diverged: a real external edit; stop echo-matching until
          // the next app-side flush refreshes the record.
          this.lastFlushedContent.delete(suppressKey);
        } else if (suppressedUntil !== undefined && suppressedUntil > Date.now()) {
          return;
        }
      } else if (suppressedUntil !== undefined && suppressedUntil > Date.now()) {
        return;
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

      if (generation !== this.watcherGeneration || this.activeVaultRoot !== rootPath) {
        return;
      }

      this.queueWatcherEvent(window, rootPath, {
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

  private queueWatcherEvent(
    window: DesktopWindow,
    rootPath: string,
    event: ExternalVaultEvent,
  ): void {
    if (this.activeVaultRoot !== rootPath) return;

    this.watcherBatch.push(event);
    if (this.watcherFlushTimer) return;

    this.watcherFlushTimer = setTimeout(() => {
      this.watcherFlushTimer = null;
      if (this.watcherBatch.length === 0 || this.activeVaultRoot !== rootPath) {
        this.watcherBatch = [];
        return;
      }

      const events = [...this.watcherBatch];
      this.watcherBatch = [];
      window.webview.rpc?.send.externalVaultEvents({
        rootPath,
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
