import type { WindowState } from "./rpc";

/** Default restored / first-launch window frame. Keep in sync across bun + app-state. */
export const DEFAULT_WINDOW_FRAME = {
  x: 180,
  y: 80,
  width: 1480,
  height: 940,
} as const satisfies Required<WindowState>;

export const MIN_WINDOW_WIDTH = 900;
export const MIN_WINDOW_HEIGHT = 600;

/** Allow slight off-screen placement (multi-monitor) but reject runaway negatives. */
export const MIN_WINDOW_COORDINATE = -100;

function finiteOr(value: number | undefined, fallback: number): number {
  return typeof value === "number" && Number.isFinite(value) ? value : fallback;
}

/**
 * Normalize persisted or restored window geometry to usable bounds.
 */
export function clampWindowState(state: WindowState): WindowState {
  const width = Math.max(
    MIN_WINDOW_WIDTH,
    Math.round(finiteOr(state.width, DEFAULT_WINDOW_FRAME.width)),
  );
  const height = Math.max(
    MIN_WINDOW_HEIGHT,
    Math.round(finiteOr(state.height, DEFAULT_WINDOW_FRAME.height)),
  );

  const next: WindowState = { width, height };

  if (state.x !== undefined) {
    next.x = Math.max(
      MIN_WINDOW_COORDINATE,
      Math.round(finiteOr(state.x, DEFAULT_WINDOW_FRAME.x)),
    );
  }
  if (state.y !== undefined) {
    next.y = Math.max(
      MIN_WINDOW_COORDINATE,
      Math.round(finiteOr(state.y, DEFAULT_WINDOW_FRAME.y)),
    );
  }

  return next;
}
