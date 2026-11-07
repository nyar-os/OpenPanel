/**
 * Local config paths for the desktop client control plane.
 *
 * Secrets never live next to the tracked repo. Override with
 * `OPEN_PANEL_CONFIG_DIR` when needed.
 */
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

const CONFIG_DIR_ENV = 'OPEN_PANEL_CONFIG_DIR';
const APP_DIR_NAME = 'open-panel';

/** Resolve the per-user OpenPanel config directory. */
export function resolveConfigDir(): string {
    const override = process.env[CONFIG_DIR_ENV]?.trim();
    if (override) return path.resolve(override);

    if (process.platform === 'win32') {
        const appData = process.env.APPDATA?.trim();
        if (appData) return path.join(appData, APP_DIR_NAME);
        return path.join(os.homedir(), 'AppData', 'Roaming', APP_DIR_NAME);
    }

    const xdg = process.env.XDG_CONFIG_HOME?.trim();
    if (xdg) return path.join(xdg, APP_DIR_NAME);
    return path.join(os.homedir(), '.config', APP_DIR_NAME);
}

/** Ensure the config directory exists and return it. */
export function ensureConfigDir(): string {
    const dir = resolveConfigDir();
    fs.mkdirSync(dir, { recursive: true });
    return dir;
}

/** Managed server roster JSON path. */
export function rosterPath(): string {
    return path.join(resolveConfigDir(), 'servers.json');
}
