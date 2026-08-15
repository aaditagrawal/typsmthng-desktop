export function normalizeVaultRelativePath(input: string): string {
  return input.replace(/\\/g, "/").replace(/^\/+/, "");
}

function pathExtension(input: string): string {
  const base = input.split("/").pop() ?? input;
  const lastDot = base.lastIndexOf(".");
  if (lastDot <= 0 || lastDot === base.length - 1) return "";
  return base.slice(lastDot).toLowerCase();
}

/**
 * Compile root for a vault. Prefer conventional main.typ so editing a chapter
 * still previews the whole document (Overleaf-style). Fall back to the current
 * .typ file, then the first .typ in the tree.
 */
export function resolveCompileMainFile(
  files: Array<{
    path: string;
    kind: "file" | "directory";
    extension: string;
  }>,
  currentFilePath?: string | null,
): string {
  const normalizedCurrent = currentFilePath
    ? normalizeVaultRelativePath(currentFilePath)
    : null;
  const typFiles = files.filter((file) => {
    if (file.kind !== "file") return false;
    const extension = file.extension || pathExtension(file.path);
    return extension === ".typ";
  });

  const mainTyp = typFiles.find(
    (file) => normalizeVaultRelativePath(file.path) === "main.typ",
  );
  if (mainTyp) return "main.typ";

  if (
    normalizedCurrent
    && typFiles.some((file) => normalizeVaultRelativePath(file.path) === normalizedCurrent)
  ) {
    return normalizedCurrent;
  }

  const firstTyp = typFiles[0];
  if (firstTyp) return normalizeVaultRelativePath(firstTyp.path);
  return normalizedCurrent || "main.typ";
}

/** Prefer scaffold main, then recent last file, then conventional main.typ / first .typ. */
export function resolveVaultMainFile(
  files: Array<{
    path: string;
    kind: "file" | "directory";
    extension: string;
    isBinary: boolean;
  }>,
  options?: {
    recent?: { lastFilePath: string | null };
    preferredMainFile?: string | null;
  },
): string {
  const preferredFromScaffold = options?.preferredMainFile
    ? normalizeVaultRelativePath(options.preferredMainFile)
    : null;
  if (
    preferredFromScaffold
    && files.some((file) => file.path === preferredFromScaffold)
  ) {
    return preferredFromScaffold;
  }

  const recentPath = options?.recent?.lastFilePath;
  if (recentPath && files.some((file) => file.path === recentPath)) {
    return recentPath;
  }
  const mainTyp = files.find((file) => file.path === "main.typ");
  if (mainTyp) return mainTyp.path;
  const firstTyp = files.find((file) => file.kind === "file" && file.extension === ".typ");
  if (firstTyp) return firstTyp.path;
  const firstText = files.find((file) => file.kind === "file" && !file.isBinary);
  if (firstText) return firstText.path;
  return files.find((file) => file.kind === "file")?.path ?? "main.typ";
}
