#!/usr/bin/env bun
/**
 * Write an Electrobun updater manifest from a packaged version.json.
 * Used when `electrobun build` skipped artifacts/ (dev wrap, or a host-arch mismatch).
 *
 * Usage: bun scripts/generate-update-json.ts <dest-update.json> <version.json> [platform] [arch]
 */
import { mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { dirname } from "node:path";

const dest = process.argv[2];
const source = process.argv[3];
const platform = process.argv[4];
const arch = process.argv[5];

if (!dest || !source) {
	console.error(
		"Usage: bun scripts/generate-update-json.ts <dest-update.json> <version.json> [platform] [arch]",
	);
	process.exit(1);
}

const parsed = JSON.parse(readFileSync(source, "utf-8")) as {
	version?: unknown;
	hash?: unknown;
};

if (typeof parsed.version !== "string" || !parsed.version) {
	console.error("version.json missing version:", source);
	process.exit(1);
}
if (typeof parsed.hash !== "string" || !parsed.hash || parsed.hash === "unknown") {
	console.error("version.json missing usable hash:", source);
	process.exit(1);
}

const out: Record<string, string> = {
	version: parsed.version,
	hash: parsed.hash,
};
if (platform) out.platform = platform;
if (arch) out.arch = arch;

mkdirSync(dirname(dest), { recursive: true });
writeFileSync(dest, `${JSON.stringify(out)}\n`, "utf-8");
console.log(`generated update.json version=${out.version} hash=${out.hash} -> ${dest}`);
