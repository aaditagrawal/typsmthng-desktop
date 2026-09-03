import fs from "node:fs/promises";
import path from "node:path";

import type { UserSettings } from "../../shared/rpc";
import { getUserDataDir } from "./version-info";

const SETTINGS_FILENAME = "user-settings.json";

function settingsPath(): string {
  return path.join(getUserDataDir(), SETTINGS_FILENAME);
}

export async function loadUserSettings(): Promise<UserSettings | null> {
  try {
    const raw = await fs.readFile(settingsPath(), "utf8");
    const parsed = JSON.parse(raw) as UserSettings;
    return parsed && typeof parsed === "object" ? parsed : null;
  } catch {
    return null;
  }
}

export async function saveUserSettings(settings: UserSettings): Promise<{ ok: true }> {
  await fs.mkdir(getUserDataDir(), { recursive: true });
  await fs.writeFile(settingsPath(), `${JSON.stringify(settings, null, 2)}\n`, "utf8");
  return { ok: true as const };
}
