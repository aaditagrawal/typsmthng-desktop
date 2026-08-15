import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";

const FONT_EXTENSIONS = new Set([".ttf", ".otf", ".ttc", ".woff", ".woff2"]);
const MAX_FILE_BYTES = 8 * 1024 * 1024;
const MAX_FILES_PER_FAMILY = 8;
const MAX_DEPTH = 4;

function systemFontDirectories(): string[] {
  const home = os.homedir();
  if (process.platform === "darwin") {
    return [
      path.join(home, "Library", "Fonts"),
      "/Library/Fonts",
      "/System/Library/Fonts",
    ];
  }
  if (process.platform === "win32") {
    const windir = process.env.WINDIR || "C:\\Windows";
    const local = process.env.LOCALAPPDATA || "";
    return [
      path.join(windir, "Fonts"),
      local ? path.join(local, "Microsoft", "Windows", "Fonts") : "",
    ].filter(Boolean);
  }
  return [
    path.join(home, ".local", "share", "fonts"),
    path.join(home, ".fonts"),
    "/usr/local/share/fonts",
    "/usr/share/fonts",
  ];
}

function familyKey(value: string): string {
  return value.toLowerCase().replace(/[\s-_]+/g, "");
}

async function collectFontFiles(root: string, depth: number, out: string[]): Promise<void> {
  if (depth > MAX_DEPTH) return;
  let entries: Array<{ name: string; isDirectory(): boolean; isFile(): boolean }>
  try {
    entries = await fs.readdir(root, { withFileTypes: true });
  } catch {
    return;
  }
  for (const entry of entries) {
    const full = path.join(root, entry.name);
    if (entry.isDirectory()) {
      await collectFontFiles(full, depth + 1, out);
      continue;
    }
    if (!entry.isFile()) continue;
    const ext = path.extname(entry.name).toLowerCase();
    if (FONT_EXTENSIONS.has(ext)) out.push(full);
  }
}

export async function loadSystemFontFiles(families: string[]): Promise<{
  files: Array<{ family: string; data: Uint8Array }>;
}> {
  const wanted = families
    .map((family) => family.trim())
    .filter(Boolean)
    .map((family) => ({ family, key: familyKey(family) }));
  if (wanted.length === 0) return { files: [] };

  const fontPaths: string[] = [];
  for (const dir of systemFontDirectories()) {
    await collectFontFiles(dir, 0, fontPaths);
  }

  const used = new Set<string>();
  const files: Array<{ family: string; data: Uint8Array }> = [];

  for (const target of wanted) {
    let matched = 0;
    for (const fontPath of fontPaths) {
      if (matched >= MAX_FILES_PER_FAMILY) break;
      if (used.has(fontPath)) continue;
      const stem = familyKey(path.basename(fontPath, path.extname(fontPath)));
      if (!stem.includes(target.key) && !target.key.includes(stem)) continue;
      try {
        const stat = await fs.stat(fontPath);
        if (stat.size <= 0 || stat.size > MAX_FILE_BYTES) continue;
        const buffer = await fs.readFile(fontPath);
        files.push({ family: target.family, data: new Uint8Array(buffer) });
        used.add(fontPath);
        matched += 1;
      } catch {
        // Skip unreadable font files.
      }
    }
  }

  return { files };
}
