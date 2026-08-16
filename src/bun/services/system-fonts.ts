import fs from "node:fs/promises";
import os from "node:os";
import path from "node:path";

const FONT_EXTENSIONS = new Set([".ttf", ".otf", ".ttc", ".woff", ".woff2"]);
// Matched files are loaded whole; .ttc collections (Apple system fonts, CJK
// faces) routinely exceed the old 8 MB cap, so allow up to 64 MB.
const MAX_FILE_BYTES = 64 * 1024 * 1024;
const MAX_FILES_PER_FAMILY = 8;
const MAX_DEPTH = 4;
const MAX_TTC_FONTS = 32;
const MAX_NAME_TABLE_BYTES = 512 * 1024;

const SFNT_VERSION_TRUETYPE = 0x00010000;
const SFNT_VERSION_APPLE = 0x74727565; // 'true'
const SFNT_VERSION_OTTO = 0x4f54544f; // 'OTTO'
const TTC_TAG = 0x74746366; // 'ttcf'

const NAME_ID_FAMILY = 1;
const NAME_ID_TYPOGRAPHIC_FAMILY = 16;

function systemFontDirectories(): string[] {
  const home = os.homedir();
  if (process.platform === "darwin") {
    return [
      path.join(home, "Library", "Fonts"),
      "/Library/Fonts",
      "/System/Library/Fonts",
      "/System/Library/Fonts/Supplemental",
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

/**
 * Extract family names (nameID 16 preferred over nameID 1) from an SFNT
 * `name` table. Decodes Windows/Unicode records as UTF-16BE and Macintosh
 * records as Latin-1; malformed records are skipped.
 */
export function parseNameTableFamilies(nameTable: Uint8Array): string[] {
  if (nameTable.length < 6) return [];
  const view = new DataView(nameTable.buffer, nameTable.byteOffset, nameTable.byteLength);
  const count = view.getUint16(2);
  const stringOffset = view.getUint16(4);
  const typographic = new Set<string>();
  const legacy = new Set<string>();

  for (let i = 0; i < count; i += 1) {
    const recordOffset = 6 + i * 12;
    if (recordOffset + 12 > nameTable.length) break;
    const platformId = view.getUint16(recordOffset);
    const nameId = view.getUint16(recordOffset + 6);
    if (nameId !== NAME_ID_FAMILY && nameId !== NAME_ID_TYPOGRAPHIC_FAMILY) continue;

    const length = view.getUint16(recordOffset + 8);
    const offset = stringOffset + view.getUint16(recordOffset + 10);
    if (length === 0 || offset + length > nameTable.length) continue;

    const bytes = nameTable.subarray(offset, offset + length);
    let value = "";
    if (platformId === 0 || platformId === 3) {
      for (let j = 0; j + 1 < bytes.length; j += 2) {
        value += String.fromCharCode((bytes[j] << 8) | bytes[j + 1]);
      }
    } else if (platformId === 1) {
      for (let j = 0; j < bytes.length; j += 1) {
        value += String.fromCharCode(bytes[j]);
      }
    } else {
      continue;
    }

    const trimmed = value.trim();
    if (!trimmed) continue;
    if (nameId === NAME_ID_TYPOGRAPHIC_FAMILY) typographic.add(trimmed);
    else legacy.add(trimmed);
  }

  // Typographic family ("SF Pro") groups weight-specific legacy families
  // ("SF Pro Text Light"); report both so either spelling matches.
  return [...typographic, ...legacy];
}

async function readBytes(
  handle: fs.FileHandle,
  offset: number,
  length: number,
  fileSize: number,
): Promise<Uint8Array | null> {
  if (length <= 0 || offset < 0 || offset + length > fileSize) return null;
  const buffer = new Uint8Array(length);
  const { bytesRead } = await handle.read(buffer, 0, length, offset);
  if (bytesRead !== length) return null;
  return buffer;
}

async function sfntFamiliesAt(
  handle: fs.FileHandle,
  fontOffset: number,
  fileSize: number,
): Promise<string[]> {
  const header = await readBytes(handle, fontOffset, 12, fileSize);
  if (!header) return [];
  const headerView = new DataView(header.buffer, header.byteOffset, header.byteLength);
  const version = headerView.getUint32(0);
  if (version !== SFNT_VERSION_TRUETYPE && version !== SFNT_VERSION_APPLE && version !== SFNT_VERSION_OTTO) {
    return [];
  }

  const numTables = headerView.getUint16(4);
  const directory = await readBytes(handle, fontOffset + 12, numTables * 16, fileSize);
  if (!directory) return [];
  const directoryView = new DataView(directory.buffer, directory.byteOffset, directory.byteLength);

  for (let i = 0; i < numTables; i += 1) {
    const entryOffset = i * 16;
    const tag = directoryView.getUint32(entryOffset);
    if (tag !== 0x6e616d65) continue; // 'name'
    const tableOffset = directoryView.getUint32(entryOffset + 8);
    const tableLength = Math.min(directoryView.getUint32(entryOffset + 12), MAX_NAME_TABLE_BYTES);
    const table = await readBytes(handle, tableOffset, tableLength, fileSize);
    return table ? parseNameTableFamilies(table) : [];
  }
  return [];
}

/** Family names declared inside a .ttf/.otf/.ttc file, via random-access reads. */
async function readFontFileFamilies(fontPath: string, fileSize: number): Promise<string[]> {
  let handle: fs.FileHandle | null = null;
  try {
    handle = await fs.open(fontPath, "r");
    const tagBytes = await readBytes(handle, 0, 12, fileSize);
    if (!tagBytes) return [];
    const tagView = new DataView(tagBytes.buffer, tagBytes.byteOffset, tagBytes.byteLength);
    const tag = tagView.getUint32(0);

    if (tag === TTC_TAG) {
      const numFonts = Math.min(tagView.getUint32(8), MAX_TTC_FONTS);
      const offsets = await readBytes(handle, 12, numFonts * 4, fileSize);
      if (!offsets) return [];
      const offsetsView = new DataView(offsets.buffer, offsets.byteOffset, offsets.byteLength);
      const families = new Set<string>();
      for (let i = 0; i < numFonts; i += 1) {
        for (const family of await sfntFamiliesAt(handle, offsetsView.getUint32(i * 4), fileSize)) {
          families.add(family);
        }
      }
      return [...families];
    }

    return await sfntFamiliesAt(handle, 0, fileSize);
  } catch {
    return [];
  } finally {
    await handle?.close().catch(() => {});
  }
}

interface FontFileInfo {
  path: string;
  sizeBytes: number;
  /** Normalized family keys declared in the file (empty when unparseable, e.g. woff/woff2). */
  familyKeys: string[];
  /** Filename-stem fallback used when the name table is unavailable. */
  stemKey: string;
}

const fontInfoCache = new Map<string, { mtimeMs: number; sizeBytes: number; info: FontFileInfo }>();

async function getFontFileInfo(fontPath: string): Promise<FontFileInfo | null> {
  let stat;
  try {
    stat = await fs.stat(fontPath);
  } catch {
    return null;
  }
  if (stat.size <= 0 || stat.size > MAX_FILE_BYTES) return null;

  const cached = fontInfoCache.get(fontPath);
  if (cached && cached.mtimeMs === stat.mtimeMs && cached.sizeBytes === stat.size) {
    return cached.info;
  }

  const ext = path.extname(fontPath).toLowerCase();
  const parseable = ext === ".ttf" || ext === ".otf" || ext === ".ttc";
  const families = parseable ? await readFontFileFamilies(fontPath, stat.size) : [];
  const info: FontFileInfo = {
    path: fontPath,
    sizeBytes: stat.size,
    familyKeys: [...new Set(families.map(familyKey).filter(Boolean))],
    stemKey: familyKey(path.basename(fontPath, path.extname(fontPath))),
  };
  fontInfoCache.set(fontPath, { mtimeMs: stat.mtimeMs, sizeBytes: stat.size, info });
  return info;
}

function matchScore(info: FontFileInfo, targetKey: string): number {
  // 3: exact declared family; 2: declared family extends the request
  // ("SF Pro" → "SF Pro Text"); 1: filename fallback for unparseable files.
  if (info.familyKeys.some((key) => key === targetKey)) return 3;
  if (info.familyKeys.some((key) => key.startsWith(targetKey))) return 2;
  if (info.familyKeys.length === 0 && (info.stemKey === targetKey || info.stemKey.startsWith(targetKey))) {
    return 1;
  }
  return 0;
}

export async function loadSystemFontFiles(families: string[]): Promise<{
  files: Array<{ family: string; data: Uint8Array }>;
}> {
  const wanted = families
    .map((family) => family.trim())
    .filter(Boolean)
    .map((family) => ({ family, key: familyKey(family) }))
    .filter((entry) => entry.key.length > 0);
  if (wanted.length === 0) return { files: [] };

  const fontPaths: string[] = [];
  for (const dir of systemFontDirectories()) {
    await collectFontFiles(dir, 0, fontPaths);
  }

  const used = new Set<string>();
  const files: Array<{ family: string; data: Uint8Array }> = [];

  for (const target of wanted) {
    const candidates: Array<{ info: FontFileInfo; score: number }> = [];
    for (const fontPath of fontPaths) {
      if (used.has(fontPath)) continue;
      const info = await getFontFileInfo(fontPath);
      if (!info) continue;
      const score = matchScore(info, target.key);
      if (score > 0) candidates.push({ info, score });
    }

    candidates.sort((left, right) =>
      right.score - left.score || left.info.sizeBytes - right.info.sizeBytes,
    );

    for (const candidate of candidates.slice(0, MAX_FILES_PER_FAMILY)) {
      try {
        const buffer = await fs.readFile(candidate.info.path);
        files.push({ family: target.family, data: new Uint8Array(buffer) });
        used.add(candidate.info.path);
      } catch {
        // Skip unreadable font files.
      }
    }
  }

  return { files };
}
