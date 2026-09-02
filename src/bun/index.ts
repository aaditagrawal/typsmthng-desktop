import { ApplicationMenu, BrowserView, BrowserWindow, Updater } from "electrobun/bun";
import { dlopen, FFIType } from "bun:ffi";
import { existsSync, mkdirSync, rmSync, statSync } from "node:fs";
import { createServer, connect } from "node:net";
import { extname, join, resolve } from "node:path";

import type { PresentationCommand } from "../shared/presentation";
import type { DesktopRPC } from "../shared/rpc";
import type { UpdateState } from "../shared/update-types";
import { resolveVaultRootFromTypFile } from "../shared/vault-root";
import { DEFAULT_WINDOW_FRAME, clampWindowState } from "../shared/window-state";
import { VaultService } from "./services/vault-service";
import { ensurePackagedWorkingDirectory, resolvePackagedAppRoot, runPlatformSetup } from "./services/platform-setup";
import { saveDownloadFile } from "./services/save-download";
import { loadUserSettings, saveUserSettings } from "./services/user-settings";
import { loadSystemFontFiles } from "./services/system-fonts";
import { AUDIENCE_HASH, PresentationWindowManager } from "./services/presentation-windows";

// $OWD is set by the AppImage runtime; the AppRun script cds into the AppDir,
// so process.cwd() there is not where the user invoked us from.
const LAUNCH_CWD = process.env.OWD ?? process.cwd();
ensurePackagedWorkingDirectory();

const DEV_SERVER_PORT = 5173;
const DEV_SERVER_URL = `http://localhost:${DEV_SERVER_PORT}`;
const DEFAULT_FRAME = DEFAULT_WINDOW_FRAME;
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
	currentVersion: "0.0.0",
	availableVersion: null,
	error: null,
};

// Resolve the real packaged version instead of hardcoding it (it would
// silently drift on the next release).
void Updater.localInfo
	.version()
	.then((version) => {
		if (version) setUpdateState({ currentVersion: version });
	})
	.catch(() => {});

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
		const info = await Updater.checkForUpdate() as {
			updateAvailable?: boolean;
			version?: string;
			error?: unknown;
		};

		if (info.error) {
			setUpdateState({
				status: "error",
				error: info.error instanceof Error ? info.error.message : String(info.error),
			});
		} else if (info.updateAvailable) {
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
		} else if (!arg.startsWith("-")) {
			try {
				// Resolve against the directory the user launched from, not the
				// app root that ensurePackagedWorkingDirectory chdir'd into.
				const resolved = resolve(LAUNCH_CWD, arg);
				if (existsSync(resolved)) {
					const stat = statSync(resolved);
					if (stat.isDirectory()) {
						vaultPath = resolved;
					} else if (stat.isFile() && extname(resolved).toLowerCase() === ".typ") {
						const fromTyp = resolveVaultRootFromTypFile(resolved);
						vaultPath = fromTyp.vaultPath;
						selectFile = fromTyp.selectFile;
					}
					// Other files (including the bun script path itself) are ignored.
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

let mainViewUrl = "views://mainview/index.html";

const presentationWindows = new PresentationWindowManager({
	createRpc: () => createRpc(),
	getAudienceUrl: () => `${mainViewUrl}${AUDIENCE_HASH}`,
	getMainWindow: () => mainWindow,
});

// Every webview needs its own RPC instance because Electrobun binds the
// transport to the RPC object; the handlers themselves are shared.
function createRpc(): DesktopBunRPC {
	return BrowserView.defineRPC<DesktopRPC>({
	handlers: {
		requests: {
			waitUntilReady: () => vaultService.waitUntilReady(),
			getBootstrapState: (params) =>
				vaultService.getBootstrapState(requireMainWindow(), {
					restoreActive: params?.restoreActive,
				}),
			openVaultDialog: () => vaultService.openVaultDialog(requireMainWindow()),
			openRecentVault: ({ rootPath }) =>
				vaultService.openRecentVault(rootPath, requireMainWindow()),
			createVault: (params) =>
				vaultService.createVault(params, requireMainWindow()),
			closeVault: (params) => vaultService.closeVault(params ?? {}),
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
			getVaultExportBundle: ({ rootPath }) =>
				vaultService.getVaultExportBundle(rootPath),
			saveDownload: ({ filename, data }) => saveDownloadFile(filename, data),
			getUserSettings: () => loadUserSettings(),
			setUserSettings: ({ settings }) => saveUserSettings(settings),
			loadSystemFonts: ({ families }) => loadSystemFontFiles(families),
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
					vaultService.flushWritesSync();
					await vaultService.flushWrites({});
				} catch {}
				presentationWindows.closeAudience();
				mainWindow?.close();
			},
			setMainWindowFullScreen: ({ fullScreen }) => {
				const window = requireMainWindow();
				if (window.isFullScreen() !== fullScreen) {
					window.setFullScreen(fullScreen);
				}
				return { ok: true as const };
			},
			presentationGetDisplays: () => presentationWindows.listDisplays(),
			presentationOpenAudience: ({ displayId }) =>
				presentationWindows.openAudience(displayId),
			presentationCloseAudience: () => {
				presentationWindows.closeAudience();
				return { ok: true as const };
			},
			presentationPublish: (snapshot) => {
				presentationWindows.publish(snapshot);
				return { ok: true as const };
			},
			presentationInput: (input) => {
				presentationWindows.relayInput(input);
				return { ok: true as const };
			},
			presentationGetSnapshot: () => presentationWindows.getSnapshot(),
		},
	},
	});
}

const rpc = createRpc();

function applyMacOSWindowEffects(window: BrowserWindow<DesktopBunRPC>) {
	// With ASAR packaging the bun entrypoint runs from a temp dir, so also
	// resolve the dylib through the real bundle (Resources/app/bun/).
	const appRoot = resolvePackagedAppRoot(process.execPath);
	const dylibCandidates = [
		join(import.meta.dir, "libMacWindowEffects.dylib"),
		...(appRoot ? [join(appRoot, "Resources", "app", "bun", "libMacWindowEffects.dylib")] : []),
	];
	const dylibPath = dylibCandidates.find((candidate) => existsSync(candidate));

	if (!dylibPath) {
		console.warn(
			`Native macOS effects lib not found (checked ${dylibCandidates.join(", ")}). Falling back to transparent-only mode.`,
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

function sendPresentationCommand(command: PresentationCommand) {
	try {
		mainWindow?.webview.rpc?.send.presentationCommand(command);
	} catch {}
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
					action: "close-window",
					accelerator: "w",
				},
				{ type: "separator" },
				{ role: "quit" },
			],
		},
		{
			label: "Present",
			submenu: [
				{
					label: "Start Presentation",
					action: "present-here",
					accelerator: "CommandOrControl+Shift+P",
				},
				{
					label: "Presenter View…",
					action: "presenter-view",
					accelerator: "CommandOrControl+Alt+P",
				},
				{ type: "separator" },
				{
					label: "End Presentation",
					action: "end-presentation",
				},
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
		switch (action) {
			case "close-window":
				// Cmd+W on the fullscreen audience window should only end the
				// presentation, never take the editor down with it.
				if (presentationWindows.isAudienceFocused()) {
					sendPresentationCommand("end-presentation");
				} else {
					window.close();
				}
				break;
			case "present-here":
			case "presenter-view":
			case "end-presentation":
				sendPresentationCommand(action);
				break;
		}
	});
}

const isWindows = process.platform === "win32";
// Per-user pipe name: a machine-global pipe would route a second logged-in
// user's opens into the first user's session.
const windowsPipeUser = (process.env.USERNAME ?? "default").replace(/[^\w.-]/g, "_");
const socketHome = process.env.HOME ?? process.env.USERPROFILE ?? null;
const SOCKET_DIR = isWindows || !socketHome ? "" : join(socketHome, ".typsmthng");
const SOCKET_PATH = isWindows
	? `\\\\.\\pipe\\typsmthng-cli-${windowsPipeUser}`
	: SOCKET_DIR
		? join(SOCKET_DIR, "cli.sock")
		: null;

let cliServer: ReturnType<typeof createServer> | null = null;

function tryForwardToRunningInstance(): Promise<boolean> {
	return new Promise((resolve) => {
		if (!SOCKET_PATH) {
			resolve(false);
			return;
		}
		const client = connect(SOCKET_PATH);
		// A stale socket file fails fast with ECONNREFUSED; the timeout only
		// covers a live-but-busy instance, so keep it generous to avoid a
		// second instance stealing (and unlinking) the live server's socket.
		const timer = setTimeout(() => {
			client.destroy();
			resolve(false);
		}, 1500);
		client.on("connect", () => {
			clearTimeout(timer);
			const args = parseStartupArgs();
			const payload = args.vaultPath
				? { action: "open", path: args.vaultPath, selectFile: args.selectFile }
				: { action: "focus" };
			client.write(JSON.stringify(payload));
			client.end();
			resolve(true);
		});
		client.on("error", () => {
			clearTimeout(timer);
			resolve(false);
		});
	});
}

if (await tryForwardToRunningInstance()) {
	process.exit(0);
}

const storedWindowState = await vaultService.getStoredWindowState();
const restoredFrame = clampWindowState({
	x: storedWindowState?.x ?? DEFAULT_FRAME.x,
	y: storedWindowState?.y ?? DEFAULT_FRAME.y,
	width: storedWindowState?.width ?? DEFAULT_FRAME.width,
	height: storedWindowState?.height ?? DEFAULT_FRAME.height,
});
const url = await getMainViewUrl();
mainViewUrl = url;

mainWindow = new BrowserWindow<DesktopBunRPC>({
	title: "typsmthng",
	url,
	frame: {
		x: restoredFrame.x ?? DEFAULT_FRAME.x,
		y: restoredFrame.y ?? DEFAULT_FRAME.y,
		width: restoredFrame.width,
		height: restoredFrame.height,
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

async function handleOpenFromCli(vaultPath: string | null, selectFile: string | null) {
	const window = requireMainWindow();
	if (vaultPath) {
		const vault = await vaultService.openRecentVault(vaultPath, window, selectFile);
		if (vault && selectFile) {
			try {
				await vaultService.persistLastFile(vaultPath, selectFile);
			} catch {}
		}
		if (vault) {
			await vaultService.resendActiveVault(window);
		}
	}
	window.focus();
}

function startCliServer() {
	if (!SOCKET_PATH) return;
	if (!isWindows) {
		mkdirSync(SOCKET_DIR, { recursive: true });
		try {
			rmSync(SOCKET_PATH);
		} catch {}
	}

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
				} else if (msg.action === "focus") {
					void handleOpenFromCli(null, null);
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

mainWindow.on("focus", () => {
	presentationWindows.noteMainWindowFocused();
});

mainWindow.on("close", () => {
	presentationWindows.closeAudience();
	vaultService.flushWritesSync();
	void (async () => {
		try {
			await vaultService.flushWrites({});
		} catch {}
		clearInterval(framePersistTimer);
		void persistWindowFrame();
		if (cliServer) {
			cliServer.close();
			if (!isWindows && SOCKET_PATH) {
				try {
					rmSync(SOCKET_PATH);
				} catch {}
			}
		}
	})();
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

// Handle startup arguments (e.g. `typsmthng /path/to/vault`).
// Only set the override — getBootstrapState opens it once. Calling handleOpenFromCli
// here raced bootstrap openVault and could orphan watchers / desync UI state.
const startupArgs = parseStartupArgs();
if (startupArgs.vaultPath) {
	vaultService.setStartupVaultOverride(
		startupArgs.vaultPath,
		startupArgs.selectFile ?? null,
	);
}

console.log("typsmthng desktop window ready");
