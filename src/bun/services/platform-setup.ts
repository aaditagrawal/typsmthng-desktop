import fs from "node:fs/promises";
import { existsSync, readlinkSync } from "node:fs";
import { execSync } from "node:child_process";
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

function findMacOSAppBundle(): string | null {
	// Walk up from import.meta.dir to find the .app bundle
	let dir = import.meta.dir;
	for (let i = 0; i < 10; i++) {
		if (dir.endsWith(".app")) return dir;
		// Check if parent contains .app
		const parent = path.dirname(dir);
		if (parent === dir) break;
		dir = parent;
	}

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

	const desktopContent = [
		"[Desktop Entry]",
		`Name=${APP_NAME}`,
		`Exec=${execPath} %f`,
		`Icon=${APP_NAME}`,
		"Type=Application",
		"Categories=Office;TextEditor;",
		"Comment=Folder-backed Typst editor",
		"MimeType=text/x-typst;",
	].join("\n");

	const desktopPath = path.join(applicationsDir, `${APP_NAME}.desktop`);
	const existing = await safeRead(desktopPath);
	if (existing !== desktopContent) {
		await fs.writeFile(desktopPath, desktopContent, "utf8");
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
			execSync("update-mime-database " + path.join(HOME, ".local", "share", "mime"), {
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

async function setupWindows(): Promise<void> {
	// The NSIS installer handles this for fresh installs, but users who
	// installed before these features were added need them applied at runtime.
	const exePath = process.execPath;
	const instDir = path.dirname(exePath);

	// 1. File association: .typ → typsmthng
	try {
		// Check if already registered
		const result = safeRegQuery("HKCU\\Software\\Classes\\.typ");
		if (!result?.includes("typsmthng.typ")) {
			execSync(`reg add "HKCU\\Software\\Classes\\.typ" /ve /d "typsmthng.typ" /f`, { stdio: "ignore" });
			execSync(`reg add "HKCU\\Software\\Classes\\typsmthng.typ" /ve /d "Typst Document" /f`, { stdio: "ignore" });
			execSync(`reg add "HKCU\\Software\\Classes\\typsmthng.typ\\shell\\open\\command" /ve /d "\\"${exePath}\\" \\"%1\\"" /f`, { stdio: "ignore" });
			console.log("Registered .typ file association");
		}
	} catch (error) {
		console.warn("Could not register .typ file association:", error);
	}

	// 2. Add to user PATH if not already present
	try {
		const pathResult = safeRegQuery("HKCU\\Environment", "Path");
		if (pathResult && !pathResult.includes(instDir)) {
			// Extract current PATH value from reg query output
			const match = pathResult.match(/Path\s+REG_(?:EXPAND_)?SZ\s+(.+)/i);
			const currentPath = match?.[1]?.trim() ?? "";
			const newPath = currentPath ? `${currentPath};${instDir}` : instDir;
			execSync(`reg add "HKCU\\Environment" /v Path /t REG_EXPAND_SZ /d "${newPath}" /f`, { stdio: "ignore" });
			// Broadcast environment change
			execSync('rundll32 user32.dll,UpdatePerUserSystemParameters', { stdio: "ignore", timeout: 3000 });
			console.log("Added to user PATH");
		}
	} catch (error) {
		console.warn("Could not update PATH:", error);
	}
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
