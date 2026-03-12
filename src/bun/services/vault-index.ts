import fs from "node:fs/promises";
import type { Dirent } from "node:fs";
import path from "node:path";

import type { PathSearchResult, VaultPathEntry } from "../../shared/rpc";
import { isKnownTextPath, normalizeExtension } from "../../mainview/lib/file-classification";
import { runProcess } from "./process-runner";

const INDEX_TTL_MS = 15_000;
const INDEX_MAX_KEYS = 6;
const INDEX_MAX_ENTRIES = 40_000;
const SCAN_READDIR_CONCURRENCY = 32;
const GIT_CHECK_IGNORE_MAX_STDIN_BYTES = 256 * 1024;
const IGNORED_DIRECTORY_NAMES = new Set([
  ".git",
  ".svn",
  ".hg",
  "node_modules",
  ".next",
  ".turbo",
  ".cache",
  "dist",
  "build",
  "out",
]);

interface CachedVaultIndex {
  includeHidden: boolean;
  scannedAt: number;
  truncated: boolean;
  entries: VaultPathEntry[];
}

const indexCache = new Map<string, CachedVaultIndex>();
const inFlightBuilds = new Map<string, Promise<CachedVaultIndex>>();

function cacheKey(rootPath: string, includeHidden: boolean): string {
  return `${rootPath}::${includeHidden ? "show" : "hide"}`;
}

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

function isHiddenPath(relativePath: string): boolean {
  return relativePath.split("/").some((segment) => segment.startsWith("."));
}

function isPathInIgnoredDirectory(relativePath: string): boolean {
  const segments = relativePath.split("/");
  return segments.some((segment) => IGNORED_DIRECTORY_NAMES.has(segment));
}

function normalizeQuery(input: string): string {
  return input.trim().replace(/^[@./]+/, "").toLowerCase();
}

function scoreEntry(entry: VaultPathEntry, query: string): number {
  if (!query) {
    return entry.kind === "directory" ? 0 : 1;
  }

  const normalizedPath = entry.path.toLowerCase();
  const normalizedName = entry.name.toLowerCase();
  if (normalizedName === query) return 0;
  if (normalizedPath === query) return 1;
  if (normalizedName.startsWith(query)) return 2;
  if (normalizedPath.startsWith(query)) return 3;
  if (normalizedPath.includes(`/${query}`)) return 4;
  return 5;
}

function splitNullSeparatedPaths(input: string, truncated: boolean): string[] {
  const parts = input.split("\0");
  if (parts.length === 0) return [];
  if (truncated && parts[parts.length - 1]?.length) {
    parts.pop();
  }
  return parts.filter((value) => value.length > 0);
}

function directoryAncestorsOf(relativePath: string): string[] {
  const segments = relativePath.split("/").filter(Boolean);
  if (segments.length <= 1) return [];
  const directories: string[] = [];
  for (let index = 1; index < segments.length; index += 1) {
    directories.push(segments.slice(0, index).join("/"));
  }
  return directories;
}

async function mapWithConcurrency<TInput, TOutput>(
  items: readonly TInput[],
  concurrency: number,
  mapper: (item: TInput, index: number) => Promise<TOutput>,
): Promise<TOutput[]> {
  if (items.length === 0) return [];

  const boundedConcurrency = Math.max(1, Math.min(concurrency, items.length));
  const results = Array.from({ length: items.length }) as TOutput[];
  let nextIndex = 0;

  const workers = Array.from({ length: boundedConcurrency }, async () => {
    while (nextIndex < items.length) {
      const currentIndex = nextIndex;
      nextIndex += 1;
      results[currentIndex] = await mapper(items[currentIndex] as TInput, currentIndex);
    }
  });

  await Promise.all(workers);
  return results;
}

function toEntry(input: {
  relativePath: string;
  kind: "file" | "directory";
  lastModified?: number;
  sizeBytes?: number;
}): VaultPathEntry {
  return {
    path: input.relativePath,
    name: basenameOf(input.relativePath),
    kind: input.kind,
    parentPath: parentPathOf(input.relativePath),
    extension: input.kind === "file" ? normalizeExtension(input.relativePath) : "",
    isHidden: isHiddenPath(input.relativePath),
    isBinary: input.kind === "file" ? !isKnownTextPath(input.relativePath) : false,
    lastModified: input.lastModified ?? 0,
    sizeBytes: input.sizeBytes ?? 0,
  };
}

async function isInsideGitWorkTree(cwd: string): Promise<boolean> {
  const result = await runProcess("git", ["rev-parse", "--is-inside-work-tree"], {
    cwd,
    allowNonZeroExit: true,
    timeoutMs: 5_000,
    maxBufferBytes: 4_096,
  }).catch(() => null);

  return Boolean(result && result.code === 0 && result.stdout.trim() === "true");
}

async function filterGitIgnoredPaths(cwd: string, relativePaths: string[]): Promise<string[]> {
  if (relativePaths.length === 0) return relativePaths;

  const ignoredPaths = new Set<string>();
  let chunk: string[] = [];
  let chunkBytes = 0;

  const flushChunk = async (): Promise<boolean> => {
    if (chunk.length === 0) return true;

    const checkIgnore = await runProcess(
      "git",
      ["check-ignore", "--no-index", "-z", "--stdin"],
      {
        cwd,
        allowNonZeroExit: true,
        timeoutMs: 20_000,
        maxBufferBytes: 16 * 1024 * 1024,
        outputMode: "truncate",
        stdin: `${chunk.join("\0")}\0`,
      },
    ).catch(() => null);

    chunk = [];
    chunkBytes = 0;
    if (!checkIgnore) return false;
    if (checkIgnore.code !== 0 && checkIgnore.code !== 1) return false;

    for (const ignoredPath of splitNullSeparatedPaths(
      checkIgnore.stdout,
      Boolean(checkIgnore.stdoutTruncated),
    )) {
      ignoredPaths.add(ignoredPath);
    }
    return true;
  };

  for (const relativePath of relativePaths) {
    const relativePathBytes = Buffer.byteLength(relativePath) + 1;
    if (
      chunk.length > 0 &&
      chunkBytes + relativePathBytes > GIT_CHECK_IGNORE_MAX_STDIN_BYTES &&
      !(await flushChunk())
    ) {
      return relativePaths;
    }

    chunk.push(relativePath);
    chunkBytes += relativePathBytes;

    if (chunkBytes >= GIT_CHECK_IGNORE_MAX_STDIN_BYTES && !(await flushChunk())) {
      return relativePaths;
    }
  }

  if (!(await flushChunk())) {
    return relativePaths;
  }

  if (ignoredPaths.size === 0) return relativePaths;
  return relativePaths.filter((relativePath) => !ignoredPaths.has(relativePath));
}

async function buildIndexFromGit(
  rootPath: string,
  includeHidden: boolean,
): Promise<CachedVaultIndex | null> {
  if (!(await isInsideGitWorkTree(rootPath))) {
    return null;
  }

  const listedFiles = await runProcess(
    "git",
    ["ls-files", "--cached", "--others", "--exclude-standard", "-z"],
    {
      cwd: rootPath,
      allowNonZeroExit: true,
      timeoutMs: 20_000,
      maxBufferBytes: 16 * 1024 * 1024,
      outputMode: "truncate",
    },
  ).catch(() => null);

  if (!listedFiles || listedFiles.code !== 0) {
    return null;
  }

  const listedPaths = splitNullSeparatedPaths(
    listedFiles.stdout,
    Boolean(listedFiles.stdoutTruncated),
  )
    .map((entry) => toPosixPath(entry))
    .filter((entry) => entry.length > 0)
    .filter((entry) => includeHidden || !isHiddenPath(entry))
    .filter((entry) => !isPathInIgnoredDirectory(entry));

  const filePaths = await filterGitIgnoredPaths(rootPath, listedPaths);
  const directorySet = new Set<string>();
  for (const filePath of filePaths) {
    for (const directoryPath of directoryAncestorsOf(filePath)) {
      if (!isPathInIgnoredDirectory(directoryPath)) {
        directorySet.add(directoryPath);
      }
    }
  }

  const directoryEntries = [...directorySet]
    .sort((left, right) => left.localeCompare(right))
    .map((directoryPath) => toEntry({ relativePath: directoryPath, kind: "directory" }));
  const fileEntries = [...new Set(filePaths)]
    .sort((left, right) => left.localeCompare(right))
    .map((filePath) => toEntry({ relativePath: filePath, kind: "file" }));

  const entries = [...directoryEntries, ...fileEntries];
  return {
    includeHidden,
    scannedAt: Date.now(),
    entries: entries.slice(0, INDEX_MAX_ENTRIES),
    truncated: Boolean(listedFiles.stdoutTruncated) || entries.length > INDEX_MAX_ENTRIES,
  };
}

async function buildIndexFromScan(rootPath: string, includeHidden: boolean): Promise<CachedVaultIndex> {
  const shouldFilterWithGitIgnore = await isInsideGitWorkTree(rootPath);
  let pendingDirectories: string[] = [""];
  const entries: VaultPathEntry[] = [];
  let truncated = false;

  while (pendingDirectories.length > 0 && !truncated) {
    const currentDirectories = pendingDirectories;
    pendingDirectories = [];
    const directoryEntries = await mapWithConcurrency(
      currentDirectories,
      SCAN_READDIR_CONCURRENCY,
      async (relativeDir) => {
        const absoluteDir = relativeDir ? path.join(rootPath, relativeDir) : rootPath;
        try {
          const dirents = await fs.readdir(absoluteDir, { withFileTypes: true });
          return { relativeDir, dirents };
        } catch (error) {
          if (!relativeDir) {
            throw new Error(
              `Unable to scan vault entries at '${rootPath}': ${
                error instanceof Error ? error.message : "unknown error"
              }`,
              { cause: error },
            );
          }
          return { relativeDir, dirents: null as Dirent[] | null };
        }
      },
    );

    const candidateEntriesByDirectory = directoryEntries.map((directoryEntry) => {
      const { relativeDir, dirents } = directoryEntry;
      if (!dirents) return [] as Array<{ dirent: Dirent; relativePath: string }>;

      dirents.sort((left, right) => left.name.localeCompare(right.name));
      const candidates: Array<{ dirent: Dirent; relativePath: string }> = [];
      for (const dirent of dirents) {
        if (!dirent.name || dirent.name === "." || dirent.name === "..") continue;
        if (dirent.isDirectory() && IGNORED_DIRECTORY_NAMES.has(dirent.name)) continue;
        if (!dirent.isDirectory() && !dirent.isFile()) continue;

        const relativePath = toPosixPath(
          relativeDir ? path.join(relativeDir, dirent.name) : dirent.name,
        );

        if (!includeHidden && isHiddenPath(relativePath)) continue;
        if (isPathInIgnoredDirectory(relativePath)) continue;
        candidates.push({ dirent, relativePath });
      }
      return candidates;
    });

    const candidatePaths = candidateEntriesByDirectory.flatMap((items) =>
      items.map((item) => item.relativePath),
    );
    const allowedPathSet = shouldFilterWithGitIgnore
      ? new Set(await filterGitIgnoredPaths(rootPath, candidatePaths))
      : null;

    for (const candidateEntries of candidateEntriesByDirectory) {
      for (const candidate of candidateEntries) {
        if (allowedPathSet && !allowedPathSet.has(candidate.relativePath)) continue;

        let stat: Awaited<ReturnType<typeof fs.stat>> | null = null;
        if (candidate.dirent.isFile()) {
          try {
            stat = await fs.stat(path.join(rootPath, candidate.relativePath));
          } catch {
            stat = null;
          }
        }

        entries.push(
          toEntry({
            relativePath: candidate.relativePath,
            kind: candidate.dirent.isDirectory() ? "directory" : "file",
            lastModified: stat?.mtimeMs ?? 0,
            sizeBytes: stat?.size ?? 0,
          }),
        );

        if (candidate.dirent.isDirectory()) {
          pendingDirectories.push(candidate.relativePath);
        }

        if (entries.length >= INDEX_MAX_ENTRIES) {
          truncated = true;
          break;
        }
      }

      if (truncated) break;
    }
  }

  return {
    includeHidden,
    scannedAt: Date.now(),
    truncated,
    entries,
  };
}

async function buildVaultIndex(rootPath: string, includeHidden: boolean): Promise<CachedVaultIndex> {
  const gitIndexed = await buildIndexFromGit(rootPath, includeHidden);
  if (gitIndexed) return gitIndexed;
  return buildIndexFromScan(rootPath, includeHidden);
}

export class VaultIndexService {
  async getIndex(rootPath: string, includeHidden: boolean): Promise<CachedVaultIndex> {
    const key = cacheKey(rootPath, includeHidden);
    const cached = indexCache.get(key);
    if (cached && Date.now() - cached.scannedAt < INDEX_TTL_MS) {
      return cached;
    }

    const inFlight = inFlightBuilds.get(key);
    if (inFlight) return inFlight;

    const nextPromise = buildVaultIndex(rootPath, includeHidden)
      .then((next) => {
        if (inFlightBuilds.get(key) === nextPromise) {
          indexCache.set(key, next);
          while (indexCache.size > INDEX_MAX_KEYS) {
            const oldestKey = indexCache.keys().next().value;
            if (!oldestKey) break;
            indexCache.delete(oldestKey);
          }
        }
        return next;
      })
      .finally(() => {
        if (inFlightBuilds.get(key) === nextPromise) {
          inFlightBuilds.delete(key);
        }
      });

    inFlightBuilds.set(key, nextPromise);
    return nextPromise;
  }

  invalidate(rootPath: string): void {
    for (const key of [...indexCache.keys()]) {
      if (key.startsWith(`${rootPath}::`)) {
        indexCache.delete(key);
      }
    }
    for (const key of [...inFlightBuilds.keys()]) {
      if (key.startsWith(`${rootPath}::`)) {
        inFlightBuilds.delete(key);
      }
    }
  }

  async search(
    rootPath: string,
    query: string,
    limit: number,
    includeHidden: boolean,
  ): Promise<{ results: PathSearchResult[]; truncated: boolean }> {
    const index = await this.getIndex(rootPath, includeHidden);
    const normalizedQuery = normalizeQuery(query);
    const candidates = normalizedQuery
      ? index.entries.filter((entry) => entry.path.toLowerCase().includes(normalizedQuery))
      : index.entries;

    const ranked = candidates
      .map((entry) => ({
        ...entry,
        score: scoreEntry(entry, normalizedQuery),
      }))
      .sort((left, right) => {
        const scoreDelta = left.score - right.score;
        if (scoreDelta !== 0) return scoreDelta;
        return left.path.localeCompare(right.path);
      });

    return {
      results: ranked.slice(0, limit),
      truncated: index.truncated || ranked.length > limit,
    };
  }
}
