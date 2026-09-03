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
});

describe("collect-update-artifacts.sh", () => {
	let dir = "";

	afterEach(() => {
		if (dir) rmSync(dir, { recursive: true, force: true });
	});

	it("copies the exact filename Electrobun fetches", () => {
		dir = mkdtempSync(path.join(tmpdir(), "typsmthng-artifacts-"));
		const repo = path.join(dir, "repo");
		mkdirSync(path.join(repo, "scripts"), { recursive: true });
		mkdirSync(path.join(repo, "artifacts"), { recursive: true });
		mkdirSync(path.join(repo, "build", "release"), { recursive: true });
		execSync(`cp "${path.join(ROOT, "scripts/collect-update-artifacts.sh")}" "${path.join(repo, "scripts")}"`);
		execSync(`cp "${path.join(ROOT, "scripts/validate-update-json.ts")}" "${path.join(repo, "scripts")}"`);
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
});

describe("optional tray stub", () => {
	it.skipIf(process.platform !== "linux")("exports the AppIndicator symbols Electrobun links", () => {
		const dir = mkdtempSync(path.join(tmpdir(), "typsmthng-tray-stub-"));
		try {
			const so = path.join(dir, "libayatana-appindicator3.so.1");
			execFileSync("gcc", [
				"-shared",
				"-fPIC",
				"-Wl,-soname,libayatana-appindicator3.so.1",
				"-o",
				so,
				path.join(ROOT, "native/linux/stub-ayatana-appindicator3.c"),
			]);
			const nm = execFileSync("nm", ["-D", "--defined-only", so], { encoding: "utf-8" });
			for (const symbol of [
				"app_indicator_new",
				"app_indicator_set_icon_full",
				"app_indicator_set_menu",
				"app_indicator_set_status",
				"app_indicator_set_title",
			]) {
				expect(nm).toContain(symbol);
			}
		} finally {
			rmSync(dir, { recursive: true, force: true });
		}
	});
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
