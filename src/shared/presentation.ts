/**
 * Presentation protocol shared by the presenter (main window), the bun host,
 * and audience windows. The presenter owns the state; bun relays snapshots to
 * audience windows and relays their input back.
 */

export interface PresentationPageSize {
  width: number;
  height: number;
}

/**
 * Where speaker notes live inside the compiled pages, if anywhere. Slide
 * packages (touying, polylux) can render notes in the right half of a
 * double-width page; "right" shows only the left half to the audience.
 */
export type NotesLayout = "auto" | "right" | "none";

/**
 * Deck payload sent to audience windows. `svg` is the whole compiled document
 * (all pages stacked vertically) exactly as the preview renders it; each
 * window slices it into slides locally so a recompile is one message.
 */
export interface PresentationDeck {
  revision: number;
  title: string;
  svg: string;
  pages: PresentationPageSize[];
  notesLayout: NotesLayout;
}

export type PresentationTool = "pointer" | "laser" | "pen" | "highlighter" | "eraser";

export type BlackoutMode = "none" | "black" | "white";

export interface StrokePoint {
  /** Normalised to the slide box: 0..1 across the slide width. */
  x: number;
  /** Normalised to the slide box: 0..1 across the slide height. */
  y: number;
}

export interface Stroke {
  id: string;
  tool: "pen" | "highlighter";
  color: string;
  /** Width as a fraction of the slide width, so it scales with any display. */
  width: number;
  points: StrokePoint[];
}

export interface LaserPointer {
  x: number;
  y: number;
  visible: boolean;
}

export interface PresentationState {
  /** Zero-based current slide. */
  slide: number;
  blackout: BlackoutMode;
  tool: PresentationTool;
  penColor: string;
  laser: LaserPointer;
  /** Keyed by slide index (as string so it survives JSON). */
  annotations: Record<string, Stroke[]>;
  /** Whether the audience should mirror the presenter's cursor as a laser. */
  laserEnabled: boolean;
}

export const DEFAULT_PEN_COLOR = "#FF4D00";

export const PEN_COLORS = ["#FF4D00", "#FFD400", "#2BD46B", "#3B9DFF", "#FF3B80", "#111111", "#FFFFFF"] as const;

export function createInitialPresentationState(): PresentationState {
  return {
    slide: 0,
    blackout: "none",
    tool: "pointer",
    penColor: DEFAULT_PEN_COLOR,
    laser: { x: 0.5, y: 0.5, visible: false },
    annotations: {},
    laserEnabled: false,
  };
}

/** Full snapshot pushed to audience windows. Either half may be omitted. */
export interface PresentationSnapshot {
  deck?: PresentationDeck | null;
  state?: PresentationState;
  /** True when the presenter ended the session; audience windows should show idle. */
  ended?: boolean;
}

export type PresentationAction =
  | "next"
  | "prev"
  | "first"
  | "last"
  | "toggle-black"
  | "toggle-white"
  | "toggle-laser"
  | "toggle-pen"
  | "toggle-highlighter"
  | "toggle-eraser"
  | "clear-annotations"
  | "toggle-grid"
  | "toggle-notes"
  | "toggle-timer"
  | "reset-timer"
  | "toggle-fullscreen"
  | "exit";

/** Input that originates in an audience window and is relayed to the presenter. */
export type PresentationInput =
  | { kind: "action"; action: PresentationAction }
  | { kind: "goto"; slide: number }
  | { kind: "laser"; pointer: LaserPointer }
  | { kind: "stroke"; slide: number; stroke: Stroke }
  | { kind: "erase"; slide: number; strokeId: string }
  | { kind: "ready" };

export interface DisplayInfo {
  id: number;
  bounds: { x: number; y: number; width: number; height: number };
  workArea: { x: number; y: number; width: number; height: number };
  scaleFactor: number;
  isPrimary: boolean;
  /** True when the main (presenter) window currently sits on this display. */
  hasMainWindow: boolean;
  /** True when an audience window is currently shown on this display. */
  hasAudienceWindow: boolean;
}

export interface PresentationDisplays {
  displays: DisplayInfo[];
}

export type PresentationCommand = "present-here" | "presenter-view" | "end-presentation";

export function clampSlide(slide: number, slideCount: number): number {
  if (!Number.isFinite(slide) || slideCount <= 0) return 0;
  return Math.max(0, Math.min(slideCount - 1, Math.trunc(slide)));
}
