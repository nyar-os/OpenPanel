/**
 * Windows RDP remote execution via FreeRDP drive redirect + one-shot shell.
 *
 * Flow:
 * 1. Stage a local job directory with `run.ps1` / `job.ps1` and payload files
 * 2. FreeRDP connects with `/drive:op,<stageDir>` and starts PowerShell that
 *    runs `\\tsclient\op\run.ps1`
 * 3. The remote script writes `result.json` and `done.flag` back into the
 *    redirected drive (visible locally in the stage dir)
 * 4. Controller waits for the sentinel, then stops FreeRDP
 *
 * This is the primary Windows install channel. OpenSSH on the target is not
 * required and must not block delivery when only 3389 is open.
 */
import { spawn, type ChildProcess } from 'node:child_process';
import fs from 'node:fs';
import path from 'node:path';

import { AppError } from './errors.js';
import { ensureConfigDir } from './paths.js';
import { ensureFreeRdp } from './remote-freerdp.js';

export interface RdpCredentials {
    host: string;
    username: string;
    password: string;
    port?: number;
}

export interface RdpJobOptions extends RdpCredentials {
    /** Files to place in the stage directory (name → content or absolute path to copy). */
    files?: Record<string, string | { copyFrom: string }>;
    /** PowerShell body executed on the remote host. Should write result.json when useful. */
    script: string;
    /** Max wait for remote completion. */
    timeoutMs?: number;
    /** Poll interval for the local sentinel file. */
    pollMs?: number;
}

export interface RdpJobResult {
    channel: 'rdp';
    host: string;
    stageDir: string;
    exitCode: number | null;
    freerdpPath: string;
    result: Record<string, unknown>;
    stdoutTail: string;
    stderrTail: string;
}

function stageRoot(): string {
    return path.join(ensureConfigDir(), 'rdp-jobs');
}

function writeStage(files: Record<string, string | { copyFrom: string }>, script: string): string {
    const dir = path.join(stageRoot(), `${Date.now()}-${process.pid}`);
    fs.mkdirSync(dir, { recursive: true });
    for (const [name, value] of Object.entries(files)) {
        const target = path.join(dir, name);
        if (typeof value === 'string') fs.writeFileSync(target, value, 'utf8');
        else fs.copyFileSync(value.copyFrom, target);
    }

    const wrapper = [
        '$ErrorActionPreference = "Stop"',
        '$opRoot = "\\\\tsclient\\op"',
        'if (-not (Test-Path -LiteralPath $opRoot)) {',
        '  @{ ok = $false; error = "tsclient drive op missing" } | ConvertTo-Json -Compress | Set-Content -LiteralPath (Join-Path $env:TEMP "open-panel-rdp-result.json") -Encoding utf8',
        '  exit 1',
        '}',
        '$resultPath = Join-Path $opRoot "result.json"',
        '$donePath = Join-Path $opRoot "done.flag"',
        'try {',
        '  & ([ScriptBlock]::Create((Get-Content -LiteralPath (Join-Path $opRoot "job.ps1") -Raw)))',
        '  if (-not (Test-Path -LiteralPath $resultPath)) {',
        '    @{ ok = $true; message = "job finished without result.json" } | ConvertTo-Json -Compress | Set-Content -LiteralPath $resultPath -Encoding utf8',
        '  }',
        '}',
        'catch {',
        '  @{ ok = $false; error = $_.Exception.Message } | ConvertTo-Json -Compress | Set-Content -LiteralPath $resultPath -Encoding utf8',
        '}',
        'finally {',
        '  "done" | Set-Content -LiteralPath $donePath -Encoding utf8',
        '  Start-Sleep -Seconds 2',
        '  exit 0',
        '}',
        '',
    ].join('\n');

    fs.writeFileSync(path.join(dir, 'job.ps1'), `${script.trim()}\n`, 'utf8');
    fs.writeFileSync(path.join(dir, 'run.ps1'), wrapper, 'utf8');
    return dir;
}

function tail(text: string, max = 4000): string {
    if (text.length <= max) return text;
    return text.slice(text.length - max);
}

function sleep(ms: number): Promise<void> {
    return new Promise((resolve) => setTimeout(resolve, ms));
}

async function waitForDone(stageDir: string, timeoutMs: number, pollMs: number): Promise<Record<string, unknown> | null> {
    const donePath = path.join(stageDir, 'done.flag');
    const resultPath = path.join(stageDir, 'result.json');
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
        if (fs.existsSync(donePath) || fs.existsSync(resultPath)) {
            if (fs.existsSync(resultPath)) {
                try {
                    return JSON.parse(fs.readFileSync(resultPath, 'utf8')) as Record<string, unknown>;
                }
                catch (error) {
                    return {
                        ok: false,
                        error: `result.json parse failed: ${error instanceof Error ? error.message : String(error)}`,
                    };
                }
            }
            return { ok: true, message: 'done.flag present without result.json' };
        }
        await sleep(pollMs);
    }
    return null;
}

function stopChild(child: ChildProcess): void {
    if (child.killed) return;
    child.kill();
    if (process.platform === 'win32' && child.pid) {
        spawn('taskkill', ['/PID', String(child.pid), '/T', '/F'], { windowsHide: true, stdio: 'ignore' });
    }
}

/** Execute a PowerShell job on a Windows host over RDP. */
export async function runRdpJob(options: RdpJobOptions): Promise<RdpJobResult> {
    const host = options.host.trim();
    const username = options.username.trim();
    const password = options.password;
    if (!host) throw new AppError({ code: 'invalid_argument', message: '`--host` is required' });
    if (!username) throw new AppError({ code: 'invalid_argument', message: '`--user` is required' });
    if (!password) {
        throw new AppError({
            code: 'invalid_argument',
            message: 'password is required via `--password-env` (never pass secrets on the CLI argv)',
        });
    }

    const freerdpPath = await ensureFreeRdp();
    const stageDir = writeStage(options.files ?? {}, options.script);
    const timeoutMs = options.timeoutMs ?? 10 * 60 * 1000;
    const pollMs = options.pollMs ?? 1500;
    const port = options.port ?? 3389;

    for (const name of ['done.flag', 'result.json']) {
        const file = path.join(stageDir, name);
        if (fs.existsSync(file)) fs.unlinkSync(file);
    }

    // Prefer RemoteApp-style launch. If the server ignores /app, FreeRDP still
    // redirects the drive so a later interactive session can run run.ps1, but
    // the automated path expects /app or /shell to start the script.
    const args = [
        `/v:${host}:${port}`,
        `/u:${username}`,
        `/p:${password}`,
        '/cert:ignore',
        '/sec:nla',
        '/network:auto',
        '/timeout:20000',
        `/drive:op,${stageDir}`,
        '/app:powershell.exe',
        '/app-cmd:-NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -File \\\\tsclient\\op\\run.ps1',
        '-wallpaper',
        '-themes',
        '/size:800x600',
    ];

    let child: ChildProcess;
    try {
        child = spawn(freerdpPath, args, {
            windowsHide: true,
            stdio: ['ignore', 'pipe', 'pipe'],
            env: { ...process.env },
        });
    }
    catch (error) {
        throw new AppError({
            code: 'dependency_blocked',
            capability: 'remote-rdp',
            message: `failed to spawn FreeRDP: ${error instanceof Error ? error.message : String(error)}`,
            path: freerdpPath,
        });
    }

    let stdout = '';
    let stderr = '';
    child.stdout?.on('data', (chunk) => {
        stdout += String(chunk);
    });
    child.stderr?.on('data', (chunk) => {
        stderr += String(chunk);
    });

    const resultPromise = waitForDone(stageDir, timeoutMs, pollMs);
    const exitPromise = new Promise<number | null>((resolve) => {
        child.on('error', (error) => {
            stderr += `\nspawn error: ${error.message}`;
            resolve(null);
        });
        child.on('exit', (code) => resolve(code));
    });

    const result = await resultPromise;
    stopChild(child);
    const exitCode = await Promise.race([exitPromise, sleep(5000).then(() => null)]);

    fs.writeFileSync(path.join(stageDir, 'freerdp.stdout.log'), stdout, 'utf8');
    fs.writeFileSync(path.join(stageDir, 'freerdp.stderr.log'), stderr, 'utf8');

    if (!result) {
        throw new AppError({
            code: 'remote_timeout',
            capability: 'remote-rdp',
            message:
                `RDP job timed out after ${timeoutMs}ms waiting for result.json under \`${stageDir}\`. ` +
                'Confirm FreeRDP can open the session and that drive redirection is allowed.',
            path: stageDir,
        });
    }

    if (result.ok === false) {
        throw new AppError({
            code: 'remote_exec_failed',
            capability: 'remote-rdp',
            message: String(result.error ?? result.message ?? 'remote job reported failure'),
            path: stageDir,
        });
    }

    return {
        channel: 'rdp',
        host,
        stageDir,
        exitCode,
        freerdpPath,
        result,
        stdoutTail: tail(stdout),
        stderrTail: tail(stderr),
    };
}
