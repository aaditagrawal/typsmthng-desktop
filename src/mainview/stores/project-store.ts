import { create } from "zustand";

import type {
  AppMetadata,
  CompileBundle,
  FileConflictState,
  ProjectScaffold as SharedProjectScaffold,
  ProjectScaffoldFile as SharedProjectScaffoldFile,
  ProjectTemplateMeta as SharedProjectTemplateMeta,
  RecentVaultRecord,
  VaultRecord,
} from "../../shared/rpc";
import {
  desktopRpc,
  onActiveVaultClosed,
  onExternalVaultEvents,
  onMetadataUpdated,
} from "@/lib/desktop-rpc";
import { useEditorStore } from "./editor-store";

export interface ProjectFile {
  path: string;
  content: string;
  isBinary: boolean;
  binaryData?: Uint8Array;
  lastModified: number;
  loaded?: boolean;
  kind?: "file" | "directory";
  name?: string;
  parentPath?: string | null;
  extension?: string;
  isHidden?: boolean;
  sizeBytes?: number;
}
export type ProjectTemplateMeta = SharedProjectTemplateMeta;
export type ProjectScaffoldFile = SharedProjectScaffoldFile;
export type ProjectScaffold = SharedProjectScaffold;
export interface Project extends Omit<VaultRecord, "files"> {
  files: ProjectFile[];
  fileCount?: number;
}

export interface HomeWorkspace {
  id: string;
  name: string;
  createdAt: number;
  updatedAt: number;
}

type ProjectWorkspaceAssignments = Record<string, string>;

interface ProjectState {
  projects: Project[];
  metadata: AppMetadata | null;
  homeWorkspaces: HomeWorkspace[];
  projectWorkspaceAssignments: ProjectWorkspaceAssignments;
  selectedHomeWorkspaceId: string | null;
  currentProjectId: string | null;
  currentFilePath: string | null;
  sidebarOpen: boolean;
  loading: boolean;
  hasSelectedProject: boolean;
  activeConflict: FileConflictState | null;
  openVaultDialog: () => Promise<string | null>;
  loadProjects: () => Promise<void>;
  createProject: (name: string, scaffold?: ProjectScaffold) => Promise<string>;
  deleteProject: (id: string) => Promise<void>;
  renameProject: (id: string, name: string) => Promise<void>;
  createHomeWorkspace: (name: string, projectIds?: string[]) => Promise<string>;
  renameHomeWorkspace: (id: string, name: string) => Promise<void>;
  deleteHomeWorkspace: (id: string) => Promise<void>;
  assignProjectsToHomeWorkspace: (projectIds: string[], workspaceId: string | null) => Promise<void>;
  setSelectedHomeWorkspace: (id: string | null) => Promise<void>;
  selectProject: (id: string) => void;
  openRecentVault: (rootPath: string) => Promise<void>;
  goHome: () => void;
  selectFile: (path: string) => void;
  createFile: (path: string, content?: string) => Promise<void>;
  createFilesBatch: (entries: Array<{ path: string; content: string }>) => Promise<void>;
  deleteFile: (path: string) => Promise<void>;
  renameFile: (oldPath: string, newPath: string) => Promise<void>;
  updateFileContent: (path: string, content: string) => void;
  stageFileContent: (path: string, content: string) => void;
  addBinaryFile: (path: string, data: Uint8Array) => Promise<void>;
  addBinaryFilesBatch: (entries: Array<{ path: string; data: Uint8Array }>) => Promise<void>;
  createFolder: (path: string) => Promise<void>;
  deleteFolder: (path: string) => Promise<void>;
  moveFile: (oldPath: string, newPath: string) => Promise<void>;
  renameFolder: (oldPath: string, newPath: string) => Promise<void>;
  setSidebarOpen: (open: boolean) => void;
  saveCurrentProject: () => Promise<void>;
  getCurrentProject: () => Project | undefined;
  toggleFavoriteProject: (id: string) => Promise<void>;
  removeRecentProject: (id: string) => Promise<void>;
  setHiddenFilesVisible: (value: boolean) => Promise<void>;
  revealCurrentProjectInFinder: () => Promise<void>;
  revealPathInFinder: (projectPath: string) => Promise<void>;
  reloadConflictFile: () => Promise<void>;
  dismissConflict: () => void;
  getCompileBundle: (liveSource: string, currentFilePath?: string | null) => Promise<CompileBundle>;
}

let subscriptionsBound = false;

function basenameOf(input: string): string {
  const normalized = input.replace(/\\/g, "/");
  const separatorIndex = normalized.lastIndexOf("/");
  return separatorIndex === -1 ? normalized : normalized.slice(separatorIndex + 1);
}

function parentOf(input: string): string | null {
  const normalized = input.replace(/\\/g, "/");
  const separatorIndex = normalized.lastIndexOf("/");
  if (separatorIndex <= 0) return null;
  return normalized.slice(0, separatorIndex);
}

function extensionOf(input: string): string {
  const base = basenameOf(input);
  const dotIndex = base.lastIndexOf(".");
  if (dotIndex <= 0) return "";
  return base.slice(dotIndex).toLowerCase();
}

function normalizeRelativePath(input: string): string {
  return input.replace(/^\/+/, "").replace(/\\/g, "/");
}

function normalizeScaffold(
  scaffold?: ProjectScaffold,
): ProjectScaffold | undefined {
  if (!scaffold) return undefined;
  return {
    ...scaffold,
    mainFile: normalizeRelativePath(scaffold.mainFile),
    files: scaffold.files.map((file) => ({
      ...file,
      path: normalizeRelativePath(file.path),
    })),
  };
}

function isHiddenPath(input: string): boolean {
  return normalizeRelativePath(input).split("/").some((segment) => segment.startsWith("."));
}

function joinAbsolute(rootPath: string, relativePath: string): string {
  const normalizedRoot = rootPath.replace(/[\\/]+$/, "");
  return `${normalizedRoot}/${normalizeRelativePath(relativePath)}`;
}

function asRecentProject(record: RecentVaultRecord): Project {
  return {
    id: record.rootPath,
    rootPath: record.rootPath,
    name: record.name,
    files: [],
    fileCount: record.fileCount,
    mainFile: record.lastFilePath ?? "main.typ",
    createdAt: record.lastOpenedAt,
    updatedAt: record.lastOpenedAt,
  };
}

function sortEntries(entries: ProjectFile[]): ProjectFile[] {
  return [...entries].sort((left, right) => {
    if (left.kind !== right.kind) {
      return left.kind === "directory" ? -1 : 1;
    }
    return left.path.localeCompare(right.path);
  });
}

function mergeProjects(metadata: AppMetadata | null, activeProject?: Project | null): Project[] {
  const recentProjects = (metadata?.recentVaults ?? []).map(asRecentProject);
  if (!activeProject) return recentProjects;

  return [activeProject, ...recentProjects.filter((project) => project.id !== activeProject.id)];
}

function getActiveProject(state: Pick<ProjectState, "projects" | "currentProjectId">): Project | undefined {
  return state.projects.find((project) => project.id === state.currentProjectId);
}

function ensureDirectory(entries: ProjectFile[], directoryPath: string): ProjectFile[] {
  if (!directoryPath) return entries;
  if (entries.some((entry) => entry.path === directoryPath && entry.kind === "directory")) {
    return entries;
  }

  const parentPath = parentOf(directoryPath);
  const nextEntries = parentPath ? ensureDirectory(entries, parentPath) : entries;
  return [
    ...nextEntries,
    {
      path: directoryPath,
      name: basenameOf(directoryPath),
      kind: "directory",
      parentPath,
      extension: "",
      isHidden: isHiddenPath(directoryPath),
      isBinary: false,
      lastModified: Date.now(),
      sizeBytes: 0,
      loaded: true,
      content: "",
    },
  ];
}

function replaceEntry(entries: ProjectFile[], nextEntry: ProjectFile): ProjectFile[] {
  const withParents =
    nextEntry.parentPath && nextEntry.parentPath !== "."
      ? ensureDirectory(entries, nextEntry.parentPath)
      : entries;

  const nextEntries = withParents.filter((entry) => entry.path !== nextEntry.path);
  nextEntries.push(nextEntry);
  return sortEntries(nextEntries);
}

function removeEntry(entries: ProjectFile[], targetPath: string): ProjectFile[] {
  const normalizedTarget = normalizeRelativePath(targetPath);
  const prefix = `${normalizedTarget}/`;
  return sortEntries(
    entries.filter(
      (entry) => entry.path !== normalizedTarget && !entry.path.startsWith(prefix),
    ),
  );
}

function renameEntries(entries: ProjectFile[], oldPath: string, newPath: string): ProjectFile[] {
  const normalizedOld = normalizeRelativePath(oldPath);
  const normalizedNew = normalizeRelativePath(newPath);
  const oldPrefix = `${normalizedOld}/`;
  const newPrefix = `${normalizedNew}/`;

  const renamed = entries.map((entry) => {
    if (entry.path !== normalizedOld && !entry.path.startsWith(oldPrefix)) {
      return entry;
    }

    const suffix = entry.path === normalizedOld ? "" : entry.path.slice(oldPrefix.length);
    const nextPath = suffix ? `${newPrefix}${suffix}` : normalizedNew;
    return {
      ...entry,
      path: nextPath,
      name: basenameOf(nextPath),
      parentPath: parentOf(nextPath),
    };
  });

  const parentPath = parentOf(normalizedNew);
  return sortEntries(parentPath ? ensureDirectory(renamed, parentPath) : renamed);
}

function updateProjectList(projects: Project[], nextProject: Project): Project[] {
  const others = projects.filter((project) => project.id !== nextProject.id);
  return [nextProject, ...others];
}

function replaceEntryPreservingOrder(entries: ProjectFile[], nextEntry: ProjectFile): ProjectFile[] {
  const index = entries.findIndex((entry) => entry.path === nextEntry.path);
  if (index === -1) {
    return replaceEntry(entries, nextEntry);
  }

  const nextEntries = entries.slice();
  nextEntries[index] = nextEntry;
  return nextEntries;
}

function updateWindowTitle() {
  const { currentProjectId, currentFilePath, projects } = useProjectStore.getState()
  const project = projects.find((p) => p.id === currentProjectId)
  if (!project || !currentFilePath) {
    void desktopRpc.request.setWindowTitle({ title: "typsmthng" })
    return
  }
  const fileName = currentFilePath.split("/").pop() ?? ""
  void desktopRpc.request.setWindowTitle({
    title: `${project.name} — ${fileName} — typsmthng`,
  })
}

function applyMetadataState(metadata: AppMetadata) {
  useProjectStore.setState((state) => ({
    metadata,
    projects: mergeProjects(metadata, getActiveProject(state) ?? null),
  }));
}

function clearSelectionState() {
  useProjectStore.setState((state) => ({
    projects: mergeProjects(state.metadata, null),
    currentProjectId: null,
    currentFilePath: null,
    hasSelectedProject: false,
    activeConflict: null,
  }));

  useEditorStore.setState({ source: "", isDirty: false, saveStatus: "saved" });
  updateWindowTitle()
}

async function hydrateFile(rootPath: string, filePath: string): Promise<void> {
  const entry = await desktopRpc.request.readFile({ rootPath, path: filePath });
  if (!entry) return;

  useProjectStore.setState((state) => {
    const project = getActiveProject(state);
    if (!project || project.rootPath !== rootPath) return state;

    const nextProject: Project = {
      ...project,
      files: replaceEntryPreservingOrder(project.files, entry),
      updatedAt: Date.now(),
    };

    return {
      projects: updateProjectList(state.projects, nextProject),
    };
  });
}

async function applyVault(project: Project | null): Promise<void> {
  useProjectStore.setState((state) => {
    if (!project) {
      return {
        projects: mergeProjects(state.metadata, null),
        currentProjectId: null,
        currentFilePath: null,
        hasSelectedProject: false,
        activeConflict: null,
      };
    }

    return {
      projects: updateProjectList(state.projects, {
        ...project,
        files: sortEntries(project.files),
      }),
      currentProjectId: project.id,
      currentFilePath: project.mainFile,
      hasSelectedProject: true,
      activeConflict: null,
    };
  });

  const mainTextFile = project?.files.find(
    (entry) => entry.path === project.mainFile && entry.kind === "file" && !entry.isBinary,
  );

  useEditorStore.setState({
    source: mainTextFile?.content ?? "",
    isDirty: false,
    saveStatus: "saved",
  });

  if (project?.rootPath) {
    await desktopRpc.request.persistLastFile({
      rootPath: project.rootPath,
      path: project.mainFile,
    });
  }

  updateWindowTitle()
}

function bindSubscriptions() {
  if (subscriptionsBound) return;
  subscriptionsBound = true;

  onMetadataUpdated((metadata) => {
    applyMetadataState(metadata);
  });

  onActiveVaultClosed(() => {
    clearSelectionState();
  });

  onExternalVaultEvents(({ rootPath, events }) => {
    if (useProjectStore.getState().currentProjectId !== rootPath) return;

    let shouldReloadCurrentFile = false;
    let conflictPath: string | null = null;

    useProjectStore.setState((state) => {
      const project = getActiveProject(state);
      if (!project || project.rootPath !== rootPath) return state;

      let nextFiles = project.files;
      let nextMainFile = project.mainFile;
      let nextCurrentFilePath = state.currentFilePath;

      for (const event of events) {
        if (event.kind === "unlink" || event.kind === "unlinkDir") {
          nextFiles = removeEntry(nextFiles, event.path);
          if (nextCurrentFilePath === event.path) {
            nextCurrentFilePath =
              nextFiles.find((entry) => entry.kind === "file" && !entry.isBinary)?.path ?? null;
          }
          if (nextMainFile === event.path && nextCurrentFilePath) {
            nextMainFile = nextCurrentFilePath;
          }
          continue;
        }

        if (event.kind === "addDir") {
          nextFiles = replaceEntry(nextFiles, {
            path: event.path,
            name: basenameOf(event.path),
            kind: "directory",
            parentPath: parentOf(event.path),
            extension: "",
            isHidden: isHiddenPath(event.path),
            isBinary: false,
            lastModified: event.lastModified ?? Date.now(),
            sizeBytes: 0,
            loaded: true,
            content: "",
          });
          continue;
        }

        const existing = nextFiles.find((entry) => entry.path === event.path);
        nextFiles = replaceEntry(nextFiles, {
          path: event.path,
          name: basenameOf(event.path),
          kind: "file",
          parentPath: parentOf(event.path),
          extension: extensionOf(event.path),
          isHidden: isHiddenPath(event.path),
          isBinary: event.isBinary ?? existing?.isBinary ?? false,
          lastModified: event.lastModified ?? Date.now(),
          sizeBytes: event.sizeBytes ?? existing?.sizeBytes ?? 0,
          loaded: false,
          content: existing?.loaded ? existing.content : "",
          binaryData: undefined,
        });

        if (state.currentFilePath === event.path) {
          if (useEditorStore.getState().isDirty) {
            conflictPath = event.path;
          } else {
            shouldReloadCurrentFile = true;
          }
        }
      }

      const nextProject: Project = {
        ...project,
        files: nextFiles,
        mainFile: nextMainFile,
        updatedAt: Date.now(),
      };

      return {
        projects: updateProjectList(state.projects, nextProject),
        currentFilePath: nextCurrentFilePath,
        activeConflict: conflictPath
          ? { path: conflictPath, changedAt: Date.now() }
          : state.activeConflict,
      };
    });

    const currentFilePath = useProjectStore.getState().currentFilePath;
    if (shouldReloadCurrentFile && currentFilePath) {
      void hydrateFile(rootPath, currentFilePath);
      useEditorStore.setState({ isDirty: false, saveStatus: "saved" });
    }
  });
}

export const useProjectStore = create<ProjectState>((set, get) => ({
  projects: [],
  metadata: null,
  homeWorkspaces: [],
  projectWorkspaceAssignments: {},
  selectedHomeWorkspaceId: null,
  currentProjectId: null,
  currentFilePath: null,
  sidebarOpen: true,
  loading: true,
  hasSelectedProject: false,
  activeConflict: null,

  openVaultDialog: async () => {
    const vault = await desktopRpc.request.openVaultDialog();
    await applyVault(vault);
    return vault?.id ?? null;
  },

  loadProjects: async () => {
    bindSubscriptions();
    await desktopRpc.request.waitUntilReady();
    const bootstrap = await desktopRpc.request.getBootstrapState();
    const activeVault = bootstrap.activeVault;

    set({
      metadata: bootstrap.metadata,
      projects: mergeProjects(bootstrap.metadata, activeVault),
      currentProjectId: activeVault?.id ?? null,
      currentFilePath: activeVault?.mainFile ?? null,
      hasSelectedProject: Boolean(activeVault),
      loading: false,
      activeConflict: null,
    });

    if (activeVault) {
      const mainTextFile = activeVault.files.find(
        (entry) =>
          entry.path === activeVault.mainFile &&
          (entry.kind ?? "file") === "file" &&
          !entry.isBinary,
      );
      useEditorStore.setState({
        source: mainTextFile?.content ?? "",
        isDirty: false,
        saveStatus: "saved",
      });
    }
  },

  createProject: async (name, scaffold) => {
    const vault = await desktopRpc.request.createVault({
      name,
      scaffold: normalizeScaffold(scaffold),
    });
    if (!vault) return "";
    await applyVault(vault);
    return vault.id;
  },

  deleteProject: async (id) => {
    const metadata = await desktopRpc.request.removeRecentVault({ rootPath: id });
    applyMetadataState(metadata);
  },

  renameProject: async (id, name) => {
    set((state) => ({
      projects: state.projects.map((project) =>
        project.id === id ? { ...project, name, updatedAt: Date.now() } : project,
      ),
    }));
  },

  createHomeWorkspace: async (name) => {
    const now = Date.now();
    const workspace: HomeWorkspace = {
      id: `workspace-${now}`,
      name,
      createdAt: now,
      updatedAt: now,
    };
    set((state) => ({
      homeWorkspaces: [...state.homeWorkspaces, workspace],
    }));
    return workspace.id;
  },

  renameHomeWorkspace: async (id, name) => {
    set((state) => ({
      homeWorkspaces: state.homeWorkspaces.map((workspace) =>
        workspace.id === id ? { ...workspace, name, updatedAt: Date.now() } : workspace,
      ),
    }));
  },

  deleteHomeWorkspace: async (id) => {
    set((state) => ({
      homeWorkspaces: state.homeWorkspaces.filter((workspace) => workspace.id !== id),
      selectedHomeWorkspaceId:
        state.selectedHomeWorkspaceId === id ? null : state.selectedHomeWorkspaceId,
      projectWorkspaceAssignments: Object.fromEntries(
        Object.entries(state.projectWorkspaceAssignments).filter(
          ([, workspaceId]) => workspaceId !== id,
        ),
      ),
    }));
  },

  assignProjectsToHomeWorkspace: async (projectIds, workspaceId) => {
    set((state) => {
      const nextAssignments: ProjectWorkspaceAssignments = {
        ...state.projectWorkspaceAssignments,
      };

      for (const projectId of projectIds) {
        if (workspaceId) nextAssignments[projectId] = workspaceId;
        else delete nextAssignments[projectId];
      }

      return { projectWorkspaceAssignments: nextAssignments };
    });
  },

  setSelectedHomeWorkspace: async (id) => {
    set({ selectedHomeWorkspaceId: id });
  },

  selectProject: (id) => {
    void get().openRecentVault(id);
  },

  openRecentVault: async (rootPath) => {
    const vault = await desktopRpc.request.openRecentVault({ rootPath });
    await applyVault(vault);
  },

  goHome: () => {
    const project = get().getCurrentProject();
    if (!project) {
      clearSelectionState();
      return;
    }

    const currentFilePath = get().currentFilePath;
    const currentEntry = currentFilePath
      ? project.files.find((entry) => entry.path === currentFilePath)
      : null;
    const source = useEditorStore.getState().source;

    if (currentFilePath && currentEntry && (currentEntry.kind ?? "file") === "file" && !currentEntry.isBinary) {
      get().stageFileContent(currentFilePath, source);
    }

    clearSelectionState();

    void (async () => {
      try {
        await desktopRpc.request.flushWrites({ rootPath: project.rootPath });
        await desktopRpc.request.closeVault();
      } catch (error) {
        console.error("Failed to close vault while returning home:", error);
      }
    })();
  },

  selectFile: (filePath) => {
    const project = get().getCurrentProject();
    if (!project) return;

    const entry = project.files.find((candidate) => candidate.path === filePath);
    set({ currentFilePath: filePath, activeConflict: null });
    void desktopRpc.request.persistLastFile({ rootPath: project.rootPath, path: filePath });
    updateWindowTitle();

    if (entry && (entry.kind ?? "file") === "file" && !entry.loaded) {
      void hydrateFile(project.rootPath, filePath);
      return;
    }
  },

  createFile: async (filePath, content = "") => {
    const project = get().getCurrentProject();
    if (!project) return;

    const normalizedPath = normalizeRelativePath(filePath);
    const entry = await desktopRpc.request.createFile({
      rootPath: project.rootPath,
      path: normalizedPath,
      content,
    });
    if (!entry) return;

    set((state) => {
      const activeProject = getActiveProject(state);
      if (!activeProject) return state;

      const nextProject: Project = {
        ...activeProject,
        files: replaceEntry(activeProject.files, entry),
        updatedAt: Date.now(),
      };

      return {
        projects: updateProjectList(state.projects, nextProject),
        currentFilePath: entry.path,
      };
    });
  },

  createFilesBatch: async (entries) => {
    const project = get().getCurrentProject();
    if (!project || entries.length === 0) return;

    const normalizedEntries = entries.map((entry) => ({
      path: normalizeRelativePath(entry.path),
      content: entry.content,
    }));

    await desktopRpc.request.createFilesBatch({
      rootPath: project.rootPath,
      entries: normalizedEntries,
    });

    set((state) => {
      const activeProject = getActiveProject(state);
      if (!activeProject) return state;

      let nextFiles = activeProject.files;
      let lastPath: string | null = null;
      for (const entry of normalizedEntries) {
        lastPath = entry.path;
        nextFiles = replaceEntry(nextFiles, {
          path: entry.path,
          name: basenameOf(entry.path),
          kind: "file",
          parentPath: parentOf(entry.path),
          extension: extensionOf(entry.path),
          isHidden: isHiddenPath(entry.path),
          isBinary: false,
          lastModified: Date.now(),
          sizeBytes: entry.content.length,
          loaded: true,
          content: entry.content,
        });
      }

      const nextProject: Project = {
        ...activeProject,
        files: nextFiles,
        updatedAt: Date.now(),
      };

      return {
        projects: updateProjectList(state.projects, nextProject),
        currentFilePath: lastPath ?? state.currentFilePath,
      };
    });
  },

  deleteFile: async (filePath) => {
    const project = get().getCurrentProject();
    if (!project) return;

    const normalizedPath = normalizeRelativePath(filePath);
    await desktopRpc.request.deletePath({
      rootPath: project.rootPath,
      path: normalizedPath,
    });

    set((state) => {
      const activeProject = getActiveProject(state);
      if (!activeProject) return state;

      const nextProject: Project = {
        ...activeProject,
        files: removeEntry(activeProject.files, normalizedPath),
        updatedAt: Date.now(),
      };

      const nextCurrentFilePath =
        state.currentFilePath === normalizedPath
          ? nextProject.files.find((entry) => entry.kind === "file" && !entry.isBinary)?.path ??
            null
          : state.currentFilePath;

      return {
        projects: updateProjectList(state.projects, nextProject),
        currentFilePath: nextCurrentFilePath,
      };
    });
  },

  renameFile: async (oldPath, newPath) => {
    const project = get().getCurrentProject();
    if (!project) return;

    const normalizedOld = normalizeRelativePath(oldPath);
    const normalizedNew = normalizeRelativePath(newPath);
    await desktopRpc.request.renamePath({
      rootPath: project.rootPath,
      oldPath: normalizedOld,
      newPath: normalizedNew,
    });

    set((state) => {
      const activeProject = getActiveProject(state);
      if (!activeProject) return state;

      const nextProject: Project = {
        ...activeProject,
        files: renameEntries(activeProject.files, normalizedOld, normalizedNew),
        updatedAt: Date.now(),
      };

      return {
        projects: updateProjectList(state.projects, nextProject),
        currentFilePath:
          state.currentFilePath === normalizedOld ? normalizedNew : state.currentFilePath,
      };
    });
  },

  updateFileContent: (filePath, content) => {
    const project = get().getCurrentProject();
    if (!project) return;

    const normalizedPath = normalizeRelativePath(filePath);
    const previous = project.files.find((entry) => entry.path === normalizedPath);
    if (
      previous
      && (previous.kind ?? "file") === "file"
      && !previous.isBinary
      && previous.loaded
      && previous.content === content
    ) {
      return;
    }

    void desktopRpc.request.stageFileWrite({
      rootPath: project.rootPath,
      path: normalizedPath,
      content,
    });

    set((state) => {
      const activeProject = getActiveProject(state);
      if (!activeProject) return state;

      const previous = activeProject.files.find((entry) => entry.path === normalizedPath);
      const nextEntry: ProjectFile = {
        ...(previous ?? {}),
        path: normalizedPath,
        name: basenameOf(normalizedPath),
        kind: "file",
        parentPath: parentOf(normalizedPath),
        extension: extensionOf(normalizedPath),
        isHidden: isHiddenPath(normalizedPath),
        isBinary: false,
        lastModified: Date.now(),
        sizeBytes: content.length,
        loaded: true,
        content,
      };

      const nextProject: Project = {
        ...activeProject,
        files: replaceEntryPreservingOrder(activeProject.files, nextEntry),
        updatedAt: Date.now(),
      };

      return {
        projects: updateProjectList(state.projects, nextProject),
      };
    });
  },

  stageFileContent: (filePath, content) => {
    const project = get().getCurrentProject();
    if (!project) return;

    void desktopRpc.request.stageFileWrite({
      rootPath: project.rootPath,
      path: normalizeRelativePath(filePath),
      content,
    });
  },

  addBinaryFile: async (filePath, data) => {
    await get().addBinaryFilesBatch([{ path: filePath, data }]);
  },

  addBinaryFilesBatch: async (entries) => {
    const project = get().getCurrentProject();
    if (!project || entries.length === 0) return;

    const normalizedEntries = entries.map((entry) => ({
      path: normalizeRelativePath(entry.path),
      data: entry.data,
    }));

    await desktopRpc.request.addBinaryFilesBatch({
      rootPath: project.rootPath,
      entries: normalizedEntries,
    });

    set((state) => {
      const activeProject = getActiveProject(state);
      if (!activeProject) return state;

      let nextFiles = activeProject.files;
      for (const entry of normalizedEntries) {
        nextFiles = replaceEntry(nextFiles, {
          path: entry.path,
          name: basenameOf(entry.path),
          kind: "file",
          parentPath: parentOf(entry.path),
          extension: extensionOf(entry.path),
          isHidden: isHiddenPath(entry.path),
          isBinary: true,
          lastModified: Date.now(),
          sizeBytes: entry.data.byteLength,
          loaded: true,
          content: "",
          binaryData: entry.data,
        });
      }

      const nextProject: Project = {
        ...activeProject,
        files: nextFiles,
        updatedAt: Date.now(),
      };

      return {
        projects: updateProjectList(state.projects, nextProject),
      };
    });
  },

  createFolder: async (folderPath) => {
    const project = get().getCurrentProject();
    if (!project) return;

    const normalizedPath = normalizeRelativePath(folderPath);
    await desktopRpc.request.createFolder({
      rootPath: project.rootPath,
      path: normalizedPath,
    });

    set((state) => {
      const activeProject = getActiveProject(state);
      if (!activeProject) return state;

      const nextProject: Project = {
        ...activeProject,
        files: replaceEntry(activeProject.files, {
          path: normalizedPath,
          name: basenameOf(normalizedPath),
          kind: "directory",
          parentPath: parentOf(normalizedPath),
          extension: "",
          isHidden: isHiddenPath(normalizedPath),
          isBinary: false,
          lastModified: Date.now(),
          sizeBytes: 0,
          loaded: true,
          content: "",
        }),
        updatedAt: Date.now(),
      };

      return {
        projects: updateProjectList(state.projects, nextProject),
      };
    });
  },

  deleteFolder: async (folderPath) => {
    const project = get().getCurrentProject();
    if (!project) return;

    const normalizedPath = normalizeRelativePath(folderPath);
    await desktopRpc.request.deletePath({
      rootPath: project.rootPath,
      path: normalizedPath,
    });

    set((state) => {
      const activeProject = getActiveProject(state);
      if (!activeProject) return state;

      const nextProject: Project = {
        ...activeProject,
        files: removeEntry(activeProject.files, normalizedPath),
        updatedAt: Date.now(),
      };

      const nextCurrentFilePath =
        state.currentFilePath === normalizedPath ||
        state.currentFilePath?.startsWith(`${normalizedPath}/`)
          ? nextProject.files.find((entry) => entry.kind === "file" && !entry.isBinary)?.path ??
            null
          : state.currentFilePath;

      return {
        projects: updateProjectList(state.projects, nextProject),
        currentFilePath: nextCurrentFilePath,
      };
    });
  },

  moveFile: async (oldPath, newPath) => {
    await get().renameFile(oldPath, newPath);
  },

  renameFolder: async (oldPath, newPath) => {
    const project = get().getCurrentProject();
    if (!project) return;

    const normalizedOld = normalizeRelativePath(oldPath);
    const normalizedNew = normalizeRelativePath(newPath);
    await desktopRpc.request.renamePath({
      rootPath: project.rootPath,
      oldPath: normalizedOld,
      newPath: normalizedNew,
    });

    set((state) => {
      const activeProject = getActiveProject(state);
      if (!activeProject) return state;

      const nextProject: Project = {
        ...activeProject,
        files: renameEntries(activeProject.files, normalizedOld, normalizedNew),
        updatedAt: Date.now(),
      };

      const nextCurrentFilePath =
        state.currentFilePath === normalizedOld ||
        state.currentFilePath?.startsWith(`${normalizedOld}/`)
          ? state.currentFilePath?.replace(normalizedOld, normalizedNew) ?? null
          : state.currentFilePath;

      return {
        projects: updateProjectList(state.projects, nextProject),
        currentFilePath: nextCurrentFilePath,
      };
    });
  },

  setSidebarOpen: (sidebarOpen) => {
    set({ sidebarOpen });
  },

  saveCurrentProject: async () => {
    const project = get().getCurrentProject();
    if (!project) return;

    const currentFilePath = get().currentFilePath;
    const source = useEditorStore.getState().source;
    const currentEntry = currentFilePath
      ? project.files.find((entry) => entry.path === currentFilePath)
      : null;
    if (currentFilePath && currentEntry && (currentEntry.kind ?? "file") === "file" && !currentEntry.isBinary) {
      get().updateFileContent(currentFilePath, source);
    }

    useEditorStore.setState({ saveStatus: "saving" });
    await desktopRpc.request.flushWrites({ rootPath: project.rootPath });
    useEditorStore.setState({ isDirty: false, saveStatus: "saved" });
  },

  getCurrentProject: () => getActiveProject(get()),

  toggleFavoriteProject: async (id) => {
    set((state) => {
      const metadata = state.metadata;
      if (!metadata) return state;

      return {
        metadata: {
          ...metadata,
          recentVaults: metadata.recentVaults.map((record) =>
            record.rootPath === id ? { ...record, favorite: !record.favorite } : record,
          ),
        },
      };
    });

    const metadata = await desktopRpc.request.toggleFavoriteVault({ rootPath: id });
    applyMetadataState(metadata);
  },

  removeRecentProject: async (id) => {
    const metadata = await desktopRpc.request.removeRecentVault({ rootPath: id });
    applyMetadataState(metadata);
  },

  setHiddenFilesVisible: async (value) => {
    const project = get().getCurrentProject();
    if (!project) return;

    const result = await desktopRpc.request.setHiddenFilesVisible({
      rootPath: project.rootPath,
      value,
    });
    applyMetadataState(result.metadata);
    await applyVault(result.vault);
  },

  revealCurrentProjectInFinder: async () => {
    const project = get().getCurrentProject();
    if (!project) return;
    await desktopRpc.request.revealInFinder({ absolutePath: project.rootPath });
  },

  revealPathInFinder: async (projectPath) => {
    const project = get().getCurrentProject();
    if (!project) return;
    await desktopRpc.request.revealInFinder({
      absolutePath: joinAbsolute(project.rootPath, projectPath),
    });
  },

  reloadConflictFile: async () => {
    const project = get().getCurrentProject();
    const conflict = get().activeConflict;
    if (!project || !conflict) return;

    await hydrateFile(project.rootPath, conflict.path);
    set({ activeConflict: null });
    useEditorStore.setState({ isDirty: false, saveStatus: "saved" });
  },

  dismissConflict: () => {
    set({ activeConflict: null });
  },

  getCompileBundle: async (liveSource, currentFilePath) => {
    const project = get().getCurrentProject();
    if (!project) {
      return {
        mainPath: currentFilePath ? `/${currentFilePath.replace(/^\/+/, '')}` : "/main.typ",
        mainSource: liveSource,
        extraFiles: [],
        extraBinaryFiles: [],
      };
    }

    return desktopRpc.request.getCompileBundle({
      rootPath: project.rootPath,
      currentFilePath: currentFilePath ?? get().currentFilePath,
      liveSource,
    });
  },
}));
