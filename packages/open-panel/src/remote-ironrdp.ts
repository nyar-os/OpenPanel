/**
 * Node façade over the Rust `open-panel-rdp` protocol client (IronRDP).
 *
 * Never shells out to `mstsc`, FreeRDP, or `wfreerdp`.
 */
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

import { AppError } from './errors.js';
import { workspaceRoot } from './native.js';

export interface RdpHandshakeInput {
    host: string;
    user: string;
    passwordEnv: string;
    port?: number;
    domain?: string;
}

export interface RdpHandshakeReport {
    ok: boolean;
    host?: string;
    port?: number;
    remoteAddr?: string;
    localAddr?: string;
    desktopWidth?: number;
    desktopHeight?: number;
    ioChannelId?: number;
    userChannelId?: number;
    layers?: string[];
    notes?: string[];
    error?: string;
    stack?: string;
}

function candidateBins(): string[] {
    const root = workspaceRoot();
    const exe = process.platform === 'win32' ? 'open-panel-rdp.exe' : 'open-panel-rdp';
    return [
        process.env.OPEN_PANEL_RDP_BIN?.trim(),
        path.join(root, 'target', 'release', exe),
        path.join(root, 'target', 'debug', exe),
    ].filter((value): value is string => Boolean(value && value.trim()));
}

/** Resolve the `open-panel-rdp` binary built from `crates/rdp`. */
export function resolveRdpBin(): string {
    for (const candidate of candidateBins()) {
        if (fs.existsSync(candidate)) return candidate;
    }
    throw new AppError({
        code: 'dependency_blocked',
        capability: 'remote-rdp',
        message:
            'Rust RDP client binary not found. Build it with `cargo build -p open-panel-rdp` ' +
            'or set `OPEN_PANEL_RDP_BIN` to the `open-panel-rdp` executable.',
    });
}

/** TCP probe via the Rust client. */
export function rdpProbe(host: string, port = 3389): RdpHandshakeReport {
    const bin = resolveRdpBin();
    const result = spawnSync(bin, ['probe', '--host', host, '--port', String(port)], {
        encoding: 'utf8',
        windowsHide: true,
    });
    return parseJson(result.stdout, result.stderr, result.status);
}

/** Full IronRDP handshake through CredSSP / connection finalization. */
export function rdpHandshake(input: RdpHandshakeInput): RdpHandshakeReport {
    const host = input.host.trim();
    const user = input.user.trim();
    if (!host) throw new AppError({ code: 'invalid_argument', message: '`--host` is required' });
    if (!user) throw new AppError({ code: 'invalid_argument', message: '`--user` is required' });
    if (!input.passwordEnv.trim()) {
        throw new AppError({
            code: 'invalid_argument',
            message: '`--password-env` is required (password must not be passed as a CLI argument)',
        });
    }
    if (!process.env[input.passwordEnv]) {
        throw new AppError({
            code: 'invalid_argument',
            message: `environment variable \`${input.passwordEnv}\` is empty or unset`,
        });
    }

    const bin = resolveRdpBin();
    const args = [
        'handshake',
        '--host',
        host,
        '--user',
        user,
        '--password-env',
        input.passwordEnv,
        '--port',
        String(input.port ?? 3389),
    ];
    if (input.domain?.trim()) args.push('--domain', input.domain.trim());

    const result = spawnSync(bin, args, {
        encoding: 'utf8',
        windowsHide: true,
        env: process.env,
    });
    return parseJson(result.stdout, result.stderr, result.status);
}

function parseJson(stdout: string | null, stderr: string | null, status: number | null): RdpHandshakeReport {
    const text = (stdout ?? '').trim();
    if (text) {
        try {
            return JSON.parse(text) as RdpHandshakeReport;
        }
        catch (error) {
            throw new AppError({
                code: 'remote_rdp_invalid_json',
                capability: 'remote-rdp',
                message: `open-panel-rdp returned non-JSON: ${error instanceof Error ? error.message : String(error)}`,
            });
        }
    }
    throw new AppError({
        code: 'remote_rdp_failed',
        capability: 'remote-rdp',
        message: stderr?.trim() || `open-panel-rdp exited with status ${status ?? 'null'}`,
    });
}
