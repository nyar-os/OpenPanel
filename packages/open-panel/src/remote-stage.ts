/**
 * Ephemeral HTTP staging for RDP-only Windows install.
 *
 * MS-RDPBCGR caps AlternateShell at 512 bytes, so the IronRDP path cannot
 * embed a tarball. Controllers serve a short install script + packed tarball
 * over HTTP that the remote fetches via a tiny `powershell -c "iwr …|iex"`
 * bootstrap. Future RDPDR drive redirect can replace this when the remote
 * cannot reach the controller.
 */
import http from 'node:http';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import { networkInterfaces } from 'node:os';

import { DEFAULT_WEBUI_PORT } from './defaults.js';
import { AppError } from './errors.js';
import type { PackResult } from './remote-pack.js';

export interface StageServer {
    baseUrl: string;
    scriptUrl: string;
    tarballUrl: string;
    tarballName: string;
    listenPort: number;
    close: () => Promise<void>;
}

export interface StageOptions {
    pack: PackResult;
    /** Preferred public/base URL override (no trailing path). */
    baseUrl?: string;
    /** Server listen port (0 = ephemeral). */
    port?: number;
    serverPort?: number;
    skipStart?: boolean;
}

function buildInstallScript(opts: {
    tarballName: string;
    serverPort: number;
    skipStart: boolean;
}): string {
    const startBlock = opts.skipStart
        ? `
$startSkipped = $true
`
        : `
$startSkipped = $false
$serverCmd = Get-Command open-panel -ErrorAction SilentlyContinue
if (-not $serverCmd) { throw "open-panel not found on PATH after install" }
$argList = @("server","--host","0.0.0.0","--port","${opts.serverPort}")
Start-Process -FilePath $serverCmd.Source -ArgumentList $argList -WindowStyle Hidden | Out-Null
Start-Sleep -Seconds 2
`;

    return `# OpenPanel RDP bootstrap (generated). Runs on the remote host only.
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
$stageRoot = Join-Path $env:TEMP ("open-panel-rdp-" + [guid]::NewGuid().ToString("n"))
New-Item -ItemType Directory -Force -Path $stageRoot | Out-Null
$tarball = Join-Path $stageRoot "${opts.tarballName}"
$base = $PSScriptRoot
if (-not $base) { $base = "." }
# Controllers rewrite __OP_BASE__ to the live staging origin.
$origin = "__OP_BASE__"
Invoke-WebRequest -UseBasicParsing -Uri ($origin.TrimEnd("/") + "/${opts.tarballName}") -OutFile $tarball
if (-not (Get-Command npm -ErrorAction SilentlyContinue)) {
  throw "npm is required on the remote host for OpenPanel install (Node.js LTS)"
}
npm install -g --omit=dev $tarball
$panelCmd = Get-Command open-panel -ErrorAction SilentlyContinue
if (-not $panelCmd) { throw "open-panel not found on PATH after npm install -g" }
# Same install path as local / npx / postinstall: write administrator + strong password.
& $panelCmd.Source install --port ${opts.serverPort}
${startBlock}
$result = [ordered]@{
  ok = $true
  tarball = $tarball
  stageRoot = $stageRoot
  serverPort = ${opts.serverPort}
  loginUrl = ("http://0.0.0.0:${opts.serverPort}/login")
  startSkipped = $startSkipped
  at = (Get-Date).ToString("o")
}
$result | ConvertTo-Json -Compress | Set-Content -Encoding utf8 (Join-Path $env:TEMP "open-panel-rdp-result.json")
`;
}

function candidateBaseUrls(port: number): string[] {
    const out: string[] = [];
    const nets = networkInterfaces();
    for (const entries of Object.values(nets)) {
        if (!entries) continue;
        for (const entry of entries) {
            if (entry.internal) continue;
            if (String(entry.family) !== 'IPv4' && String(entry.family) !== '4') continue;
            out.push(`http://${entry.address}:${port}`);
        }
    }
    out.push(`http://127.0.0.1:${port}`);
    return [...new Set(out)];
}

/** Build the short AlternateShell bootstrap (must stay under the 512-byte wire cap). */
export function buildAlternateShellBootstrap(scriptUrl: string): string {
    // Prefer a compact ASCII one-liner. Keep well under 512 bytes.
    const url = scriptUrl.trim();
    if (!url) {
        throw new AppError({ code: 'invalid_argument', message: 'script URL is required for RDP bootstrap' });
    }
    const shell =
        `powershell.exe -NoProfile -ExecutionPolicy Bypass -WindowStyle Hidden -Command "iwr -UseBasicParsing '${url}'|iex"`;
    if (Buffer.byteLength(shell, 'utf8') + 1 > 510) {
        throw new AppError({
            code: 'invalid_argument',
            message: `alternate_shell bootstrap exceeds MS-RDPBCGR 512-byte limit (${shell.length} chars)`,
        });
    }
    return shell;
}

/** Serve pack + install script until `close()` is called. */
export async function startPackStageServer(options: StageOptions): Promise<StageServer> {
    const tarballPath = options.pack.tarballPath;
    if (!fs.existsSync(tarballPath)) {
        throw new AppError({
            code: 'pack_failed',
            capability: 'remote-install',
            message: `packed tarball missing at \`${tarballPath}\``,
            path: tarballPath,
        });
    }
    const tarballName = path.basename(tarballPath);
    const tarballBytes = fs.readFileSync(tarballPath);
    const serverPort = options.serverPort ?? DEFAULT_WEBUI_PORT;
    const skipStart = Boolean(options.skipStart);
    const scriptTemplate = buildInstallScript({ tarballName, serverPort, skipStart });

    let chosenBase = options.baseUrl?.replace(/\/$/, '') ?? '';
    let scriptBody = '';

    const server = http.createServer((req, res) => {
        const url = new URL(req.url ?? '/', 'http://127.0.0.1');
        if (url.pathname === `/${tarballName}` || url.pathname === '/package.tgz') {
            res.writeHead(200, {
                'content-type': 'application/gzip',
                'content-length': tarballBytes.length,
                'cache-control': 'no-store',
            });
            res.end(tarballBytes);
            return;
        }
        if (url.pathname === '/i.ps1' || url.pathname === '/install.ps1') {
            res.writeHead(200, {
                'content-type': 'text/plain; charset=utf-8',
                'content-length': Buffer.byteLength(scriptBody),
                'cache-control': 'no-store',
            });
            res.end(scriptBody);
            return;
        }
        if (url.pathname === '/health') {
            res.writeHead(200, { 'content-type': 'application/json' });
            res.end(JSON.stringify({ ok: true, tarballName }));
            return;
        }
        res.writeHead(404).end('not found');
    });

    const listenPort = await new Promise<number>((resolve, reject) => {
        server.once('error', reject);
        server.listen(options.port ?? 0, '0.0.0.0', () => {
            const address = server.address();
            if (!address || typeof address === 'string') {
                reject(new Error('stage server has no TCP address'));
                return;
            }
            resolve(address.port);
        });
    });

    if (!chosenBase) {
        const envBase = process.env.OPEN_PANEL_PACK_BASE_URL?.trim();
        if (envBase) {
            chosenBase = envBase.replace(/\/$/, '').replace(':0', `:${listenPort}`);
        }
        else {
            const candidates = candidateBaseUrls(listenPort);
            // Prefer non-loopback. Callers can override with OPEN_PANEL_PACK_BASE_URL
            // when the remote cannot route to private controller addresses.
            chosenBase = candidates.find((item) => !item.includes('127.0.0.1')) ?? candidates[0]!;
        }
    }
    else if (chosenBase.includes(':0')) {
        chosenBase = chosenBase.replace(':0', `:${listenPort}`);
    }

    scriptBody = scriptTemplate.replaceAll('__OP_BASE__', chosenBase);
    const scriptUrl = `${chosenBase}/i.ps1`;
    const tarballUrl = `${chosenBase}/${tarballName}`;

    return {
        baseUrl: chosenBase,
        scriptUrl,
        tarballUrl,
        tarballName,
        listenPort,
        close: () =>
            new Promise<void>((resolve, reject) => {
                server.close((error) => (error ? reject(error) : resolve()));
            }),
    };
}

/** Probe whether a TCP port accepts connections (used after remote server start). */
export async function waitForTcp(host: string, port: number, timeoutMs: number): Promise<boolean> {
    const net = await import('node:net');
    const deadline = Date.now() + timeoutMs;
    while (Date.now() < deadline) {
        const open = await new Promise<boolean>((resolve) => {
            const socket = net.connect({ host, port, timeout: 2_000 }, () => {
                socket.destroy();
                resolve(true);
            });
            socket.on('error', () => {
                socket.destroy();
                resolve(false);
            });
            socket.on('timeout', () => {
                socket.destroy();
                resolve(false);
            });
        });
        if (open) return true;
        await new Promise((resolve) => setTimeout(resolve, 1_500));
    }
    return false;
}

export function defaultWorkDir(): string {
    return 'C:\\Windows\\Temp';
}

export function hostnameHint(): string {
    return os.hostname();
}
