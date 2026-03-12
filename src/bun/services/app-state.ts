import fs from "node:fs/promises";
import path from "node:path";
import { Utils } from "electrobun/bun";

import type { AppMetadata, RecentVaultRecord, WindowState } from "../../shared/rpc";

const METADATA_FILENAME = "app-state.json";

const DEFAULT_METADATA: AppMetadata = {
  version: 1,
  recentVaults: [],
  reopenLastVaultPath: null,
  windowState: {
    width: 1440,
    height: 920,
  },
};

function metadataPath(): string {
  return path.join(Utils.paths.userData, METADATA_FILENAME);
}

function dedupeRecentDocuments(
  docs: RecentVaultRecord["recentDocuments"],
): RecentVaultRecord["recentDocuments"] {
  const seen = new Set<string>();
  const next: RecentVaultRecord["recentDocuments"] = [];
  const sorted = [...docs].sort((left, right) => right.lastOpenedAt - left.lastOpenedAt);
  for (const item of sorted) {
    if (seen.has(item.path)) continue;
    seen.add(item.path);
    next.push(item);
    if (next.length >= 12) break;
  }
  return next;
}

function normalizeMetadata(metadata: AppMetadata): AppMetadata {
  return {
    version: 1,
    recentVaults: metadata.recentVaults
      .map((vault) => ({
        ...vault,
        recentDocuments: dedupeRecentDocuments(vault.recentDocuments ?? []),
        hiddenFilesVisible: vault.hiddenFilesVisible ?? false,
        favorite: vault.favorite ?? false,
        fileCount: typeof vault.fileCount === "number" ? vault.fileCount : undefined,
        lastFilePath: vault.lastFilePath ?? null,
      }))
      .sort((left, right) => {
        if (left.favorite !== right.favorite) {
          return left.favorite ? -1 : 1;
        }
        return right.lastOpenedAt - left.lastOpenedAt;
      })
      .slice(0, 24),
    reopenLastVaultPath: metadata.reopenLastVaultPath ?? null,
    windowState: metadata.windowState ?? DEFAULT_METADATA.windowState,
  };
}

export class AppStateService {
  private cache: AppMetadata | null = null;

  async load(): Promise<AppMetadata> {
    if (this.cache) return this.cache;

    await fs.mkdir(Utils.paths.userData, { recursive: true });
    try {
      const raw = await fs.readFile(metadataPath(), "utf8");
      const parsed = JSON.parse(raw) as AppMetadata;
      this.cache = normalizeMetadata(parsed);
    } catch {
      this.cache = DEFAULT_METADATA;
      await this.save(this.cache);
    }
    return this.cache;
  }

  async save(metadata: AppMetadata): Promise<AppMetadata> {
    const normalized = normalizeMetadata(metadata);
    await fs.mkdir(Utils.paths.userData, { recursive: true });
    await fs.writeFile(metadataPath(), `${JSON.stringify(normalized, null, 2)}\n`, "utf8");
    this.cache = normalized;
    return normalized;
  }

  async update(mutator: (current: AppMetadata) => AppMetadata): Promise<AppMetadata> {
    const current = await this.load();
    return this.save(mutator(current));
  }

  async upsertRecentVault(input: {
    rootPath: string;
    name: string;
    fileCount?: number;
    lastFilePath?: string | null;
  }): Promise<AppMetadata> {
    return this.update((current) => {
      const now = Date.now();
      const existing = current.recentVaults.find((vault) => vault.rootPath === input.rootPath);
      const nextRecord: RecentVaultRecord = {
        id: input.rootPath,
        rootPath: input.rootPath,
        name: input.name,
        favorite: existing?.favorite ?? false,
        hiddenFilesVisible: existing?.hiddenFilesVisible ?? false,
        fileCount: input.fileCount ?? existing?.fileCount,
        lastOpenedAt: now,
        lastFilePath: input.lastFilePath ?? existing?.lastFilePath ?? null,
        recentDocuments: existing?.recentDocuments ?? [],
      };

      return {
        ...current,
        reopenLastVaultPath: input.rootPath,
        recentVaults: [
          nextRecord,
          ...current.recentVaults.filter((vault) => vault.rootPath !== input.rootPath),
        ],
      };
    });
  }

  async removeRecentVault(rootPath: string): Promise<AppMetadata> {
    return this.update((current) => ({
      ...current,
      reopenLastVaultPath:
        current.reopenLastVaultPath === rootPath ? null : current.reopenLastVaultPath,
      recentVaults: current.recentVaults.filter((vault) => vault.rootPath !== rootPath),
    }));
  }

  async toggleFavoriteVault(rootPath: string): Promise<AppMetadata> {
    return this.update((current) => ({
      ...current,
      recentVaults: current.recentVaults.map((vault) =>
        vault.rootPath === rootPath
          ? { ...vault, favorite: !vault.favorite }
          : vault,
      ),
    }));
  }

  async setHiddenFilesVisible(rootPath: string, value: boolean): Promise<AppMetadata> {
    return this.update((current) => ({
      ...current,
      recentVaults: current.recentVaults.map((vault) =>
        vault.rootPath === rootPath
          ? { ...vault, hiddenFilesVisible: value }
          : vault,
      ),
    }));
  }

  async persistLastFile(rootPath: string, relativePath: string | null): Promise<AppMetadata> {
    return this.update((current) => ({
      ...current,
      recentVaults: current.recentVaults.map((vault) => {
        if (vault.rootPath !== rootPath) return vault;
        const recentDocuments = relativePath
          ? dedupeRecentDocuments([
              { path: relativePath, lastOpenedAt: Date.now() },
              ...vault.recentDocuments,
            ])
          : vault.recentDocuments;
        return {
          ...vault,
          lastFilePath: relativePath,
          recentDocuments,
        };
      }),
    }));
  }

  async setWindowState(windowState: WindowState): Promise<AppMetadata> {
    return this.update((current) => ({
      ...current,
      windowState,
    }));
  }
}
