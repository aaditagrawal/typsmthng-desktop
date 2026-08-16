import fs from "node:fs/promises";
import { existsSync, readlinkSync } from "node:fs";
import { execSync, spawnSync } from "node:child_process";
import path from "node:path";

const APP_NAME = "typsmthng";
const HOME = process.env.HOME ?? process.env.USERPROFILE ?? "";

/**
 * Runs idempotent platform integration on every launch so existing users
 * who receive an update get CLI, file association, and MIME registration
 * without needing to reinstall.
 */
export async function runPlatformSetup(): Promise<void> {
	if (!HOME) return;

	try {
		if (process.platform === "darwin") {
			await setupMacOS();
		} else if (process.platform === "linux") {
			await setupLinux();
		} else if (process.platform === "win32") {
			await setupWindows();
		}
	} catch (error) {
		console.warn("Platform setup warning:", error);
	}
}

// ── macOS ───────────────────────────────────────────────────────────────

async function setupMacOS(): Promise<void> {
	// Find the .app bundle path — the bun process runs inside
	// typsmthng.app/Contents/Resources/bun/...
	const appBundlePath = findMacOSAppBundle();
	if (!appBundlePath) return;

	const cliTarget = path.join(appBundlePath, "Contents", "MacOS", APP_NAME);
	if (!existsSync(cliTarget)) return;

	// Symlink to /usr/local/bin (user-writable on most macOS installs)
	await ensureSymlink("/usr/local/bin/typsmthng", cliTarget);
}

function walkUpToAppBundle(startDir: string): string | null {
	let dir = startDir;
	for (let i = 0; i < 10; i++) {
		if (dir.endsWith(".app")) return dir;
		const parent = path.dirname(dir);
		if (parent === dir) break;
		dir = parent;
	}
	return null;
}

function findMacOSAppBundle(): string | null {
	// Prefer the real binary location: with ASAR packaging the bun entrypoint
	// is extracted to a temp dir, so import.meta.dir escapes the bundle.
	const fromExecPath = walkUpToAppBundle(path.dirname(process.execPath));
	if (fromExecPath) return fromExecPath;

	const fromModuleDir = walkUpToAppBundle(import.meta.dir);
	if (fromModuleDir) return fromModuleDir;

	// Fallback: common install location
	const defaultPath = `/Applications/${APP_NAME}.app`;
	if (existsSync(defaultPath)) return defaultPath;

	return null;
}

// ── Linux ───────────────────────────────────────────────────────────────

async function setupLinux(): Promise<void> {
	// Find the AppImage or binary path
	const appImagePath = process.env.APPIMAGE;
	const execPath = appImagePath ?? process.execPath;

	// 1. CLI symlink in ~/.local/bin
	const localBin = path.join(HOME, ".local", "bin");
	await fs.mkdir(localBin, { recursive: true });
	await ensureSymlink(path.join(localBin, APP_NAME), execPath);

	// 2. .desktop file
	const applicationsDir = path.join(HOME, ".local", "share", "applications");
	await fs.mkdir(applicationsDir, { recursive: true });

	// Desktop-entry spec: quote the exec path (AppImages can live under
	// paths with spaces) and escape the reserved characters inside quotes.
	const quotedExec = `"${execPath.replace(/[\\"`$]/g, "\\$&")}"`;
	const desktopContent = [
		"[Desktop Entry]",
		`Name=${APP_NAME}`,
		`Exec=${quotedExec} %f`,
		`Icon=${APP_NAME}`,
		"Type=Application",
		"Categories=Office;TextEditor;",
		"Comment=Folder-backed Typst editor",
		"MimeType=text/x-typst;inode/directory;",
	].join("\n");

	const desktopPath = path.join(applicationsDir, `${APP_NAME}.desktop`);
	const existing = await safeRead(desktopPath);
	if (existing !== desktopContent) {
		await fs.writeFile(desktopPath, desktopContent, "utf8");

		// Folder "Open With" entries resolve through mimeinfo.cache, which many
		// environments only rebuild via update-desktop-database.
		try {
			spawnSync("update-desktop-database", [applicationsDir], {
				stdio: "ignore",
				timeout: 5000,
			});
		} catch {}
	}

	// 3. MIME type for .typ files
	const mimeDir = path.join(HOME, ".local", "share", "mime", "packages");
	await fs.mkdir(mimeDir, { recursive: true });

	const mimeContent = [
		'<?xml version="1.0" encoding="UTF-8"?>',
		'<mime-info xmlns="http://www.freedesktop.org/standards/shared-mime-info">',
		'  <mime-type type="text/x-typst">',
		"    <comment>Typst document</comment>",
		'    <glob pattern="*.typ"/>',
		"  </mime-type>",
		"</mime-info>",
	].join("\n");

	const mimePath = path.join(mimeDir, `${APP_NAME}-typst.xml`);
	const existingMime = await safeRead(mimePath);
	if (existingMime !== mimeContent) {
		await fs.writeFile(mimePath, mimeContent, "utf8");

		// Update MIME database
		try {
			spawnSync("update-mime-database", [path.join(HOME, ".local", "share", "mime")], {
				stdio: "ignore",
				timeout: 5000,
			});
		} catch {}
	}

	// 4. Set as default handler for text/x-typst
	try {
		execSync(`xdg-mime default ${APP_NAME}.desktop text/x-typst`, {
			stdio: "ignore",
			timeout: 5000,
		});
	} catch {}

	// 5. Icon — copy to hicolor theme if available from AppImage
	const iconDest = path.join(HOME, ".local", "share", "icons", "hicolor", "256x256", "apps", `${APP_NAME}.png`);
	if (!existsSync(iconDest)) {
		const iconCandidates = [
			// Inside mounted AppImage
			process.env.APPDIR ? path.join(process.env.APPDIR, `${APP_NAME}.png`) : null,
			// Alongside the binary
			path.join(path.dirname(execPath), "Resources", "appIcon.png"),
		].filter(Boolean) as string[];

		for (const candidate of iconCandidates) {
			if (existsSync(candidate)) {
				await fs.mkdir(path.dirname(iconDest), { recursive: true });
				await fs.copyFile(candidate, iconDest);
				break;
			}
		}
	}
}

// ── Windows ─────────────────────────────────────────────────────────────

function readWindowsInstallDir(): string | null {
	const result = safeRegQuery("HKCU\\Software\\typsmthng", "InstallDir");
	const match = result?.match(/InstallDir\s+REG_\w+\s+(.+)/i);
	const value = match?.[1]?.trim();
	return value || null;
}

function resolveWindowsLaunchExe(): string | null {
	const execPath = process.execPath;
	const execDir = path.dirname(execPath);
	const localAppData = process.env.LOCALAPPDATA ?? "";
	const installDir = readWindowsInstallDir();

	const candidates = [
		path.join(execDir, "launcher.exe"),
		path.join(execDir, "bin", "launcher.exe"),
		path.join(path.dirname(execDir), "bin", "launcher.exe"),
		installDir ? path.join(installDir, "bin", "launcher.exe") : "",
		localAppData ? path.join(localAppData, "typsmthng", "bin", "launcher.exe") : "",
	];

	for (const candidate of candidates) {
		if (candidate && existsSync(candidate)) return candidate;
	}

	// Never fall back to bun.exe — that rewrites .typ open/PATH to the wrong binary.
	return null;
}

async function setupWindows(): Promise<void> {
	// The NSIS installer handles this for fresh installs, but users who
	// installed before these features were added need them applied at runtime.
	const exePath = resolveWindowsLaunchExe();
	if (!exePath) {
		console.warn("Windows launcher.exe not found; skipping file association and PATH setup");
		return;
	}
	const binDir = path.dirname(exePath);

	// 1. File association. Claim `.typ` only when nothing else owns it (don't
	// steal the extension back from an editor the user chose), but always
	// refresh our own ProgID's open command to the current launcher path.
	try {
		const typDefault = readRegDefaultValue("HKCU\\Software\\Classes\\.typ");
		if (!typDefault || typDefault === "typsmthng.typ") {
			regAdd(["HKCU\\Software\\Classes\\.typ", "/ve", "/d", "typsmthng.typ", "/f"]);
		}
		regAdd(["HKCU\\Software\\Classes\\typsmthng.typ", "/ve", "/d", "Typst Document", "/f"]);
		const openCommand = windowsTypstOpenCommand(exePath);
		if (
			regAdd(["HKCU\\Software\\Classes\\typsmthng.typ\\shell\\open\\command", "/ve", "/d", openCommand, "/f"])
		) {
			console.log("Registered .typ file association");
		}
	} catch (error) {
		console.warn("Could not register .typ file association:", error);
	}

	// 2. Folder context menu: "Open with typsmthng". Refresh the command on
	// every launch so a reinstall to a different directory doesn't leave a
	// dead exe behind the menu entry.
	try {
		regAdd(["HKCU\\Software\\Classes\\Directory\\shell\\typsmthng", "/ve", "/d", "Open with typsmthng", "/f"]);
		if (
			regAdd(["HKCU\\Software\\Classes\\Directory\\shell\\typsmthng\\command", "/ve", "/d", `"${exePath}" "%V"`, "/f"])
		) {
			console.log("Registered folder context menu entry");
		}
	} catch (error) {
		console.warn("Could not register folder context menu:", error);
	}

	// 3. Ensure the launcher bin dir is on user PATH (idempotent)
	try {
		const pathResult = safeRegQuery("HKCU\\Environment", "Path");
		const match = pathResult?.match(/Path\s+REG_(?:EXPAND_)?SZ\s+(.+)/i);
		const currentPath = match?.[1]?.trim() ?? "";
		const alreadyPresent = currentPath
			.toLowerCase()
			.split(";")
			.some((entry) => entry.trim().toLowerCase() === binDir.toLowerCase());

		if (!alreadyPresent) {
			if (currentPath.length + binDir.length + 1 > 2000) {
				console.warn("PATH is too long, skipping addition");
				return;
			}
			const newPath = currentPath ? `${binDir};${currentPath}` : binDir;
			// shell:false is load-bearing: through cmd.exe, %VAR% inside the
			// user's REG_EXPAND_SZ entries would be expanded and hardcoded.
			if (!regAdd(["HKCU\\Environment", "/v", "Path", "/t", "REG_EXPAND_SZ", "/d", newPath, "/f"])) {
				console.warn("reg add failed; PATH not updated");
				return;
			}
			broadcastEnvironmentChange();
			console.log("Added to user PATH");
		}
	} catch (error) {
		console.warn("Could not update PATH:", error);
	}
}

/** Run `reg add` without a shell; returns true only if reg reported success. */
function regAdd(args: string[]): boolean {
	const result = spawnSync("reg", ["add", ...args], { shell: false, stdio: "ignore" });
	return result.status === 0;
}

function readRegDefaultValue(key: string): string | null {
	const output = safeRegQuery(key);
	// reg query /ve prints: `    (Default)    REG_SZ    value`
	const match = output?.match(/\(Default\)\s+REG_\w+\s+(.+)/i);
	return match?.[1]?.trim() ?? null;
}

/** Broadcast WM_SETTINGCHANGE "Environment" so new Explorer-spawned shells see PATH edits. */
function broadcastEnvironmentChange(): void {
	const script = [
		"$sig = '[DllImport(\"user32.dll\", SetLastError = true, CharSet = CharSet.Auto)]",
		"public static extern IntPtr SendMessageTimeout(IntPtr hWnd, uint Msg, UIntPtr wParam, string lParam, uint fuFlags, uint uTimeout, out UIntPtr lpdwResult);';",
		"$type = Add-Type -MemberDefinition $sig -Name NativeBroadcast -Namespace Win32 -PassThru;",
		"[UIntPtr]$result = [UIntPtr]::Zero;",
		"$type::SendMessageTimeout([IntPtr]0xffff, 0x1A, [UIntPtr]::Zero, 'Environment', 2, 5000, [ref]$result) | Out-Null;",
	].join(" ");
	try {
		spawnSync("powershell", ["-NoProfile", "-NonInteractive", "-Command", script], {
			shell: false,
			stdio: "ignore",
			timeout: 10_000,
		});
	} catch {}
}

function safeRegQuery(key: string, valueName?: string): string | null {
	try {
		const cmd = valueName
			? `reg query "${key}" /v "${valueName}"`
			: `reg query "${key}" /ve`;
		return execSync(cmd, { encoding: "utf8", stdio: ["pipe", "pipe", "ignore"] });
	} catch {
		return null;
	}
}

// ── Helpers ─────────────────────────────────────────────────────────────

async function ensureSymlink(linkPath: string, target: string): Promise<void> {
	try {
		if (existsSync(linkPath)) {
			// Check if it already points to the right place
			try {
				const currentTarget = readlinkSync(linkPath);
				if (currentTarget === target) return;
			} catch {
				// Not a symlink — don't overwrite user files
				return;
			}
			// Symlink exists but points elsewhere — update it
			await fs.unlink(linkPath);
		}
		await fs.symlink(target, linkPath);
		console.log(`Symlinked ${linkPath} → ${target}`);
	} catch (error) {
		// Permission denied is expected for /usr/local/bin on some setups
		console.warn(`Could not create symlink ${linkPath}: ${error}`);
	}
}

async function safeRead(filePath: string): Promise<string | null> {
	try {
		return await fs.readFile(filePath, "utf8");
	} catch {
		return null;
	}
}

/** Registry /d value for opening a .typ file. Must keep `%1` literal (cmd expands it). */
export function windowsTypstOpenCommand(exePath: string): string {
	return `"${exePath}" "%1"`;
}

/** Electrobun locates Resources from process.cwd(). File-association launches often start elsewhere. */
export function resolvePackagedAppRoot(
	execPath: string,
	exists: (candidate: string) => boolean = existsSync,
): string | null {
	const execDir = path.dirname(execPath);
	const appRoot = path.join(execDir, "..");
	if (exists(path.join(appRoot, "Resources", "version.json"))) return appRoot;
	return null;
}

export function ensurePackagedWorkingDirectory(
	execPath: string = process.execPath,
	exists: (candidate: string) => boolean = existsSync,
	chdir: (dir: string) => void = (dir) => process.chdir(dir),
): string | null {
	const appRoot = resolvePackagedAppRoot(execPath, exists);
	if (!appRoot) return null;
	try {
		chdir(appRoot);
		return appRoot;
	} catch {
		return null;
	}
}
