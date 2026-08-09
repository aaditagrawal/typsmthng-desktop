export function normalizeVaultRelativePath(input: string): string {
  return input.replace(/\\/g, "/").replace(/^\/+/, "");
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
