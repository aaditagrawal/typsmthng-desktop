import fs from "node:fs/promises";
import path from "node:path";

import type { TextSearchResult } from "../../shared/rpc";
import { isKnownTextPath } from "../../mainview/lib/file-classification";
import { runProcess } from "./process-runner";
import { VaultIndexService } from "./vault-index";

const RG_TIMEOUT_MS = 8_000;
const FALLBACK_SCAN_FILE_LIMIT = 400;
const FALLBACK_SCAN_MAX_BYTES = 256 * 1024;

function normalizePreview(value: string): string {
  return value.replace(/\s+/g, " ").trim();
}

export class FullTextSearchService {
  constructor(private readonly indexService: VaultIndexService) {}

  async search(
    rootPath: string,
    query: string,
    limit: number,
    includeHidden: boolean,
  ): Promise<{ results: TextSearchResult[]; truncated: boolean }> {
    const trimmed = query.trim();
    if (!trimmed) {
      return { results: [], truncated: false };
    }

    const ripgrepResult = await this.searchWithRipgrep(rootPath, trimmed, limit, includeHidden);
    if (ripgrepResult) return ripgrepResult;
    return this.searchFallback(rootPath, trimmed, limit, includeHidden);
  }

  private async searchWithRipgrep(
    rootPath: string,
    query: string,
    limit: number,
    includeHidden: boolean,
  ): Promise<{ results: TextSearchResult[]; truncated: boolean } | null> {
    const args = [
      "--json",
      "--line-number",
      "--column",
      "--smart-case",
      "--max-count",
      String(limit),
    ];
    if (includeHidden) {
      args.push("--hidden");
    }
    args.push(query, rootPath);

    const result = await runProcess("rg", args, {
      cwd: rootPath,
      allowNonZeroExit: true,
      timeoutMs: RG_TIMEOUT_MS,
      maxBufferBytes: 6 * 1024 * 1024,
      outputMode: "truncate",
    }).catch(() => null);

    if (!result) return null;
    if (result.code !== 0 && result.code !== 1) return null;

    const matches: TextSearchResult[] = [];
    let truncated = Boolean(result.stdoutTruncated);

    for (const line of result.stdout.split("\n")) {
      if (!line.trim()) continue;
      try {
        const parsed = JSON.parse(line) as {
          type?: string;
          data?: {
            path?: { text?: string };
            lines?: { text?: string };
            line_number?: number;
            submatches?: Array<{ start?: number }>;
          };
        };

        if (parsed.type !== "match" || !parsed.data?.path?.text) continue;
        const preview = normalizePreview(parsed.data.lines?.text ?? "");
        matches.push({
          path: path.relative(rootPath, parsed.data.path.text).split(path.sep).join("/"),
          line: parsed.data.line_number ?? 1,
          column: (parsed.data.submatches?.[0]?.start ?? 0) + 1,
          preview,
        });
        if (matches.length >= limit) {
          truncated = truncated || true;
          break;
        }
      } catch {
        // Ignore malformed lines.
      }
    }

    return { results: matches, truncated };
  }

  private async searchFallback(
    rootPath: string,
    query: string,
    limit: number,
    includeHidden: boolean,
  ): Promise<{ results: TextSearchResult[]; truncated: boolean }> {
    const { results } = await this.indexService.search(
      rootPath,
      "",
      FALLBACK_SCAN_FILE_LIMIT,
      includeHidden,
    );

    const matches: TextSearchResult[] = [];
    let truncated = false;

    for (const entry of results) {
      if (entry.kind !== "file" || !isKnownTextPath(entry.path)) continue;
      try {
        const absolutePath = path.join(rootPath, entry.path);
        const stat = await fs.stat(absolutePath);
        if (stat.size > FALLBACK_SCAN_MAX_BYTES) continue;

        const content = await fs.readFile(absolutePath, "utf8");
        const lines = content.split(/\r?\n/);
        const lowerQuery = query.toLowerCase();
        for (let index = 0; index < lines.length; index += 1) {
          const line = lines[index];
          const matchIndex = line.toLowerCase().indexOf(lowerQuery);
          if (matchIndex === -1) continue;

          matches.push({
            path: entry.path,
            line: index + 1,
            column: matchIndex + 1,
            preview: normalizePreview(line),
          });

          if (matches.length >= limit) {
            truncated = true;
            return { results: matches, truncated };
          }
        }
      } catch {
        // Ignore unreadable files.
      }
    }

    return { results: matches, truncated };
  }
}
