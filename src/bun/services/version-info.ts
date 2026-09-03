import { existsSync, mkdirSync, readFileSync, writeFileSync } from "node:fs";
import { homedir } from "node:os";
import path from "node:path";

/** Keep in sync with electrobun.config.ts / package.json. `scripts/update-version.ts` rewrites APP_VERSION. */
export const APP_IDENTIFIER = "dev.typsmthng.desktop";
export const APP_NAME = "typsmthng";
export const APP_VERSION = "0.1.3";
export const APP_CHANNEL = "stable";
export const APP_RELEASE_BASE_URL =
	"https://github.com/aaditagrawal/typsmthng-desktop/releases/latest/download";

export type VersionInfo = {
	version: string;
	hash: string;
	channel: string;
	baseUrl: string;
	name: string;
	identifier: string;
};

export type VersionInfoResolveOptions = {
	cwd?: string;
	execPath?: string;
	exists?: (candidate: string) => boolean;
	readFile?: (candidate: string) => string;
	env?: NodeJS.ProcessEnv;
	home?: string;
	platform?: NodeJS.Platform;
};

export function fallbackVersionInfo(
	channel: string = process.env.ELECTROBUN_ENV || APP_CHANNEL,
): VersionInfo {
	const normalized = channel || APP_CHANNEL;
	return {
		version: APP_VERSION,
		hash: "unknown",
		channel: normalized,
		baseUrl: APP_RELEASE_BASE_URL,
		name: normalized === "stable" || normalized === "dev" ? APP_NAME : `${APP_NAME}-${normalized}`,
		identifier: APP_IDENTIFIER,
	};
}

/** Path Electrobun 1.15.1 getVersionInfo reads: join("..", "Resources", "version.json") from cwd. */
export function electrobunCwdRelativeVersionJson(cwd: string): string {
	return path.resolve(cwd, "..", "Resources", "version.json");
}

export function versionJsonCandidates(opts: VersionInfoResolveOptions = {}): string[] {
	const cwd = opts.cwd ?? process.cwd();
	const execPath = opts.execPath ?? process.execPath;
	const execDir = path.dirname(execPath);
	const appRootFromExec = path.join(execDir, "..");

	const candidates = [
		// Electrobun's cwd-relative read when cwd is bin/
		electrobunCwdRelativeVersionJson(cwd),
		// Real bundle location from the bun/launcher binary
		path.join(appRootFromExec, "Resources", "version.json"),
		// If cwd was already the app root
		path.join(cwd, "Resources", "version.json"),
		path.join(execDir, "Resources", "version.json"),
	];

	const seen = new Set<string>();
	const unique: string[] = [];
	for (const candidate of candidates) {
		const resolved = path.resolve(candidate);
		if (seen.has(resolved)) continue;
		seen.add(resolved);
		unique.push(resolved);
	}
	return unique;
}

function normalizeVersionInfo(parsed: Partial<VersionInfo>, channelHint?: string): VersionInfo {
	const fallback = fallbackVersionInfo(parsed.channel || channelHint);
	return {
		version: typeof parsed.version === "string" && parsed.version ? parsed.version : fallback.version,
		hash: typeof parsed.hash === "string" && parsed.hash ? parsed.hash : fallback.hash,
		channel: typeof parsed.channel === "string" && parsed.channel ? parsed.channel : fallback.channel,
		baseUrl: typeof parsed.baseUrl === "string" && parsed.baseUrl ? parsed.baseUrl : fallback.baseUrl,
		name: typeof parsed.name === "string" && parsed.name ? parsed.name : fallback.name,
		identifier:
			typeof parsed.identifier === "string" && parsed.identifier
				? parsed.identifier
				: fallback.identifier,
	};
}

export function readVersionInfo(opts: VersionInfoResolveOptions = {}): VersionInfo {
	const exists = opts.exists ?? existsSync;
	const readFile = opts.readFile ?? ((candidate: string) => readFileSync(candidate, "utf-8"));
	const channelHint = opts.env?.ELECTROBUN_ENV || APP_CHANNEL;

	for (const candidate of versionJsonCandidates(opts)) {
		try {
			if (!exists(candidate)) continue;
			const parsed = JSON.parse(readFile(candidate)) as Partial<VersionInfo>;
			if (!parsed || typeof parsed !== "object") continue;
			return normalizeVersionInfo(parsed, channelHint);
		} catch {
			// Missing, unreadable, or malformed — try the next absolute path.
		}
	}

	return fallbackVersionInfo(channelHint);
}

export function getAppDataDir(opts: VersionInfoResolveOptions = {}): string {
	const platform = opts.platform ?? process.platform;
	const home = opts.home ?? homedir();
	const env = opts.env ?? process.env;

	if (platform === "darwin") {
		return path.join(home, "Library", "Application Support");
	}
	if (platform === "win32") {
		return env.LOCALAPPDATA || path.join(home, "AppData", "Local");
	}
	return env.XDG_DATA_HOME || path.join(home, ".local", "share");
}

/**
 * Same layout Electrobun Utils.paths.userData uses (appData/identifier/channel),
 * but never throws if version.json is missing or cwd is wrong.
 */
export function getUserDataDir(opts: VersionInfoResolveOptions = {}): string {
	const info = readVersionInfo(opts);
	return path.join(getAppDataDir(opts), info.identifier, info.channel);
}

export function writeVersionJsonFile(
	dest: string,
	overrides: Partial<VersionInfo> = {},
): VersionInfo {
	const info = normalizeVersionInfo({ ...fallbackVersionInfo(overrides.channel), ...overrides });
	mkdirSync(path.dirname(dest), { recursive: true });
	writeFileSync(dest, `${JSON.stringify(info, null, 2)}\n`, "utf-8");
	return info;
}
