/**
 * Local install step shared by `open-panel install`, `npx @wae/open-panel install`,
 * and the package `postinstall` hook.
 *
 * Writes WebUI username / password / default port under the per-user config dir.
 * `open-panel server` only reads that file — it never generates secrets.
 */
import os from 'node:os';

import {
    credentialsExist,
    credentialsPath,
    installCredentials,
    type InstallCredentialsResult,
    type WebuiCredentials,
} from './credentials.js';
import { DEFAULT_WEBUI_PORT, WEBUI_USERNAME, webuiLoginUrl } from './defaults.js';
import { ensureConfigDir, resolveConfigDir } from './paths.js';

export interface RunInstallOptions {
    force?: boolean;
    port?: number;
    /** Host shown in the printed login URL (default: primary non-internal IPv4 or 127.0.0.1). */
    advertiseHost?: string;
    /**
     * When true (postinstall / npx lifecycle), reuse existing credentials quietly
     * and avoid failing the npm lifecycle if the environment is incomplete.
     */
    fromPostinstall?: boolean;
}

export interface InstallReport {
    ok: boolean;
    created: boolean;
    reused: boolean;
    path: string;
    configDir: string;
    username: string;
    port: number;
    loginUrl: string;
    /** Present only when a new password was generated in this run. */
    password?: string;
    note: string;
}

function pickAdvertiseHost(explicit?: string): string {
    const trimmed = explicit?.trim();
    if (trimmed) return trimmed;
    const nets = os.networkInterfaces();
    for (const entries of Object.values(nets)) {
        if (!entries) continue;
        for (const entry of entries) {
            if (String(entry.family) !== 'IPv4' && String(entry.family) !== '4') continue;
            if (entry.internal) continue;
            if (entry.address) return entry.address;
        }
    }
    return '127.0.0.1';
}

function toReport(
    result: InstallCredentialsResult,
    advertiseHost: string,
    created: boolean,
): InstallReport {
    const { credentials, path: file } = result;
    const loginUrl = webuiLoginUrl(advertiseHost, credentials.port);
    return {
        ok: true,
        created,
        reused: !created,
        path: file,
        configDir: resolveConfigDir(),
        username: credentials.username,
        port: credentials.port,
        loginUrl,
        ...(created ? { password: credentials.password } : {}),
        note: created
            ? 'Credentials created. Store the password now — it is shown only once.'
            : 'Credentials already present. Re-run with `--force` to rotate the password.',
    };
}

/**
 * Run the local OpenPanel install configuration step.
 *
 * Idempotent by default: existing credentials are reused unless `force` is set.
 */
export function runInstall(options: RunInstallOptions = {}): InstallReport {
    ensureConfigDir();
    const advertiseHost = pickAdvertiseHost(options.advertiseHost);
    const port = options.port ?? DEFAULT_WEBUI_PORT;

    if (options.fromPostinstall && credentialsExist() && !options.force) {
        const loaded = installCredentials({ force: false, port });
        return toReport(loaded, advertiseHost, false);
    }

    const result = installCredentials({
        force: options.force === true,
        port,
    });
    return toReport(result, advertiseHost, result.created);
}

/** Human summary for CLI stdout (never prints a password unless freshly created). */
export function formatInstallReport(report: InstallReport): string {
    const lines = [
        `config: ${report.configDir}`,
        `credentials: ${report.path}`,
        `user: ${report.username}`,
        `port: ${report.port}`,
        `login: ${report.loginUrl}`,
        report.note,
    ];
    if (report.created && report.password) {
        lines.push(`password: ${report.password}`);
    }
    return lines.join('\n');
}

export {
    credentialsExist,
    credentialsPath,
    DEFAULT_WEBUI_PORT,
    WEBUI_USERNAME,
    type WebuiCredentials,
};
