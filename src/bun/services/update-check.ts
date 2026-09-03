/**
 * Electrobun 1.15.1 fetches:
 *   `${baseUrl}/${channel}-${os}-${arch}-update.json`
 * e.g. stable-linux-x64-update.json
 *
 * A missing asset (GitHub 302 → 404) must not surface as "Update failed"
 * on a fresh install of the latest release. A real newer hash still shows.
 */

export type UpdateManifestDecision =
	| { kind: "up-to-date"; reason: "not-found" | "same-hash" }
	| { kind: "available"; version: string | null; hash: string }
	| { kind: "error"; error: string };

export type UpdateManifestFile = {
	version?: unknown;
	hash?: unknown;
	platform?: unknown;
	arch?: unknown;
};

export function electrobunOsName(platform: NodeJS.Platform = process.platform): string {
	if (platform === "darwin") return "macos";
	if (platform === "win32") return "win";
	return "linux";
}

export function electrobunArchName(arch: string = process.arch): string {
	if (arch === "arm64" || arch === "aarch64") return "arm64";
	return "x64";
}

export function electrobunPlatformPrefix(channel: string, os: string, arch: string): string {
	return `${channel}-${os}-${arch}`;
}

/** Filename Electrobun Updater.checkForUpdate requests from release.baseUrl. */
export function updateManifestFileName(channel: string, os: string, arch: string): string {
	return `${electrobunPlatformPrefix(channel, os, arch)}-update.json`;
}

export function updateManifestUrl(baseUrl: string, channel: string, os: string, arch: string): string {
	return `${baseUrl.replace(/\/+$/, "")}/${updateManifestFileName(channel, os, arch)}`;
}

export function isQuietManifestHttpStatus(status: number): boolean {
	return status === 404 || status === 410;
}

export function interpretUpdateManifest(opts: {
	status: number;
	bodyText: string;
	localHash: string;
	url: string;
}): UpdateManifestDecision {
	const { status, bodyText, localHash, url } = opts;

	if (isQuietManifestHttpStatus(status)) {
		return { kind: "up-to-date", reason: "not-found" };
	}

	if (status < 200 || status >= 300) {
		return {
			kind: "error",
			error: `Failed to fetch update info (HTTP ${status}) from ${url}`,
		};
	}

	let parsed: UpdateManifestFile;
	try {
		parsed = JSON.parse(bodyText) as UpdateManifestFile;
	} catch {
		return { kind: "error", error: `Invalid update.json: failed to parse JSON from ${url}` };
	}

	const hash = typeof parsed.hash === "string" ? parsed.hash : "";
	if (!hash) {
		return { kind: "error", error: `Invalid update.json: missing hash from ${url}` };
	}

	if (hash === localHash) {
		return { kind: "up-to-date", reason: "same-hash" };
	}

	const version = typeof parsed.version === "string" && parsed.version ? parsed.version : null;
	return { kind: "available", version, hash };
}

export function electrobunFetchErrorLooksLikeMissingManifest(error: string): boolean {
	const text = error.toLowerCase();
	return (
		text.includes("failed to fetch update info") &&
		(text.includes("404") || text.includes("-update.json"))
	);
}
