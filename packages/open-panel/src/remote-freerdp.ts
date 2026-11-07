/**
 * Resolve / cache a local FreeRDP client used for Windows RDP execution.
 *
 * The remote Windows host does not need OpenSSH. The controller needs a
 * FreeRDP binary (`wfreerdp` / `xfreerdp`) that can redirect a drive and start
 * a one-shot shell.
 */
import fs from 'node:fs';
import https from 'node:https';
import path from 'node:path';
import { spawnSync } from 'node:child_process';

import { AppError } from './errors.js';
import { ensureConfigDir, resolveConfigDir } from './paths.js';

const FREERDP_ENV = 'OPEN_PANEL_FREERDP_PATH';
// Nightly artifact URLs churn. Prefer an explicit local binary via
// `OPEN_PANEL_FREERDP_PATH` or `OPEN_PANEL_FREERDP_URL`. Auto-download is best-effort only.
const DEFAULT_DOWNLOAD_URL =
    process.env.OPEN_PANEL_FREERDP_URL?.trim()
    || 'https://ci.freerdp.com/job/freerdp-nightly-windows/lastSuccessfulBuild/artifact/wfreerdp.exe';

function candidatePaths(): string[] {
    const out: string[] = [];
    const env = process.env[FREERDP_ENV]?.trim();
    if (env) out.push(env);
    out.push(
        path.join(resolveConfigDir(), 'tools', 'wfreerdp.exe'),
        path.join(resolveConfigDir(), 'tools', 'xfreerdp.exe'),
        'wfreerdp',
        'xfreerdp',
        'C:\\ProgramData\\chocolatey\\bin\\wfreerdp.exe',
    );
    return out;
}

function looksExecutable(file: string): boolean {
    if (file.includes('\\') || file.includes('/') || file.endsWith('.exe')) {
        return fs.existsSync(file);
    }
    const probe = spawnSync(file, ['/version'], { encoding: 'utf8', windowsHide: true });
    return probe.error === undefined;
}

/** Return a usable FreeRDP binary path, or undefined. */
export function findFreeRdp(): string | undefined {
    for (const candidate of candidatePaths()) {
        if (looksExecutable(candidate)) return candidate;
    }
    return undefined;
}

function downloadFile(url: string, destination: string): Promise<void> {
    return new Promise((resolve, reject) => {
        const file = fs.createWriteStream(destination);
        const request = https.get(url, (response) => {
            if (response.statusCode && response.statusCode >= 300 && response.statusCode < 400 && response.headers.location) {
                file.close();
                fs.unlinkSync(destination);
                downloadFile(response.headers.location, destination).then(resolve, reject);
                return;
            }
            if (response.statusCode !== 200) {
                file.close();
                fs.unlinkSync(destination);
                reject(new Error(`download failed with HTTP ${response.statusCode}`));
                return;
            }
            response.pipe(file);
            file.on('finish', () => {
                file.close(() => resolve());
            });
        });
        request.on('error', (error) => {
            try {
                file.close();
                fs.unlinkSync(destination);
            }
            catch {
                // ignore
            }
            reject(error);
        });
    });
}

/**
 * Ensure a FreeRDP client exists locally.
 *
 * On Windows, downloads a nightly `wfreerdp.exe` into the user config tools dir
 * when none is found. Never writes into the git tree.
 */
export async function ensureFreeRdp(): Promise<string> {
    const existing = findFreeRdp();
    if (existing) return existing;
    if (process.platform !== 'win32') {
        throw new AppError({
            code: 'dependency_blocked',
            capability: 'remote-rdp',
            message:
                'FreeRDP client not found. Install `xfreerdp`/`wfreerdp` or set `OPEN_PANEL_FREERDP_PATH`.',
        });
    }
    const toolsDir = path.join(ensureConfigDir(), 'tools');
    fs.mkdirSync(toolsDir, { recursive: true });
    const destination = path.join(toolsDir, 'wfreerdp.exe');
    const temp = `${destination}.${process.pid}.tmp`;
    try {
        await downloadFile(DEFAULT_DOWNLOAD_URL, temp);
        fs.renameSync(temp, destination);
    }
    catch (error) {
        try {
            if (fs.existsSync(temp)) fs.unlinkSync(temp);
        }
        catch {
            // ignore
        }
        throw new AppError({
            code: 'dependency_blocked',
            capability: 'remote-rdp',
            message:
                `failed to download FreeRDP client: ${error instanceof Error ? error.message : String(error)}. ` +
                `Install wfreerdp manually or set \`${FREERDP_ENV}\`.`,
            path: destination,
        });
    }
    if (!fs.existsSync(destination)) {
        throw new AppError({
            code: 'dependency_blocked',
            capability: 'remote-rdp',
            message: `FreeRDP download did not produce \`${destination}\``,
            path: destination,
        });
    }
    return destination;
}
