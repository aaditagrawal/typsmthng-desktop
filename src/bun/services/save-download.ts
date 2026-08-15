import fs from "node:fs/promises";
import path from "node:path";
import { Utils } from "electrobun/bun";

function sanitizeDownloadFilename(input: string): string {
  const base = path.basename(input.replace(/\\/g, "/")).trim();
  const cleaned = base.replace(/[<>:"|?*\u0000-\u001f]/g, "_").replace(/^\.+/, "");
  return cleaned || "download";
}

export async function saveDownloadFile(
  filename: string,
  data: Uint8Array,
): Promise<{ ok: true; path: string }> {
  const safeName = sanitizeDownloadFilename(filename);
  const downloadsDir = Utils.paths.downloads;
  await fs.mkdir(downloadsDir, { recursive: true });

  const ext = path.extname(safeName);
  const stem = ext ? safeName.slice(0, -ext.length) : safeName;
  let target = path.join(downloadsDir, safeName);
  let n = 2;
  while (true) {
    try {
      await fs.writeFile(target, data, { flag: "wx" });
      return { ok: true, path: target };
    } catch (error) {
      const code = error instanceof Error && "code" in error
        ? String((error as { code?: unknown }).code)
        : "";
      if (code !== "EEXIST") throw error;
      target = path.join(downloadsDir, `${stem}-${n}${ext}`);
      n += 1;
    }
  }
}
