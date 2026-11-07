/**
 * Node façade over the Rust `open-panel-rdp` protocol client (IronRDP).
 *
 * Never shells out to `mstsc`, FreeRDP, or `wfreerdp`.
 */
import { spawnSync } from 'node:child_process';
import fs from 'node:fs';
import os from 'node:os';
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

export interface RdpExecInput extends RdpHandshakeInput {
    /** Full AlternateShell string (MS-RDPBCGR ≤512 bytes including NUL). */
    shell: string;
    workDir?: string;
    sessionTimeoutSecs?: number;
    /** Local directory redirected as `\\tsclient\OP` via IronRDP RDPDR. */
    rdpdrDir?: string;
    /**
     * Preparatory `logoff` before AlternateShell (default off).
     * Pass `true` for `--force-fresh-logon`.
     */
    forceFreshLogon?: boolean;
    /**
     * Win+R scancode inject (default off — ActiveStage graphics can abort).
     * Pass `true` for `--inject-run`.
     */
    injectRun?: boolean;
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

export interface RdpExecReport extends RdpHandshakeReport {
    alternateShell?: string;
    workDir?: string;
    endReason?: string;
    elapsedMs?: number;
    responseFrames?: number;
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

/**
 * Return a runnable path that will not lock the Cargo target exe.
 *
 * Concurrent `cargo build -p open-panel-rdp` fails with Windows os error 5 when
 * another agent still holds `target/debug/open-panel-rdp.exe`. Lab verification
 * also needs uniquely-named copies. Always copy out of the build dir (unless
 * `OPEN_PANEL_RDP_BIN` already points outside `target/`).
 */
export function resolveRunnableRdpBin(): string {
    const source = resolveRdpBin();
    const override = process.env.OPEN_PANEL_RDP_BIN?.trim();
    if (override && fs.existsSync(override)) {
        const normalized = path.resolve(override).replace(/\\/g, '/').toLowerCase();
        // Explicit lab copies (Temp / unique names) are used as-is.
        if (!normalized.includes('/target/debug/') && !normalized.includes('/target/release/')) {
            return override;
        }
    }

    const ext = process.platform === 'win32' ? '.exe' : '';
    const stamp = `${process.pid}-${Date.now().toString(36)}-${Math.random().toString(36).slice(2, 8)}`;
    const dest = path.join(os.tmpdir(), `open-panel-rdp-run-${stamp}${ext}`);
    try {
        fs.copyFileSync(source, dest);
        if (process.platform !== 'win32') {
            fs.chmodSync(dest, 0o755);
        }
    }
    catch (error) {
        throw new AppError({
            code: 'dependency_blocked',
            capability: 'remote-rdp',
            message:
                `failed to copy open-panel-rdp to a runnable path (${dest}): ` +
                (error instanceof Error ? error.message : String(error)),
        });
    }
    return dest;
}

function assertAuth(input: RdpHandshakeInput): { host: string, user: string } {
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
    return { host, user };
}

/** TCP probe via the Rust client. */
export function rdpProbe(host: string, port = 3389): RdpHandshakeReport {
    const bin = resolveRunnableRdpBin();
    const result = spawnSync(bin, ['probe', '--host', host, '--port', String(port)], {
        encoding: 'utf8',
        windowsHide: true,
    });
    return parseJson(result.stdout, result.stderr, result.status);
}

/** Full IronRDP handshake through CredSSP / connection finalization. */
export function rdpHandshake(input: RdpHandshakeInput): RdpHandshakeReport {
    const { host, user } = assertAuth(input);
    const bin = resolveRunnableRdpBin();
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

/**
 * Connect with `Config::alternate_shell` and drive the ActiveStage until the
 * remote process ends or the session times out.
 */
export function rdpExec(input: RdpExecInput): RdpExecReport {
    const { host, user } = assertAuth(input);
    const shell = input.shell.trim();
    if (!shell) {
        throw new AppError({ code: 'invalid_argument', message: '`--shell` / alternate_shell is required' });
    }

    const bin = resolveRunnableRdpBin();
    // Pass the shell via env so Windows argv quoting cannot strip spaces/quotes
    // from EncodedCommand / powershell -Command payloads.
    const shellEnv = 'OPEN_PANEL_RDP_SHELL';
    const args = [
        'exec',
        '--host',
        host,
        '--user',
        user,
        '--password-env',
        input.passwordEnv,
        '--port',
        String(input.port ?? 3389),
        '--shell-env',
        shellEnv,
        '--session-timeout-secs',
        String(input.sessionTimeoutSecs ?? 180),
    ];
    if (input.workDir?.trim()) args.push('--work-dir', input.workDir.trim());
    if (input.domain?.trim()) args.push('--domain', input.domain.trim());
    if (input.rdpdrDir?.trim()) args.push('--rdpdr-dir', input.rdpdrDir.trim());
    // Explicit polarity (default OFF). Undefined must not re-enable lab-abort paths.
    args.push(input.forceFreshLogon ? '--force-fresh-logon' : '--no-force-fresh-logon');
    args.push(input.injectRun ? '--inject-run' : '--no-inject-run');

    const result = spawnSync(bin, args, {
        encoding: 'utf8',
        windowsHide: true,
        env: { ...process.env, [shellEnv]: shell },
        // Install sessions can outlive the default spawn sync limits on busy hosts.
        // force_fresh_logon adds a preparatory logoff pass before the shell session.
        timeout: ((input.sessionTimeoutSecs ?? 180) + 90) * 1000,
    });
    return parseJson(result.stdout, result.stderr, result.status) as RdpExecReport;
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

    // Prefer the last JSON object printed on stderr (panic hook / flush path).
    const errText = (stderr ?? '').trim();
    if (errText) {
        const jsonLine = [...errText.split(/\r?\n/)]
            .reverse()
            .find((line) => line.trimStart().startsWith('{') && line.includes('"ok"'));
        if (jsonLine) {
            try {
                return JSON.parse(jsonLine) as RdpHandshakeReport;
            }
            catch {
                // fall through to structured empty-output error
            }
        }
    }

    const signedStatus = status == null ? null : status > 0x7fffffff ? status - 0x100000000 : status;
    const crumbs = errText
        .split(/\r?\n/)
        .filter((line) => line.includes('[open-panel-rdp]'))
        .slice(-8)
        .join(' | ');
    throw new AppError({
        code: 'remote_rdp_failed',
        capability: 'remote-rdp',
        message:
            crumbs
                ? `open-panel-rdp exited with status ${signedStatus ?? 'null'} and empty stdout; last crumbs: ${crumbs}`
                : `open-panel-rdp exited with status ${signedStatus ?? 'null'} and empty stdout/stderr (process abort or external kill)`,
    });
}
