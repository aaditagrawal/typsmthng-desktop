#!/usr/bin/env bun
/**
 * Headless check of a packaged Linux tree.
 *
 * Verifies the layout Electrobun 1.15.1 actually reads from cwd `$APP/bin`:
 *   join("..", "Resources", "version.json")
 * and that our resolver / userData path still work if that file is absent.
 *
 * Usage: bun scripts/check-packaged-linux-layout.ts <app-dir>
 */
import { existsSync, mkdtempSync, readFileSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";

import { AppStateService } from "../src/bun/services/app-state.ts";
import {
	electrobunCwdRelativeVersionJson,
	getUserDataDir,
	readVersionInfo,
} from "../src/bun/services/version-info.ts";

const appDir = process.argv[2];
if (!appDir) {
	console.error("Usage: bun scripts/check-packaged-linux-layout.ts <app-dir>");
	process.exit(1);
}

const binDir = join(appDir, "bin");
const expected = join(appDir, "Resources", "version.json");
const electrobunPath = electrobunCwdRelativeVersionJson(binDir);

console.log(`app-dir: ${appDir}`);
console.log(`bin cwd: ${binDir}`);
console.log(`Electrobun path from bin: ${electrobunPath}`);
console.log(`expected: ${expected}`);

if (electrobunPath !== expected) {
	console.error("FAIL: cwd-relative Electrobun path does not resolve to Resources/version.json");
	process.exit(1);
}

const present = existsSync(expected);
console.log(`version.json present: ${present}`);
if (present) {
	const raw = readFileSync(expected, "utf-8");
	JSON.parse(raw);
	console.log(`version.json: ${raw.trim()}`);
}

const info = readVersionInfo({
	cwd: binDir,
	execPath: join(binDir, "bun"),
});
console.log(`readVersionInfo: ${JSON.stringify(info)}`);

const prevCwd = process.cwd();
const tmpHome = mkdtempSync(join(tmpdir(), "typsmthng-userdata-"));
try {
	process.chdir(binDir);
	process.env.HOME = tmpHome;
	delete process.env.XDG_DATA_HOME;

	const userData = getUserDataDir({
		cwd: process.cwd(),
		execPath: join(binDir, "bun"),
		home: tmpHome,
		platform: "linux",
		env: {},
	});
	console.log(`userData: ${userData}`);

	const state = new AppStateService();
	const loaded = await state.load();
	console.log(`getStoredWindowState/load ok: width=${loaded.windowState?.width}`);
} finally {
	process.chdir(prevCwd);
	rmSync(tmpHome, { recursive: true, force: true });
}

if (present && !existsSync(electrobunPath)) {
	console.error("FAIL: version.json exists but Electrobun path from bin cwd missed it");
	process.exit(1);
}

console.log("OK: packaged Linux layout check passed");
