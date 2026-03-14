import type { RPCSchema } from "electrobun/bun";
import type { UpdateState } from "./update-types";

export interface ProjectTemplateMeta {
  source: "typst-universe" | "built-in";
  resolvedSpec: string;
  templateEntrypoint: string;
  layoutLocked: boolean;
  createdAt: number;
  initCommand?: string;
}

export interface ProjectScaffoldFile {
  path: string;
  content: string;
  isBinary: boolean;
  binaryData?: Uint8Array;
}

export interface ProjectScaffold {
  files: ProjectScaffoldFile[];
  mainFile: string;
  templateMeta?: ProjectTemplateMeta;
}

export interface VaultPathEntry {
  path: string;
  name: string;
  kind: "file" | "directory";
  parentPath: string | null;
  extension: string;
  isHidden: boolean;
  isBinary: boolean;
  lastModified: number;
  sizeBytes: number;
}

export interface VaultFileEntry extends VaultPathEntry {
  loaded: boolean;
  content: string;
  binaryData?: Uint8Array;
}

export interface VaultRecord {
  id: string;
  rootPath: string;
  name: string;
  files: VaultFileEntry[];
  mainFile: string;
  createdAt: number;
  updatedAt: number;
  templateMeta?: ProjectTemplateMeta;
}

export interface RecentDocumentRecord {
  path: string;
  lastOpenedAt: number;
}

export interface RecentVaultRecord {
  id: string;
  rootPath: string;
  name: string;
  favorite: boolean;
  hiddenFilesVisible: boolean;
  fileCount?: number;
  lastOpenedAt: number;
  lastFilePath: string | null;
  recentDocuments: RecentDocumentRecord[];
}

export interface WindowState {
  width: number;
  height: number;
  x?: number;
  y?: number;
}

export interface AppMetadata {
  version: 1;
  recentVaults: RecentVaultRecord[];
  reopenLastVaultPath: string | null;
  windowState: WindowState | null;
}

export interface PathSearchResult extends VaultPathEntry {
  score: number;
}

export interface TextSearchResult {
  path: string;
  line: number;
  column: number;
  preview: string;
}

export interface CompileBundle {
  mainPath: string;
  mainSource: string;
  extraFiles: Array<{ path: string; content: string }>;
  extraBinaryFiles: Array<{ path: string; data: Uint8Array }>;
}

export interface BootstrapState {
  metadata: AppMetadata;
  activeVault: VaultRecord | null;
}

export interface ExternalVaultEvent {
  kind: "add" | "change" | "unlink" | "addDir" | "unlinkDir";
  path: string;
  isDirectory: boolean;
  sizeBytes?: number;
  lastModified?: number;
  isBinary?: boolean;
}

export interface FileConflictState {
  path: string;
  changedAt: number;
}

export interface CreateVaultParams {
  name: string;
  scaffold?: ProjectScaffold;
}

export type DesktopRPC = {
  bun: RPCSchema<{
    requests: {
      waitUntilReady: {
        params: void;
        response: { ready: true };
      };
      getBootstrapState: {
        params: void;
        response: BootstrapState;
      };
      openVaultDialog: {
        params: void;
        response: VaultRecord | null;
      };
      openRecentVault: {
        params: { rootPath: string };
        response: VaultRecord | null;
      };
      createVault: {
        params: CreateVaultParams;
        response: VaultRecord | null;
      };
      closeVault: {
        params: void;
        response: { ok: true };
      };
      readFile: {
        params: { rootPath: string; path: string };
        response: VaultFileEntry | null;
      };
      stageFileWrite: {
        params: { rootPath: string; path: string; content: string };
        response: { queuedAt: number };
      };
      flushWrites: {
        params: { rootPath?: string; path?: string };
        response: { ok: true };
      };
      createFile: {
        params: { rootPath: string; path: string; content?: string };
        response: VaultFileEntry | null;
      };
      createFilesBatch: {
        params: { rootPath: string; entries: Array<{ path: string; content: string }> };
        response: { ok: true };
      };
      addBinaryFilesBatch: {
        params: { rootPath: string; entries: Array<{ path: string; data: Uint8Array }> };
        response: { ok: true };
      };
      createFolder: {
        params: { rootPath: string; path: string };
        response: { ok: true };
      };
      duplicateFile: {
        params: { rootPath: string; sourcePath: string; targetPath: string };
        response: VaultFileEntry | null;
      };
      renamePath: {
        params: { rootPath: string; oldPath: string; newPath: string };
        response: { ok: true };
      };
      deletePath: {
        params: { rootPath: string; path: string };
        response: { ok: true };
      };
      revealInFinder: {
        params: { absolutePath: string };
        response: { ok: boolean };
      };
      openPath: {
        params: { absolutePath: string };
        response: { ok: boolean };
      };
      searchVaultPaths: {
        params: { rootPath: string; query: string; limit: number; includeHidden: boolean };
        response: { results: PathSearchResult[]; truncated: boolean };
      };
      searchVaultText: {
        params: { rootPath: string; query: string; limit: number; includeHidden: boolean };
        response: { results: TextSearchResult[]; truncated: boolean };
      };
      setHiddenFilesVisible: {
        params: { rootPath: string; value: boolean };
        response: { metadata: AppMetadata; vault: VaultRecord | null };
      };
      toggleFavoriteVault: {
        params: { rootPath: string };
        response: AppMetadata;
      };
      removeRecentVault: {
        params: { rootPath: string };
        response: AppMetadata;
      };
      persistLastFile: {
        params: { rootPath: string; path: string | null };
        response: AppMetadata;
      };
      getCompileBundle: {
        params: { rootPath: string; currentFilePath: string | null; liveSource: string };
        response: CompileBundle;
      };
      getVaultStats: {
        params: { rootPath: string; includeHidden: boolean };
        response: { fileCount: number };
      };
      setWindowTitle: {
        params: { title: string };
        response: { ok: true };
      };
      checkForUpdate: {
        params: void;
        response: UpdateState;
      };
      downloadUpdate: {
        params: void;
        response: UpdateState;
      };
      applyUpdate: {
        params: void;
        response: void;
      };
      quitApp: {
        params: void;
        response: void;
      };
    };
  }>;
  webview: RPCSchema<{
    messages: {
      updateStateChanged: UpdateState;
      externalVaultEvents: {
        rootPath: string;
        events: ExternalVaultEvent[];
      };
      metadataUpdated: AppMetadata;
      activeVaultClosed: void;
    };
  }>;
};
