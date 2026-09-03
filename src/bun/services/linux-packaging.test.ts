import { afterEach, describe, expect, it } from "vitest";
import { execFileSync, execSync } from "node:child_process";
import { existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "../../..");

describe("Linux packaging scripts", () => {
	it("shell scripts parse", () => {
		for (const relative of [
			"scripts/linux-launcher-wrapper.sh",
			"scripts/collect-update-artifacts.sh",
			"scripts/linux-app-dir.sh",
			"scripts/package-linux-appimage.sh",
			"scripts/package-linux-deb-rpm.sh",
			"scripts/package-macos.sh",
			"scripts/package-windows.sh",
		]) {
			execFileSync("bash", ["-n", path.join(ROOT, relative)]);
		}
	});

	it("declares WebKit and Ayatana AppIndicator runtime Depends", () => {
		const src = readFileSync(path.join(ROOT, "scripts/package-linux-deb-rpm.sh"), "utf-8");
		expect(src).toContain("libwebkit2gtk-4.1-0");
		expect(src).toContain("libjavascriptcoregtk-4.1-0");
		expect(src).toContain("libayatana-appindicator3-1");
		expect(src).toContain("webkit2gtk4.1");
		expect(src).toContain("libayatana-appindicator-gtk3");
	});

	it("does not force GDK_BACKEND=x11 in typsmthng Linux wrappers", () => {
		const files = [
			"scripts/linux-launcher-wrapper.sh",
			"scripts/linux-app-dir.sh",
			"scripts/package-linux-appimage.sh",
			"scripts/package-linux-deb-rpm.sh",
			"scripts/linux-app-dir.sh",
		];
		for (const relative of files) {
			const src = readFileSync(path.join(ROOT, relative), "utf-8");
			expect(src, relative).not.toMatch(/^\s*(export\s+)?GDK_BACKEND=x11/m);
			expect(src, relative).not.toMatch(/setenv\(\s*"GDK_BACKEND"/);
		}
	});

	it("release workflow uploads Electrobun updater artifacts from build/release", () => {
		const release = readFileSync(path.join(ROOT, ".github/workflows/release.yml"), "utf-8");
		expect(release).toContain("path: build/release/*");
		expect(release).toContain("runner: macos-15-intel");
		// Channel is injected by Actions, not the shell: pwsh ignores $ELECTROBUN_ENV,
		// and Git-bash tar treats D:\ as a remote when electrobun extracts its CLI.
		expect(release).toContain("bunx electrobun build --env=${{ env.ELECTROBUN_ENV }}");
		expect(release).toContain("runner.os == 'Windows' && 'pwsh' || 'bash'");
		expect(release).not.toMatch(/shell:\s*bash\s*\n\s*run:\s*bunx electrobun build/);
		expect(readFileSync(path.join(ROOT, "scripts/package-linux-appimage.sh"), "utf-8")).toContain(
			"collect-update-artifacts.sh",
		);
		expect(readFileSync(path.join(ROOT, "scripts/package-macos.sh"), "utf-8")).toContain(
			"collect-update-artifacts.sh",
		);
		expect(readFileSync(path.join(ROOT, "scripts/package-windows.sh"), "utf-8")).toContain(
			"collect-update-artifacts.sh",
		);
	});

	it("does not package a Windows dev tree as the requested channel", () => {
		const src = readFileSync(path.join(ROOT, "scripts/package-windows.sh"), "utf-8");
		expect(src).toContain('name "${ENV}-win-*"');
		expect(src).not.toContain('name "*-win-*"');
		expect(src).toContain("unpack_zstd_tar");
		expect(src).toContain(".tar.zst");
	});

	it("names macOS installers after Electrobun's host arch", () => {
		const src = readFileSync(path.join(ROOT, "scripts/package-macos.sh"), "utf-8");
		expect(src).toContain("naming artifacts for");
		expect(src).toContain("macos-(arm64|x64)");
	});

	it("can unpack Electrobun zstd tarballs on Windows via zig-zstd.exe", () => {
		const src = readFileSync(path.join(ROOT, "scripts/linux-app-dir.sh"), "utf-8");
		expect(src).toContain("dist-win-x64/zig-zstd.exe");
	});
});

function seedCollectScripts(repo: string) {
	mkdirSync(path.join(repo, "scripts"), { recursive: true });
	for (const name of [
		"collect-update-artifacts.sh",
		"validate-update-json.ts",
		"generate-update-json.ts",
	]) {
		execSync(`cp "${path.join(ROOT, "scripts", name)}" "${path.join(repo, "scripts")}"`);
	}
}

describe("collect-update-artifacts.sh", () => {
	let dir = "";

	afterEach(() => {
		if (dir) rmSync(dir, { recursive: true, force: true });
	});

	it("copies the exact filename Electrobun fetches", () => {
		dir = mkdtempSync(path.join(tmpdir(), "typsmthng-artifacts-"));
		const repo = path.join(dir, "repo");
		seedCollectScripts(repo);
		mkdirSync(path.join(repo, "artifacts"), { recursive: true });
		mkdirSync(path.join(repo, "build", "release"), { recursive: true });
		writeFileSync(
			path.join(repo, "artifacts", "stable-linux-x64-update.json"),
			JSON.stringify({ version: "0.1.3", hash: "testhash", platform: "linux", arch: "x64" }),
		);
		writeFileSync(path.join(repo, "artifacts", "stable-linux-x64-typsmthng.tar.zst"), "fake");

		execFileSync("bash", [path.join(repo, "scripts/collect-update-artifacts.sh"), "stable", "linux", "x64"], {
			cwd: repo,
		});

		expect(existsSync(path.join(repo, "build", "release", "stable-linux-x64-update.json"))).toBe(true);
		expect(existsSync(path.join(repo, "build", "release", "stable-linux-x64-typsmthng.tar.zst"))).toBe(
			true,
		);
	});

	it("maps Electrobun host-arch artifacts when the matrix arch differs", () => {
		dir = mkdtempSync(path.join(tmpdir(), "typsmthng-artifacts-"));
		const repo = path.join(dir, "repo");
		seedCollectScripts(repo);
		mkdirSync(path.join(repo, "artifacts"), { recursive: true });
		mkdirSync(path.join(repo, "build", "release"), { recursive: true });
		writeFileSync(
			path.join(repo, "artifacts", "stable-macos-arm64-update.json"),
			JSON.stringify({ version: "0.1.3", hash: "hosthash", platform: "macos", arch: "arm64" }),
		);
		writeFileSync(path.join(repo, "artifacts", "stable-macos-arm64-typsmthng.app.tar.zst"), "fake");

		const output = execFileSync(
			"bash",
			[path.join(repo, "scripts/collect-update-artifacts.sh"), "stable", "macos", "x64"],
			{ cwd: repo, encoding: "utf-8" },
		);

		expect(output).toContain("using Electrobun host output stable-macos-arm64-update.json");
		expect(existsSync(path.join(repo, "build", "release", "stable-macos-arm64-update.json"))).toBe(
			true,
		);
		expect(existsSync(path.join(repo, "build", "release", "stable-macos-x64-update.json"))).toBe(
			false,
		);
	});

	it("generates update.json from version.json when artifacts/ is missing", () => {
		dir = mkdtempSync(path.join(tmpdir(), "typsmthng-artifacts-"));
		const repo = path.join(dir, "repo");
		seedCollectScripts(repo);
		mkdirSync(path.join(repo, "build", "release"), { recursive: true });
		mkdirSync(path.join(repo, "build", "stable-win-x64", "typsmthng", "Resources"), {
			recursive: true,
		});
		writeFileSync(
			path.join(repo, "build", "stable-win-x64", "typsmthng", "Resources", "version.json"),
			JSON.stringify({
				version: "0.1.3",
				hash: "winhash123",
				channel: "stable",
				name: "typsmthng",
			}),
		);

		execFileSync("bash", [path.join(repo, "scripts/collect-update-artifacts.sh"), "stable", "win", "x64"], {
			cwd: repo,
		});

		const dest = path.join(repo, "build", "release", "stable-win-x64-update.json");
		expect(existsSync(dest)).toBe(true);
		expect(JSON.parse(readFileSync(dest, "utf-8"))).toMatchObject({
			version: "0.1.3",
			hash: "winhash123",
			platform: "win",
			arch: "x64",
		});
	});
});

describe("generate-update-json.ts", () => {
	let dir = "";

	afterEach(() => {
		if (dir) rmSync(dir, { recursive: true, force: true });
	});

	it("writes version and hash from version.json", () => {
		dir = mkdtempSync(path.join(tmpdir(), "typsmthng-gen-update-"));
		const versionJson = path.join(dir, "version.json");
		const dest = path.join(dir, "stable-linux-x64-update.json");
		writeFileSync(versionJson, JSON.stringify({ version: "0.1.3", hash: "abc123" }));
		execFileSync("bun", [
			path.join(ROOT, "scripts/generate-update-json.ts"),
			dest,
			versionJson,
			"linux",
			"x64",
		]);
		expect(JSON.parse(readFileSync(dest, "utf-8"))).toEqual({
			version: "0.1.3",
			hash: "abc123",
			platform: "linux",
			arch: "x64",
		});
	});

	it("rejects fallback hash unknown", () => {
		dir = mkdtempSync(path.join(tmpdir(), "typsmthng-gen-update-"));
		const versionJson = path.join(dir, "version.json");
		const dest = path.join(dir, "update.json");
		writeFileSync(versionJson, JSON.stringify({ version: "0.1.3", hash: "unknown" }));
		expect(() =>
			execFileSync("bun", [path.join(ROOT, "scripts/generate-update-json.ts"), dest, versionJson]),
		).toThrow(/usable hash/);
	});
});

const TRAY_STUB_SYMBOLS = [
	"app_indicator_new",
	"app_indicator_set_icon_full",
	"app_indicator_set_menu",
	"app_indicator_set_status",
	"app_indicator_set_title",
] as const;

describe("optional tray stub", () => {
	it("declares AppIndicator symbols and packaging compiles them as the Electrobun soname", () => {
		const src = readFileSync(path.join(ROOT, "native/linux/stub-ayatana-appindicator3.c"), "utf-8");
		for (const symbol of TRAY_STUB_SYMBOLS) {
			expect(src, symbol).toMatch(new RegExp(`\\b${symbol}\\s*\\(`));
		}

		const packaging = readFileSync(path.join(ROOT, "scripts/linux-app-dir.sh"), "utf-8");
		expect(packaging).toContain("stub-ayatana-appindicator3.c");
		expect(packaging).toContain("compile_linux_tray_stub");
		expect(packaging).toContain("-Wl,-soname,libayatana-appindicator3.so.1");
		expect(packaging).toContain('dest="$APP_DIR/lib/tray-stub"');
		expect(packaging).toContain('"$dest/libayatana-appindicator3.so.1"');
	});

	// gcc's first invocation on a cold GitHub runner can exceed vitest's 5s
	// default (v0.1.3 release: 9919ms). Keep the .so export check, but do not
	// fail quality-checks on a 5s compile budget.
	it.skipIf(process.platform !== "linux")(
		"exports the AppIndicator symbols Electrobun links",
		() => {
			const dir = mkdtempSync(path.join(tmpdir(), "typsmthng-tray-stub-"));
			try {
				const so = path.join(dir, "libayatana-appindicator3.so.1");
				execFileSync(
					"gcc",
					[
						"-shared",
						"-fPIC",
						"-Wl,-soname,libayatana-appindicator3.so.1",
						"-o",
						so,
						path.join(ROOT, "native/linux/stub-ayatana-appindicator3.c"),
					],
					{ timeout: 25_000 },
				);
				let symbols = "";
				try {
					symbols = execFileSync("nm", ["-D", "--defined-only", so], {
						encoding: "utf-8",
						timeout: 5_000,
					});
				} catch {
					symbols = execFileSync("readelf", ["-Ws", so], {
						encoding: "utf-8",
						timeout: 5_000,
					});
				}
				for (const symbol of TRAY_STUB_SYMBOLS) {
					expect(symbols).toContain(symbol);
				}
			} finally {
				rmSync(dir, { recursive: true, force: true });
			}
		},
		30_000,
	);
});

describe("wrap_linux_launcher", () => {
	it("installs a relocatable wrapper next to launcher.real", () => {
		const dir = mkdtempSync(path.join(tmpdir(), "typsmthng-wrap-"));
		try {
			const appDir = path.join(dir, "opt", "typsmthng");
			mkdirSync(path.join(appDir, "bin"), { recursive: true });
			writeFileSync(path.join(appDir, "bin", "launcher"), "#!/bin/sh\necho real-launcher\n");
			execSync(`chmod +x "${path.join(appDir, "bin", "launcher")}"`);

			execFileSync(
				"bash",
				[
					"-c",
					`
set -euo pipefail
ROOT_DIR="${ROOT}"
APP_DIR="${appDir}"
# shellcheck source=linux-app-dir.sh
. "${ROOT}/scripts/linux-app-dir.sh"
wrap_linux_launcher
`,
				],
				{ encoding: "utf-8" },
			);

			const wrapper = readFileSync(path.join(appDir, "bin", "launcher"), "utf-8");
			expect(wrapper).toContain("typsmthng-linux-launcher-wrapper");
			expect(wrapper).not.toMatch(/^\s*(export\s+)?GDK_BACKEND=x11/m);
			expect(existsSync(path.join(appDir, "bin", "launcher.real"))).toBe(true);
		} finally {
			rmSync(dir, { recursive: true, force: true });
		}
	});
});
