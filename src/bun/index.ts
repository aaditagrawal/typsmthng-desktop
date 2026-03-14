import { ApplicationMenu, BrowserView, BrowserWindow, Updater } from "electrobun/bun";
import { dlopen, FFIType } from "bun:ffi";
import { existsSync, mkdirSync, rmSync, statSync } from "node:fs";
import { createServer } from "node:net";
import { basename, dirname, join, resolve } from "node:path";

import type { DesktopRPC } from "../shared/rpc";
import type { UpdateState } from "../shared/update-types";
import { VaultService } from "./services/vault-service";
import { runPlatformSetup } from "./services/platform-setup";

const DEV_SERVER_PORT = 5173;
const DEV_SERVER_URL = `http://localhost:${DEV_SERVER_PORT}`;
const DEFAULT_FRAME = {
	x: 180,
	y: 80,
	width: 1480,
	height: 940,
};
const WINDOW_STATE_PERSIST_MS = 1_500;
const MAC_TRAFFIC_LIGHTS_X = 14;
const MAC_TRAFFIC_LIGHTS_Y = 14;
const MAC_NATIVE_DRAG_REGION_X = 92;
const MAC_NATIVE_DRAG_REGION_HEIGHT = 40;

type DesktopBunRPC = ReturnType<typeof BrowserView.defineRPC<DesktopRPC>>;

async function getMainViewUrl(): Promise<string> {
	const channel = await Updater.localInfo.channel();
	if (channel === "dev") {
		try {
			await fetch(DEV_SERVER_URL, { method: "HEAD" });
			return DEV_SERVER_URL;
		} catch {
			console.log(
				"Vite dev server not running. Run 'bun run dev:hmr' for renderer HMR.",
			);
		}
	}

	return "views://mainview/index.html";
}

const isMacOS = process.platform === "darwin";
const vaultService = new VaultService();

// --- Update state machine ---
let updateState: UpdateState = {
	status: "idle",
	currentVersion: "0.1.0",
	availableVersion: null,
	error: null,
};

function broadcastUpdateState() {
	try {
		mainWindow?.webview.rpc?.send.updateStateChanged(updateState);
	} catch {}
}

function setUpdateState(patch: Partial<UpdateState>) {
	updateState = { ...updateState, ...patch };
	broadcastUpdateState();
}

async function performUpdateCheck(): Promise<UpdateState> {
	try {
		const channel = await Updater.localInfo.channel();
		if (channel === "dev") {
			setUpdateState({ status: "disabled" });
			return updateState;
		}

		setUpdateState({ status: "checking", error: null });
		const info = await Updater.checkForUpdate();

		if (info.updateAvailable) {
			setUpdateState({
				status: "available",
				availableVersion: info.version ?? null,
			});
		} else {
			setUpdateState({ status: "up-to-date" });
		}
	} catch (error) {
		setUpdateState({
			status: "error",
			error: error instanceof Error ? error.message : String(error),
		});
	}
	return updateState;
}

async function performUpdateDownload(): Promise<UpdateState> {
	if (updateState.status !== "available") return updateState;

	try {
		setUpdateState({ status: "downloading" });
		await Updater.downloadUpdate();
		setUpdateState({ status: "ready" });
		console.log("Update downloaded, will apply on next restart");
	} catch (error) {
		setUpdateState({
			status: "error",
			error: error instanceof Error ? error.message : String(error),
		});
	}
	return updateState;
}

// --- CLI argument parsing ---
function parseStartupArgs(): { vaultPath: string | null; selectFile: string | null } {
	const args = process.argv.slice(1);
	let vaultPath: string | null = null;
	let selectFile: string | null = null;

	for (let i = 0; i < args.length; i++) {
		const arg = args[i];
		if (arg === "--select" && i + 1 < args.length) {
			selectFile = args[++i];
		} else if (!arg.startsWith("-") && !arg.endsWith(".ts") && !arg.endsWith(".js")) {
			// Skip the bun script path itself
			try {
				const resolved = resolve(arg);
				if (existsSync(resolved)) {
					const stat = statSync(resolved);
					if (stat.isDirectory()) {
						vaultPath = resolved;
					} else if (stat.isFile() && resolved.endsWith(".typ")) {
						vaultPath = dirname(resolved);
						selectFile = basename(resolved);
					}
				}
			} catch {}
		}
	}

	return { vaultPath, selectFile };
}

let mainWindow:
	| BrowserWindow<ReturnType<typeof BrowserView.defineRPC<DesktopRPC>>>
	| null = null;

function requireMainWindow(): BrowserWindow<
	ReturnType<typeof BrowserView.defineRPC<DesktopRPC>>
> {
	if (!mainWindow) {
		throw new Error("main window is not ready yet");
	}
	return mainWindow;
}

const rpc = BrowserView.defineRPC<DesktopRPC>({
	handlers: {
		requests: {
			waitUntilReady: () => vaultService.waitUntilReady(),
			getBootstrapState: () => vaultService.getBootstrapState(),
			openVaultDialog: () => vaultService.openVaultDialog(requireMainWindow()),
			openRecentVault: ({ rootPath }) =>
				vaultService.openRecentVault(rootPath, requireMainWindow()),
			createVault: (params) =>
				vaultService.createVault(params, requireMainWindow()),
			closeVault: () => vaultService.closeVault(),
			readFile: ({ rootPath, path }) => vaultService.readFile(rootPath, path),
			stageFileWrite: ({ rootPath, path, content }) =>
				vaultService.stageFileWrite(rootPath, path, content),
			flushWrites: (params) => vaultService.flushWrites(params ?? {}),
			createFile: ({ rootPath, path, content }) =>
				vaultService.createFile(rootPath, path, content),
			createFilesBatch: ({ rootPath, entries }) =>
				vaultService.createFilesBatch(rootPath, entries),
			addBinaryFilesBatch: ({ rootPath, entries }) =>
				vaultService.addBinaryFilesBatch(rootPath, entries),
			createFolder: ({ rootPath, path }) =>
				vaultService.createFolder(rootPath, path),
			duplicateFile: ({ rootPath, sourcePath, targetPath }) =>
				vaultService.duplicateFile(rootPath, sourcePath, targetPath),
			renamePath: ({ rootPath, oldPath, newPath }) =>
				vaultService.renamePath(rootPath, oldPath, newPath),
			deletePath: ({ rootPath, path }) =>
				vaultService.deletePath(rootPath, path),
			revealInFinder: ({ absolutePath }) =>
				vaultService.revealInFinder(absolutePath),
			openPath: ({ absolutePath }) => vaultService.openPath(absolutePath),
			searchVaultPaths: ({ rootPath, query, limit, includeHidden }) =>
				vaultService.searchVaultPaths(rootPath, query, limit, includeHidden),
			searchVaultText: ({ rootPath, query, limit, includeHidden }) =>
				vaultService.searchVaultText(rootPath, query, limit, includeHidden),
			setHiddenFilesVisible: ({ rootPath, value }) =>
				vaultService.setHiddenFilesVisible(
					rootPath,
					value,
					requireMainWindow(),
				),
			toggleFavoriteVault: ({ rootPath }) =>
				vaultService.toggleFavoriteVault(rootPath),
			removeRecentVault: ({ rootPath }) =>
				vaultService.removeRecentVault(rootPath),
			persistLastFile: ({ rootPath, path }) =>
				vaultService.persistLastFile(rootPath, path),
			getCompileBundle: ({ rootPath, currentFilePath, liveSource }) =>
				vaultService.getCompileBundle(rootPath, currentFilePath, liveSource),
			getVaultStats: ({ rootPath, includeHidden }) =>
				vaultService.getVaultStats(rootPath, includeHidden),
			setWindowTitle: ({ title }) => {
				requireMainWindow().setTitle(title);
				return { ok: true as const };
			},
			checkForUpdate: () => performUpdateCheck(),
			downloadUpdate: () => performUpdateDownload(),
			applyUpdate: async () => {
				if (updateState.status === "ready") {
					await Updater.applyUpdate();
				}
			},
			quitApp: async () => {
				try {
					await vaultService.flushWrites({});
				} catch {}
				mainWindow?.close();
			},
		},
	},
});

function applyMacOSWindowEffects(window: BrowserWindow<DesktopBunRPC>) {
	const dylibPath = join(import.meta.dir, "libMacWindowEffects.dylib");

	if (!existsSync(dylibPath)) {
		console.warn(
			`Native macOS effects lib not found at ${dylibPath}. Falling back to transparent-only mode.`,
		);
		return;
	}

	try {
		const lib = dlopen(dylibPath, {
			enableWindowVibrancy: {
				args: [FFIType.ptr],
				returns: FFIType.bool,
			},
			ensureWindowShadow: {
				args: [FFIType.ptr],
				returns: FFIType.bool,
			},
			setWindowTrafficLightsPosition: {
				args: [FFIType.ptr, FFIType.f64, FFIType.f64],
				returns: FFIType.bool,
			},
			setNativeWindowDragRegion: {
				args: [FFIType.ptr, FFIType.f64, FFIType.f64],
				returns: FFIType.bool,
			},
		});

		const vibrancyEnabled = lib.symbols.enableWindowVibrancy(window.ptr);
		const shadowEnabled = lib.symbols.ensureWindowShadow(window.ptr);

		const alignButtons = () =>
			lib.symbols.setWindowTrafficLightsPosition(
				window.ptr,
				MAC_TRAFFIC_LIGHTS_X,
				MAC_TRAFFIC_LIGHTS_Y,
			);
		const alignNativeDragRegion = () =>
			lib.symbols.setNativeWindowDragRegion(
				window.ptr,
				MAC_NATIVE_DRAG_REGION_X,
				MAC_NATIVE_DRAG_REGION_HEIGHT,
			);

		const buttonsAligned = alignButtons();
		const dragRegionAligned = alignNativeDragRegion();

		setTimeout(() => {
			alignButtons();
			alignNativeDragRegion();
		}, 120);

		window.on("resize", () => {
			alignButtons();
			alignNativeDragRegion();
		});

		console.log(
			`macOS effects applied (vibrancy=${vibrancyEnabled}, shadow=${shadowEnabled}, trafficLights=${buttonsAligned}, nativeDrag=${dragRegionAligned})`,
		);
	} catch (error) {
		console.warn("Failed to apply native macOS effects:", error);
	}
}

function setupMacOSMenu(window: BrowserWindow<DesktopBunRPC>) {
	ApplicationMenu.setApplicationMenu([
		{
			submenu: [{ role: "quit" }],
		},
		{
			label: "File",
			submenu: [
				{
					label: "Close Window",
					action: "close-main-window",
					accelerator: "w",
				},
				{ type: "separator" },
				{ role: "quit" },
			],
		},
		{
			label: "Window",
			submenu: [
				{ role: "minimize" },
				{ role: "zoom" },
				{ role: "bringAllToFront" },
			],
		},
	]);

	ApplicationMenu.on("application-menu-clicked", (event: unknown) => {
		const action = (event as { data?: { action?: string } })?.data?.action;
		if (action === "close-main-window") {
			window.close();
		}
	});
}

const storedWindowState = await vaultService.getStoredWindowState();
const url = await getMainViewUrl();

mainWindow = new BrowserWindow<DesktopBunRPC>({
	title: "typsmthng",
	url,
	frame: {
		x: storedWindowState?.x ?? DEFAULT_FRAME.x,
		y: storedWindowState?.y ?? DEFAULT_FRAME.y,
		width: storedWindowState?.width ?? DEFAULT_FRAME.width,
		height: storedWindowState?.height ?? DEFAULT_FRAME.height,
	},
	titleBarStyle: isMacOS ? "hiddenInset" : "hidden",
	transparent: isMacOS,
	rpc,
});

if (isMacOS) {
	applyMacOSWindowEffects(mainWindow);
	setupMacOSMenu(mainWindow);
}

const persistWindowFrame = async (): Promise<void> => {
	if (!mainWindow) return;
	try {
		await vaultService.persistWindowState(mainWindow.getFrame());
	} catch (error) {
		console.error("Failed to persist window frame:", error);
	}
};

const framePersistTimer = setInterval(() => {
	void persistWindowFrame();
}, WINDOW_STATE_PERSIST_MS);

// --- IPC socket server for CLI ---
const SOCKET_DIR = join(
	process.env.HOME ?? process.env.USERPROFILE ?? "/tmp",
	".typsmthng",
);
const SOCKET_PATH = join(SOCKET_DIR, "cli.sock");

let cliServer: ReturnType<typeof createServer> | null = null;

async function handleOpenFromCli(vaultPath: string, selectFile: string | null) {
	const window = requireMainWindow();
	const vault = await vaultService.openRecentVault(vaultPath, window);
	if (vault && selectFile) {
		try {
			await vaultService.persistLastFile(vaultPath, selectFile);
		} catch {}
	}
	window.focus();
}

function startCliServer() {
	mkdirSync(SOCKET_DIR, { recursive: true });
	try {
		rmSync(SOCKET_PATH);
	} catch {}

	cliServer = createServer((conn) => {
		let data = "";
		conn.on("data", (chunk) => {
			data += chunk.toString();
		});
		conn.on("end", () => {
			try {
				const msg = JSON.parse(data);
				if (msg.action === "open" && msg.path) {
					void handleOpenFromCli(msg.path, msg.selectFile ?? null);
				}
			} catch {}
		});
	});

	cliServer.on("error", (err) => {
		console.warn("CLI server error:", err);
	});

	cliServer.listen(SOCKET_PATH);
}

startCliServer();

mainWindow.on("close", () => {
	clearInterval(framePersistTimer);
	void persistWindowFrame();
	if (cliServer) {
		cliServer.close();
		try {
			rmSync(SOCKET_PATH);
		} catch {}
	}
});

// Platform integration (CLI symlink, .desktop file, MIME type)
void runPlatformSetup();

// Check for updates (non-blocking, with UI feedback)
setTimeout(async () => {
	const state = await performUpdateCheck();
	if (state.status === "available") {
		await performUpdateDownload();
	}
}, 15_000);

// Handle startup arguments (e.g. `typsmthng /path/to/vault`)
const startupArgs = parseStartupArgs();
if (startupArgs.vaultPath) {
	void (async () => {
		await vaultService.waitUntilReady();
		await handleOpenFromCli(startupArgs.vaultPath!, startupArgs.selectFile);
	})();
}

console.log("typsmthng desktop window ready");
