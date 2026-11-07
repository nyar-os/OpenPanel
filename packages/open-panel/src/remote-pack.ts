/**
 * Build an installable tarball for remote deploy.
 *
 * Prefer public npm when a real version is published. Until then, pack the
 * local workspace package (`0.0.0`) so RDP install can still be accepted.
 */
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { AppError } from './errors.js';
import { workspaceRoot } from './native.js';
import { ensureConfigDir } from './paths.js';

export interface PackResult {
    mode: 'npm-pack' | 'workspace-pack';
    packageName: string;
    version: string;
    tarballPath: string;
    note: string;
}

function readPackageJson(): { name: string; version: string } {
    const file = path.join(workspaceRoot(), 'packages', 'open-panel', 'package.json');
    const raw = JSON.parse(fs.readFileSync(file, 'utf8')) as { name?: string; version?: string };
    if (!raw.name || !raw.version) {
        throw new AppError({ code: 'internal_error', message: `invalid package.json at \`${file}\``, path: file });
    }
    return { name: raw.name, version: raw.version };
}

/** Create a tarball under the user config dir (never inside git). */
export function packOpenPanel(): PackResult {
    const pkg = readPackageJson();
    const outDir = path.join(ensureConfigDir(), 'packs');
    fs.mkdirSync(outDir, { recursive: true });

    // Package is still 0.0.0 / unpublished: always pack from the workspace.
    // When a real registry version exists later, callers can pass --from npm.
    // Strip `workspace:*` optionalDeps so a remote `npm install -g` outside the
    // monorepo can resolve the tarball (optional native package is omitted).
    const srcDir = path.join(workspaceRoot(), 'packages', 'open-panel');
    const cwd = fs.mkdtempSync(path.join(os.tmpdir(), 'open-panel-pack-'));
    try {
        copyPackTree(srcDir, cwd);
        const pkgPath = path.join(cwd, 'package.json');
        const pkgJson = JSON.parse(fs.readFileSync(pkgPath, 'utf8')) as {
            optionalDependencies?: Record<string, string>;
        };
        if (pkgJson.optionalDependencies) {
            const cleaned: Record<string, string> = {};
            for (const [name, version] of Object.entries(pkgJson.optionalDependencies)) {
                if (version.startsWith('workspace:')) continue;
                cleaned[name] = version;
            }
            if (Object.keys(cleaned).length) {
                pkgJson.optionalDependencies = cleaned;
            }
            else {
                delete pkgJson.optionalDependencies;
            }
            fs.writeFileSync(pkgPath, `${JSON.stringify(pkgJson, null, 4)}\n`, 'utf8');
        }
    }
    catch (error) {
        try {
            fs.rmSync(cwd, { recursive: true, force: true });
        }
        catch {
            // ignore cleanup
        }
        throw error instanceof AppError
            ? error
            : new AppError({
                code: 'pack_failed',
                capability: 'remote-install',
                message: error instanceof Error ? error.message : String(error),
            });
    }
    const npmBin = process.platform === 'win32' ? 'npm.cmd' : 'npm';
    const result = spawnSync(
        npmBin,
        ['pack', '--pack-destination', outDir],
        {
            cwd,
            encoding: 'utf8',
            windowsHide: true,
            env: process.env,
            // Windows needs the shell so `npm.cmd` resolves via PATHEXT.
            ...(process.platform === 'win32' ? { shell: true as const } : {}),
        },
    );
    try {
        fs.rmSync(cwd, { recursive: true, force: true });
    }
    catch {
        // Temp pack tree cleanup must not mask pack results.
    }
    if (result.status !== 0) {
        const detail = [
            result.error?.message,
            result.stderr?.trim(),
            result.stdout?.trim(),
            result.status == null ? undefined : `exit ${result.status}`,
        ].filter(Boolean).join(' | ');
        throw new AppError({
            code: 'pack_failed',
            capability: 'remote-install',
            message: detail || 'npm pack failed',
            path: cwd,
        });
    }
    const lines = (result.stdout || '').split(/\r?\n/).map((line) => line.trim()).filter(Boolean);
    const fileName = lines.at(-1);
    if (!fileName) {
        throw new AppError({ code: 'pack_failed', capability: 'remote-install', message: 'npm pack produced no tarball name' });
    }
    const tarballPath = path.isAbsolute(fileName) ? fileName : path.join(outDir, fileName);
    if (!fs.existsSync(tarballPath)) {
        throw new AppError({
            code: 'pack_failed',
            capability: 'remote-install',
            message: `packed tarball missing at \`${tarballPath}\``,
            path: tarballPath,
        });
    }
    return {
        mode: 'workspace-pack',
        packageName: pkg.name,
        version: pkg.version,
        tarballPath,
        note:
            pkg.version === '0.0.0'
                ? 'Package is still `0.0.0` (unpublished). Remote install uses the local `npm pack` tarball.'
                : `Packed local workspace version ${pkg.version}.`,
    };
}

/** Resolve password from an environment variable name. */
export function passwordFromEnv(envName: string | undefined): string {
    const key = envName?.trim();
    if (!key) {
        throw new AppError({
            code: 'invalid_argument',
            message: '`--password-env` is required (password must not be passed as a CLI argument)',
        });
    }
    const value = process.env[key];
    if (!value) {
        throw new AppError({
            code: 'invalid_argument',
            message: `environment variable \`${key}\` is empty or unset`,
        });
    }
    return value;
}

/** Expand a hosts list from repeated `--host` and optional file (one host per line). */
export function collectHosts(hosts: string[], hostsFile?: string): string[] {
    const out: string[] = [];
    for (const host of hosts) {
        const trimmed = host.trim();
        if (trimmed) out.push(trimmed);
    }
    if (hostsFile?.trim()) {
        const text = fs.readFileSync(path.resolve(hostsFile.trim()), 'utf8');
        for (const line of text.split(/\r?\n/)) {
            const trimmed = line.trim();
            if (!trimmed || trimmed.startsWith('#')) continue;
            out.push(trimmed);
        }
    }
    return [...new Set(out)];
}

export function tempJobNote(): string {
    return path.join(os.tmpdir(), 'open-panel-remote');
}

function copyPackTree(srcDir: string, destDir: string): void {
    // Only ship what `files` would include, plus package.json / lock metadata.
    const entries = [
        'package.json',
        'Readme.md',
        'License.md',
        'postinstall.mjs',
        'dist',
        'locales',
    ];
    for (const entry of entries) {
        const from = path.join(srcDir, entry);
        if (!fs.existsSync(from)) continue;
        const to = path.join(destDir, entry);
        fs.cpSync(from, to, { recursive: true });
    }
}
