import { BrowserView, BrowserWindow, Screen, type Display } from "electrobun/bun";

import type {
	DisplayInfo,
	PresentationDeck,
	PresentationDisplays,
	PresentationInput,
	PresentationSnapshot,
	PresentationState,
} from "../../shared/presentation";
import type { DesktopRPC } from "../../shared/rpc";
import { findDisplayForFrame, insetFrame, pickDefaultAudienceDisplay } from "../../shared/display-layout";

export type DesktopBunRPC = ReturnType<typeof BrowserView.defineRPC<DesktopRPC>>;

export const AUDIENCE_HASH = "#/audience";

/** Fullscreen transitions on macOS misbehave when requested in the same tick as window creation. */
const FULLSCREEN_DELAY_MS = 160;
const FULLSCREEN_FALLBACK_DELAY_MS = 900;
/** Inset so the pre-fullscreen frame is unambiguously on the target display. */
const DISPLAY_INSET_PX = 48;

interface AudienceWindow {
	window: BrowserWindow<DesktopBunRPC>;
	displayId: number | null;
}

export interface PresentationWindowDeps {
	createRpc: () => DesktopBunRPC;
	getAudienceUrl: () => string;
	getMainWindow: () => BrowserWindow<DesktopBunRPC> | null;
}

export class PresentationWindowManager {
	private audience: AudienceWindow | null = null;
	private deck: PresentationDeck | null = null;
	private state: PresentationState | null = null;
	private ended = true;
	private audienceFocused = false;

	constructor(private readonly deps: PresentationWindowDeps) {}

	/** Call from the main window's focus handler so menu actions can tell windows apart. */
	noteMainWindowFocused(): void {
		this.audienceFocused = false;
	}

	isAudienceFocused(): boolean {
		return this.audience !== null && this.audienceFocused;
	}

	private safeDisplays(): Display[] {
		try {
			const displays = Screen.getAllDisplays();
			if (displays.length > 0) return displays;
			return [Screen.getPrimaryDisplay()];
		} catch (error) {
			console.warn("Failed to enumerate displays:", error);
			return [];
		}
	}

	private mainWindowDisplayId(displays: Display[]): number | null {
		const main = this.deps.getMainWindow();
		if (!main) return null;
		try {
			return findDisplayForFrame(displays, main.getFrame())?.id ?? null;
		} catch {
			return null;
		}
	}

	listDisplays(): PresentationDisplays {
		const displays = this.safeDisplays();
		const mainDisplayId = this.mainWindowDisplayId(displays);
		return {
			displays: displays.map<DisplayInfo>((display) => ({
				id: display.id,
				bounds: display.bounds,
				workArea: display.workArea,
				scaleFactor: display.scaleFactor,
				isPrimary: display.isPrimary,
				hasMainWindow: display.id === mainDisplayId,
				hasAudienceWindow: this.audience?.displayId === display.id,
			})),
		};
	}

	isAudienceWindow(windowId: number): boolean {
		return this.audience?.window.id === windowId;
	}

	hasAudience(): boolean {
		return this.audience !== null;
	}

	openAudience(displayId: number | null): { ok: boolean; displayId: number | null } {
		const displays = this.safeDisplays();
		const mainDisplayId = this.mainWindowDisplayId(displays);
		const target = displayId !== null
			? displays.find((display) => display.id === displayId) ?? pickDefaultAudienceDisplay(displays, mainDisplayId)
			: pickDefaultAudienceDisplay(displays, mainDisplayId);

		if (this.audience && this.audience.displayId === (target?.id ?? null)) {
			this.audience.window.focus();
			return { ok: true, displayId: this.audience.displayId };
		}

		// Moving a fullscreen window across displays is unreliable (macOS
		// animates the exit asynchronously), so recreate it on the new display.
		this.closeAudience();

		const bounds = target?.bounds ?? { x: 0, y: 0, width: 1280, height: 720 };
		const frame = insetFrame(bounds, DISPLAY_INSET_PX);

		let window: BrowserWindow<DesktopBunRPC>;
		try {
			window = new BrowserWindow<DesktopBunRPC>({
				title: "typsmthng — Presentation",
				url: this.deps.getAudienceUrl(),
				frame,
				titleBarStyle: "hidden",
				transparent: false,
				rpc: this.deps.createRpc(),
			});
		} catch (error) {
			console.error("Failed to open audience window:", error);
			return { ok: false, displayId: null };
		}

		const entry: AudienceWindow = { window, displayId: target?.id ?? null };
		this.audience = entry;

		window.on("focus", () => {
			if (this.audience?.window.id === window.id) this.audienceFocused = true;
		});

		window.on("close", () => {
			if (this.audience?.window.id !== window.id) return;
			this.audience = null;
			this.audienceFocused = false;
			this.notifyMain((rpc) => rpc.send.presentationAudienceClosed());
		});

		setTimeout(() => {
			if (this.audience?.window.id !== window.id) return;
			try {
				window.setFullScreen(true);
			} catch (error) {
				console.warn("Failed to fullscreen audience window:", error);
			}
			// Some window managers refuse fullscreen for undecorated windows;
			// covering the display bounds exactly is the next best thing.
			setTimeout(() => {
				if (this.audience?.window.id !== window.id) return;
				try {
					if (!window.isFullScreen()) {
						window.setFrame(bounds.x, bounds.y, bounds.width, bounds.height);
					}
				} catch {}
			}, FULLSCREEN_FALLBACK_DELAY_MS);
		}, FULLSCREEN_DELAY_MS);

		return { ok: true, displayId: entry.displayId };
	}

	closeAudience(): void {
		const current = this.audience;
		if (!current) return;
		this.audience = null;
		this.audienceFocused = false;
		try {
			if (current.window.isFullScreen()) current.window.setFullScreen(false);
		} catch {}
		try {
			current.window.close();
		} catch (error) {
			console.warn("Failed to close audience window:", error);
		}
	}

	publish(snapshot: PresentationSnapshot): void {
		if (snapshot.deck !== undefined) this.deck = snapshot.deck;
		if (snapshot.state !== undefined) this.state = snapshot.state;
		if (snapshot.ended !== undefined) this.ended = snapshot.ended;
		else if (snapshot.deck || snapshot.state) this.ended = false;

		if (snapshot.ended) {
			this.deck = null;
			this.state = null;
		}

		const audience = this.audience;
		if (!audience) return;
		try {
			audience.window.webview.rpc?.send.presentationSnapshot(snapshot);
		} catch (error) {
			console.warn("Failed to push presentation snapshot:", error);
		}
	}

	getSnapshot(): PresentationSnapshot {
		return { deck: this.deck, state: this.state ?? undefined, ended: this.ended };
	}

	relayInput(input: PresentationInput): void {
		if (input.kind === "ready") {
			const audience = this.audience;
			if (audience) {
				try {
					audience.window.webview.rpc?.send.presentationSnapshot(this.getSnapshot());
				} catch {}
			}
		}
		this.notifyMain((rpc) => rpc.send.presentationInput(input));
	}

	private notifyMain(fn: (rpc: DesktopBunRPC) => void): void {
		const main = this.deps.getMainWindow();
		const rpc = main?.webview?.rpc;
		if (!rpc) return;
		try {
			fn(rpc);
		} catch (error) {
			console.warn("Failed to notify presenter window:", error);
		}
	}
}
