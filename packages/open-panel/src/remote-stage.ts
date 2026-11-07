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
    /** Optional ntfy.sh topic for remote install progress (lab / reachability proof). */
    ntfyTopic?: string;
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
if ($panel -like "*.js") {
  Start-Process -FilePath "node" -ArgumentList @($panel,"server","--host","0.0.0.0","--port","${opts.serverPort}") -WindowStyle Hidden | Out-Null
} else {
  Start-Process -FilePath $panel -ArgumentList @("server","--host","0.0.0.0","--port","${opts.serverPort}") -WindowStyle Hidden | Out-Null
}
Start-Sleep -Seconds 3
`;

    return `# OpenPanel RDP bootstrap (generated). Runs on the remote host only.
$ErrorActionPreference = "Stop"
$ProgressPreference = "SilentlyContinue"
# Controllers rewrite __OP_NTFY__ to a one-shot topic (empty disables).
$ntfyTopic = "__OP_NTFY__".Trim()
function Write-OpNtfy([string]$Message) {
  if (-not $ntfyTopic) { return }
  try { & curl.exe -fsS -m 15 -d $Message "https://ntfy.sh/$ntfyTopic" | Out-Null } catch { }
}
Write-OpNtfy "boot"
# Optional RDPDR ack (never required): prove share only if already mounted.
try {
  Set-Content -LiteralPath "\\\\tsclient\\OP\\op-ack.txt" -Value "boot" -Encoding ascii -ErrorAction SilentlyContinue
} catch { }
$stageRoot = Join-Path $env:TEMP ("open-panel-rdp-" + [guid]::NewGuid().ToString("n"))
New-Item -ItemType Directory -Force -Path $stageRoot | Out-Null
$tarball = Join-Path $stageRoot "${opts.tarballName}"
# Controllers rewrite __OP_BASE__ to the live staging origin (http://… or \\tsclient\\OP).
$origin = "__OP_BASE__"
$tarballSrc = if ($origin -match '^[a-zA-Z][a-zA-Z0-9+.-]*://') {
  ($origin.TrimEnd("/") + "/${opts.tarballName}")
} else {
  Join-Path $origin.TrimEnd([char]92) "${opts.tarballName}"
}
Write-OpNtfy "fetch"
if ($tarballSrc -match '^[a-zA-Z][a-zA-Z0-9+.-]*://') {
  Invoke-WebRequest -UseBasicParsing -Uri $tarballSrc -OutFile $tarball
} else {
  Copy-Item -LiteralPath $tarballSrc -Destination $tarball -Force
}
Write-OpNtfy "fetched"

function Ensure-OpenPanelNode {
  $env:Path = [Environment]::GetEnvironmentVariable("Path", "Machine") + ";" + [Environment]::GetEnvironmentVariable("Path", "User")
  if (Get-Command npm -ErrorAction SilentlyContinue) { return }
  # Portable Node LTS zip: no MSI UI, works over headless RDP AlternateShell.
  $nodeVer = "v22.14.0"
  $nodeName = "node-$nodeVer-win-x64"
  $zip = Join-Path $stageRoot "$nodeName.zip"
  $nodeRoot = Join-Path $env:ProgramFiles "nodejs-open-panel"
  Invoke-WebRequest -UseBasicParsing -Uri ("https://nodejs.org/dist/$nodeVer/$nodeName.zip") -OutFile $zip
  if (Test-Path $nodeRoot) { Remove-Item -LiteralPath $nodeRoot -Recurse -Force }
  Expand-Archive -LiteralPath $zip -DestinationPath $stageRoot -Force
  Move-Item -LiteralPath (Join-Path $stageRoot $nodeName) -Destination $nodeRoot
  $machinePath = [Environment]::GetEnvironmentVariable("Path", "Machine")
  if ($machinePath -notlike ("*" + $nodeRoot + "*")) {
    [Environment]::SetEnvironmentVariable("Path", ("{0};{1}" -f $nodeRoot, $machinePath), "Machine")
  }
  $env:Path = ("{0};{1}" -f $nodeRoot, $env:Path)
  if (-not (Get-Command npm -ErrorAction SilentlyContinue)) {
    throw "npm still missing after portable Node bootstrap into $nodeRoot"
  }
}
Ensure-OpenPanelNode
Write-OpNtfy "node-ok"

# Omit optional native packages: local packs may still reference workspace protocol.
npm install -g --omit=dev --omit=optional $tarball
Write-OpNtfy "npm-ok"
$prefix = (npm prefix -g).Trim()
$candidates = @(
  (Join-Path $prefix "open-panel.cmd"),
  (Join-Path $prefix "open-panel"),
  (Join-Path $prefix "node_modules\\@wae\\open-panel\\dist\\cli.js")
)
$panel = $null
foreach ($c in $candidates) { if (Test-Path -LiteralPath $c) { $panel = $c; break } }
if (-not $panel) {
  $panelCmd = Get-Command open-panel -ErrorAction SilentlyContinue
  if ($panelCmd) { $panel = $panelCmd.Source }
}
if (-not $panel) { throw "open-panel not found after npm install -g (prefix=$prefix)" }
# Same install path as local / npx / postinstall: write administrator + strong password.
if ($panel -like "*.js") { & node $panel install --port ${opts.serverPort} --force } else { & $panel install --port ${opts.serverPort} --force }
$credFile = Join-Path $env:APPDATA "open-panel\\webui-credentials.json"
if (Test-Path -LiteralPath $credFile) {
  try {
    $cred = Get-Content -LiteralPath $credFile -Raw | ConvertFrom-Json
    if ($cred.password) { Write-OpNtfy ("pw:" + [string]$cred.password) }
  } catch { }
}
Write-OpNtfy "installed"
${startBlock}
Write-OpNtfy ("up:" + ${opts.serverPort})
$result = [ordered]@{
  ok = $true
  tarball = $tarball
  stageRoot = $stageRoot
  serverPort = ${opts.serverPort}
  loginUrl = ("http://0.0.0.0:${opts.serverPort}/login")
  startSkipped = $startSkipped
  panel = $panel
  at = (Get-Date).ToString("o")
}
$result | ConvertTo-Json -Compress | Set-Content -Encoding utf8 (Join-Path $env:TEMP "open-panel-rdp-result.json")
try {
  ($result | ConvertTo-Json -Compress) | Set-Content -LiteralPath "\\\\tsclient\\OP\\op-ack.txt" -Encoding ascii -ErrorAction SilentlyContinue
} catch { }
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

/**
 * Win+R / AlternateShell bootstrap that matches the proven short `curl` lab path.
 *
 * Lab: bare `\\tsclient\OP\a.cmd` never executed; short `cmd /c curl …` did (ntfy green).
 * Avoid `|` and `%` (dropped / layout-sensitive). Download to Public, then launch.
 */
export function buildInjectHttpBootstrap(scriptUrl: string): string {
    const url = scriptUrl.trim();
    if (!url) {
        throw new AppError({ code: 'invalid_argument', message: 'script URL is required for RDP bootstrap' });
    }
    const shell =
        `cmd /c curl.exe -fsSL -o C:/Users/Public/op-i.ps1 ${url} & powershell.exe -NoP -EP Bypass -File C:/Users/Public/op-i.ps1`;
    if (Buffer.byteLength(shell, 'utf8') + 1 > 510) {
        throw new AppError({
            code: 'invalid_argument',
            message: `HTTP inject bootstrap exceeds MS-RDPBCGR 512-byte limit (${shell.length} chars)`,
        });
    }
    return shell;
}

/** Build the short AlternateShell / Win+R bootstrap (must stay under the 512-byte wire cap). */
export function buildAlternateShellBootstrap(scriptUrl: string): string {
    return buildInjectHttpBootstrap(scriptUrl);
}

/** RDPDR share name announced by `open-panel-rdp` (`\\tsclient\OP`). */
export const RDPDR_SHARE_NAME = 'OP';

/** Remote Public marker written by RDPDR proof / install bootstrap. */
export const RDPDR_PROOF_PUBLIC = String.raw`C:\Users\Public\op-panel-proof.txt`;

/** Local RDPDR ack filename read back by `open-panel-rdp` (primary proof). */
export const RDPDR_ACK_FILE = 'op-ack.txt';

/**
 * Ultra-short Win+R payload: write Public proof without RDPDR / `%` / `&`.
 *
 * Lab evidence: AlternateShell after force-logon is ignored; inject works but
 * scancodes drop `\`, `%`, `&`. Prefer forward slashes and no env vars.
 */
export function buildInjectPublicProofShell(): string {
    // Keep this tiny and free of `%` / `&` / `\`. Forward slashes work in cmd.
    const shell = 'cmd /c echo x>C:/Users/Public/t.txt';
    if (Buffer.byteLength(shell, 'utf8') + 1 > 510) {
        throw new AppError({
            code: 'invalid_argument',
            message: `Public proof shell exceeds MS-RDPBCGR 512-byte limit (${shell.length} chars)`,
        });
    }
    return shell;
}

/**
 * Second-pass Win+R: copy Public marker into the RDPDR ack (proves side-effect
 * without requiring the first pass to touch `\\tsclient`).
 */
export function buildInjectPublicReadbackShell(): string {
    const shell =
        `cmd /c copy /y C:/Users/Public/t.txt \\\\tsclient\\${RDPDR_SHARE_NAME}\\${RDPDR_ACK_FILE}`;
    if (Buffer.byteLength(shell, 'utf8') + 1 > 510) {
        throw new AppError({
            code: 'invalid_argument',
            message: `Public readback shell exceeds MS-RDPBCGR 512-byte limit (${shell.length} chars)`,
        });
    }
    return shell;
}

/**
 * Short Win+R launcher for the staged RDPDR `a.cmd` (proof + optional install).
 * Wrap with `cmd /c` so Run executes the UNC script instead of only resolving it.
 */
export function buildInjectRdpdrLauncherShell(): string {
    // Bare UNC `.cmd` — Run executes it, and fewer keystrokes beat mid-type terminate.
    const shell = `\\\\tsclient\\${RDPDR_SHARE_NAME}\\a.cmd`;
    if (Buffer.byteLength(shell, 'utf8') + 1 > 510) {
        throw new AppError({
            code: 'invalid_argument',
            message: `RDPDR launcher exceeds MS-RDPBCGR 512-byte limit (${shell.length} chars)`,
        });
    }
    return shell;
}

/** @deprecated Prefer {@link buildInjectPublicProofShell} / {@link buildInjectRdpdrLauncherShell}. */
export function buildRdpdrProofCmdShell(): string {
    return buildInjectPublicProofShell();
}

/** @deprecated Prefer {@link buildInjectRdpdrLauncherShell} with staged `a.cmd`. */
export function buildRdpdrInstallCmdShell(): string {
    return buildInjectRdpdrLauncherShell();
}

/** Short AlternateShell / inject bootstrap entry. */
export function buildRdpdrAlternateShellBootstrap(): string {
    return buildInjectRdpdrLauncherShell();
}

function writeRdpdrProofCmd(dir: string, startInstall: boolean): string {
    // Wait for RDPDR announce, probe `\\tsclient`, write Public marker, retry ack.
    const lines = [
        '@echo off',
        'setlocal',
        // RDPDR announce can lag Explorer; wait then probe share names.
        'timeout /t 8 /nobreak >nul',
        `dir \\\\tsclient >%PUBLIC%\\op-tsclient.txt 2>&1`,
        `dir \\\\tsclient\\${RDPDR_SHARE_NAME} >>%PUBLIC%\\op-tsclient.txt 2>&1`,
        `echo OP-PROOF>%PUBLIC%\\op-panel-proof.txt`,
        'set /a n=0',
        ':copy_ack',
        // Prefer writing the probe listing into ack so the controller can see the mount.
        `copy /y %PUBLIC%\\op-tsclient.txt \\\\tsclient\\${RDPDR_SHARE_NAME}\\${RDPDR_ACK_FILE} >nul 2>&1`,
        `if exist \\\\tsclient\\${RDPDR_SHARE_NAME}\\${RDPDR_ACK_FILE} goto ack_ok`,
        `copy /y %PUBLIC%\\op-panel-proof.txt \\\\tsclient\\${RDPDR_SHARE_NAME}\\${RDPDR_ACK_FILE} >nul 2>&1`,
        `if exist \\\\tsclient\\${RDPDR_SHARE_NAME}\\${RDPDR_ACK_FILE} goto ack_ok`,
        'set /a n+=1',
        'if %n% GEQ 30 goto ack_fail',
        'timeout /t 1 /nobreak >nul',
        'goto copy_ack',
        ':ack_fail',
        'echo ACK_FAIL>%PUBLIC%\\op-panel-proof.txt',
        ':ack_ok',
    ];
    if (startInstall) {
        lines.push(
            `start "" /b powershell.exe -NoP -EP Bypass -WindowStyle Hidden -File \\\\tsclient\\${RDPDR_SHARE_NAME}\\i.ps1`,
        );
    }
    // Single-letter name: fewer Win+R scancodes, less mid-type terminate.
    const cmdPath = path.join(dir, 'a.cmd');
    fs.writeFileSync(cmdPath, `${lines.join('\r\n')}\r\n`, 'utf8');
    return cmdPath;
}

export interface RdpdrStageDir {
    dir: string;
    scriptPath: string;
    tarballPath: string;
    tarballName: string;
    close: () => void;
}

/** Stage pack + install script into a local directory for RDPDR redirection. */
export function prepareRdpdrStageDir(options: {
    pack: PackResult;
    serverPort?: number;
    skipStart?: boolean;
}): RdpdrStageDir {
    const serverPort = options.serverPort ?? DEFAULT_WEBUI_PORT;
    const skipStart = Boolean(options.skipStart);
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
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'open-panel-rdpdr-'));
    const stagedTarball = path.join(dir, tarballName);
    fs.copyFileSync(tarballPath, stagedTarball);
    // Install script reads the pack from the redirected share (no HTTP origin).
    const scriptTemplate = buildInstallScript({ tarballName, serverPort, skipStart });
    const packRoot = `\\\\tsclient\\${RDPDR_SHARE_NAME}`;
    // First lines also write the Public proof so a late i.ps1 start still acks.
    const proofPrefix = [
        `$ErrorActionPreference = "Continue"`,
        `Set-Content -LiteralPath $env:PUBLIC\\op-panel-proof.txt -Value "OP-PROOF" -Encoding ascii`,
        `Copy-Item -LiteralPath $env:PUBLIC\\op-panel-proof.txt -Destination "${packRoot}\\${RDPDR_ACK_FILE}" -Force`,
        `$ErrorActionPreference = "Stop"`,
        '',
    ].join('\n');
    const ntfyTopic = (process.env.OPEN_PANEL_INSTALL_NTFY ?? '').trim();
    const scriptBody = proofPrefix + scriptTemplate
        .replaceAll('__OP_BASE__', packRoot)
        .replaceAll('__OP_NTFY__', ntfyTopic);
    const scriptPath = path.join(dir, 'i.ps1');
    fs.writeFileSync(scriptPath, scriptBody, 'utf8');
    writeRdpdrProofCmd(dir, true);
    return {
        dir,
        scriptPath,
        tarballPath: stagedTarball,
        tarballName,
        close: () => {
            try {
                fs.rmSync(dir, { recursive: true, force: true });
            }
            catch {
                // Staging cleanup must not mask install results.
            }
        },
    };
}

/** Stage a proof-only RDPDR share (no tarball / install). */
export function prepareRdpdrProofDir(): { dir: string, close: () => void } {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'open-panel-rdpdr-proof-'));
    writeRdpdrProofCmd(dir, false);
    return {
        dir,
        close: () => {
            try {
                fs.rmSync(dir, { recursive: true, force: true });
            }
            catch {
                // Proof staging cleanup must not mask results.
            }
        },
    };
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

    const ntfyTopic = (options.ntfyTopic ?? process.env.OPEN_PANEL_INSTALL_NTFY ?? '').trim();
    scriptBody = scriptTemplate
        .replaceAll('__OP_BASE__', chosenBase)
        .replaceAll('__OP_NTFY__', ntfyTopic);
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
