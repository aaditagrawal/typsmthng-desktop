export interface DisplayRect {
  x: number;
  y: number;
  width: number;
  height: number;
}

export interface DisplayLike {
  id: number;
  bounds: DisplayRect;
  isPrimary: boolean;
}

function rectContains(bounds: DisplayRect, x: number, y: number): boolean {
  return x >= bounds.x && x < bounds.x + bounds.width && y >= bounds.y && y < bounds.y + bounds.height;
}

/** The display holding the centre of a window frame, if any. */
export function findDisplayForFrame<T extends DisplayLike>(displays: T[], frame: DisplayRect | null): T | null {
  if (!frame || displays.length === 0) return null;
  const cx = frame.x + frame.width / 2;
  const cy = frame.y + frame.height / 2;
  return displays.find((display) => rectContains(display.bounds, cx, cy)) ?? null;
}

/**
 * Pick the display an audience window should default to: an external display
 * that does not hold the presenter, preferring non-primary screens. With a
 * single display the audience simply covers it.
 */
export function pickDefaultAudienceDisplay<T extends DisplayLike>(displays: T[], mainDisplayId: number | null): T | null {
  if (displays.length === 0) return null;
  const others = displays.filter((display) => display.id !== mainDisplayId);
  if (others.length === 0) return displays[0] ?? null;
  return others.find((display) => !display.isPrimary) ?? others[0] ?? null;
}

/** A window frame that sits unambiguously inside the display before going fullscreen. */
export function insetFrame(bounds: DisplayRect, inset: number): DisplayRect {
  return {
    x: bounds.x + inset,
    y: bounds.y + inset,
    width: Math.max(320, bounds.width - inset * 2),
    height: Math.max(200, bounds.height - inset * 2),
  };
}
