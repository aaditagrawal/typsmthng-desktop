import { describe, expect, it } from "vitest";
import {
	electrobunArchName,
	electrobunFetchErrorLooksLikeMissingManifest,
	electrobunOsName,
	electrobunPlatformPrefix,
	interpretUpdateManifest,
	isQuietManifestHttpStatus,
	updateManifestFileName,
	updateManifestUrl,
} from "./update-check";

describe("Electrobun update manifest naming", () => {
	it("matches the URL Electrobun 1.15.1 fetches for Linux x64 stable", () => {
		expect(electrobunOsName("linux")).toBe("linux");
		expect(electrobunArchName("x64")).toBe("x64");
		expect(updateManifestFileName("stable", "linux", "x64")).toBe(
			"stable-linux-x64-update.json",
		);
		expect(
			updateManifestUrl(
				"https://github.com/aaditagrawal/typsmthng-desktop/releases/latest/download",
				"stable",
				"linux",
				"x64",
			),
		).toBe(
			"https://github.com/aaditagrawal/typsmthng-desktop/releases/latest/download/stable-linux-x64-update.json",
		);
	});

	it("covers every channel/target this repo ships", () => {
		const channels = ["stable", "canary"] as const;
		const targets = [
			["macos", "arm64"],
			["macos", "x64"],
			["linux", "x64"],
			["win", "x64"],
		] as const;
		const names = channels.flatMap((channel) =>
			targets.map(([os, arch]) => updateManifestFileName(channel, os, arch)),
		);
		expect(names).toEqual([
			"stable-macos-arm64-update.json",
			"stable-macos-x64-update.json",
			"stable-linux-x64-update.json",
			"stable-win-x64-update.json",
			"canary-macos-arm64-update.json",
			"canary-macos-x64-update.json",
			"canary-linux-x64-update.json",
			"canary-win-x64-update.json",
		]);
		expect(electrobunPlatformPrefix("stable", "linux", "x64")).toBe("stable-linux-x64");
	});

	it("maps Node platform/arch to Electrobun names", () => {
		expect(electrobunOsName("darwin")).toBe("macos");
		expect(electrobunOsName("win32")).toBe("win");
		expect(electrobunArchName("arm64")).toBe("arm64");
		expect(electrobunArchName("aarch64")).toBe("arm64");
	});
});

describe("interpretUpdateManifest", () => {
	const url =
		"https://github.com/aaditagrawal/typsmthng-desktop/releases/latest/download/stable-linux-x64-update.json";

	it("treats 404/410 as already-on-latest (no error banner)", () => {
		expect(isQuietManifestHttpStatus(404)).toBe(true);
		expect(isQuietManifestHttpStatus(410)).toBe(true);
		expect(
			interpretUpdateManifest({
				status: 404,
				bodyText: "Not Found",
				localHash: "abc",
				url,
			}),
		).toEqual({ kind: "up-to-date", reason: "not-found" });
		expect(
			interpretUpdateManifest({
				status: 410,
				bodyText: "",
				localHash: "abc",
				url,
			}),
		).toEqual({ kind: "up-to-date", reason: "not-found" });
	});

	it("treats matching hash as up-to-date", () => {
		expect(
			interpretUpdateManifest({
				status: 200,
				bodyText: JSON.stringify({ version: "0.1.3", hash: "same", platform: "linux", arch: "x64" }),
				localHash: "same",
				url,
			}),
		).toEqual({ kind: "up-to-date", reason: "same-hash" });
	});

	it("does not hide a newer hash", () => {
		expect(
			interpretUpdateManifest({
				status: 200,
				bodyText: JSON.stringify({ version: "0.1.4", hash: "newer" }),
				localHash: "older",
				url,
			}),
		).toEqual({ kind: "available", version: "0.1.4", hash: "newer" });
	});

	it("reports HTTP failures other than missing-manifest as errors", () => {
		expect(isQuietManifestHttpStatus(500)).toBe(false);
		const decision = interpretUpdateManifest({
			status: 500,
			bodyText: "oops",
			localHash: "abc",
			url,
		});
		expect(decision.kind).toBe("error");
		if (decision.kind === "error") {
			expect(decision.error).toContain("HTTP 500");
		}
	});

	it("reports invalid JSON / missing hash as errors (do not treat as up-to-date)", () => {
		expect(
			interpretUpdateManifest({
				status: 200,
				bodyText: "<!DOCTYPE html>",
				localHash: "abc",
				url,
			}).kind,
		).toBe("error");
		expect(
			interpretUpdateManifest({
				status: 200,
				bodyText: JSON.stringify({ version: "0.1.3" }),
				localHash: "abc",
				url,
			}).kind,
		).toBe("error");
	});
});

describe("electrobunFetchErrorLooksLikeMissingManifest", () => {
	it("matches the Electrobun 1.15.1 404 error string", () => {
		expect(
			electrobunFetchErrorLooksLikeMissingManifest(
				"Failed to fetch update info from https://github.com/aaditagrawal/typsmthng-desktop/releases/latest/download/stable-linux-x64-update.json?x7k2",
			),
		).toBe(true);
	});

	it("does not swallow a download/apply failure when an update exists", () => {
		expect(electrobunFetchErrorLooksLikeMissingManifest("Failed to download latest version")).toBe(
			false,
		);
		expect(electrobunFetchErrorLooksLikeMissingManifest("zig-zstd failed with exit code 1")).toBe(
			false,
		);
	});
});
