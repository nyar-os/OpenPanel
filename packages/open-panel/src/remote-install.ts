/**
 * Remote install / upgrade of `@wae/open-panel`.
 *
 * Windows: RDP (FreeRDP drive redirect + one-shot PowerShell) is first-class.
 * Linux: SSH remains the primary channel (optional helper).
 * WinRM / OpenSSH-on-Windows are optional and never required for bare 3389 hosts.
 */
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';

import { AppError } from './errors.js';
import { upsertServer } from './managed-servers.js';
import { packOpenPanel, passwordFromEnv } from './remote-pack.js';
import { probeRemoteHost, type RemoteChannel, type RemoteProbeReport } from './remote-probe.js';
import { runRdpJob, type RdpJobResult } from './remote-rdp.js';

export interface RemoteInstallTarget {
    host: string;
    user: string;
    passwordEnv: string;
    /** Override probe recommendation. */
    channel?: RemoteChannel | 'auto';
    /** Target OS policy for channel selection. */
    targetOs?: 'windows' | 'linux' | 'auto';
    /** Listen port for the remote `open-panel server`. */
    serverPort?: number;
    /** Roster id to upsert after success. Defaults to host. */
    serverId?: string;
    /** Human label for the roster entry. */
    label?: string;
    /** Skip starting the remote server after install. */
    skipStart?: boolean;
    timeoutMs?: number;
}

export interface RemoteInstallHostResult {
    host: string;
    ok: boolean;
    channel?: RemoteChannel;
    probe?: RemoteProbeReport;
    pack?: ReturnType<typeof packOpenPanel>;
    rdp?: RdpJobResult;
    rosterId?: string;
    url?: string;
    error?: string;
}

export interface RemoteInstallReport {
    ok: boolean;
    pack: ReturnType<typeof packOpenPanel>;
    results: RemoteInstallHostResult[];
}

function buildInstallScript(opts: {
    tarballName: string;
    packageName: string;
    serverPort: number;
    startServer: boolean;
}): string {
    const { tarballName, packageName, serverPort, startServer } = opts;
    const startBlock = startServer
        ? `
Get-NetTCPConnection -LocalPort $port -ErrorAction SilentlyContinue | ForEach-Object {
  try { Stop-Process -Id $_.OwningProcess -Force -ErrorAction SilentlyContinue } catch {}
}
$log = Join-Path $installRoot "server.log"
$cmd = Get-Command open-panel.cmd -ErrorAction SilentlyContinue
if (-not $cmd) { $cmd = Get-Command open-panel -ErrorAction SilentlyContinue }
if (-not $cmd) { throw "open-panel CLI not found on PATH after npm install -g" }
Start-Process -FilePath $cmd.Source -ArgumentList @("server","--host","0.0.0.0","--port","$port") -WindowStyle Hidden -RedirectStandardOutput $log -RedirectStandardError $log | Out-Null
Start-Sleep -Seconds 3
$started = $true
`.trim()
        : '$started = $false';

    return `
$ErrorActionPreference = "Stop"
$opRoot = "\\\\tsclient\\op"
$tarball = Join-Path $opRoot "${tarballName}"
$installRoot = Join-Path $env:ProgramData "open-panel"
$nodeVer = $null
try { $nodeVer = (& node -v 2>$null) } catch {}

if (-not $nodeVer) {
  $tempMsi = Join-Path $env:TEMP "node-lts.msi"
  Invoke-WebRequest -Uri "https://nodejs.org/dist/v22.20.0/node-v22.20.0-x64.msi" -OutFile $tempMsi
  Start-Process msiexec.exe -ArgumentList @("/i", $tempMsi, "/qn", "/norestart") -Wait
  $env:Path = [System.Environment]::GetEnvironmentVariable("Path","Machine") + ";" + [System.Environment]::GetEnvironmentVariable("Path","User")
  $nodeVer = & node -v
}

New-Item -ItemType Directory -Force -Path $installRoot | Out-Null
$staged = Join-Path $installRoot "${tarballName}"
Copy-Item -LiteralPath $tarball -Destination $staged -Force
& npm.cmd install -g --omit=dev $staged
if ($LASTEXITCODE -ne 0) { throw "npm install -g failed with exit $LASTEXITCODE" }
$env:Path = [System.Environment]::GetEnvironmentVariable("Path","Machine") + ";" + [System.Environment]::GetEnvironmentVariable("Path","User")

$port = ${serverPort}
$url = "http://127.0.0.1:$port/"
$started = $false
${startBlock}

@{
  ok = $true
  node = "$nodeVer"
  package = "${packageName}"
  tarball = "${tarballName}"
  installRoot = $installRoot
  serverPort = $port
  url = $url
  started = $started
  hostname = $env:COMPUTERNAME
  os = [System.Environment]::OSVersion.VersionString
} | ConvertTo-Json -Compress | Set-Content -LiteralPath (Join-Path $opRoot "result.json") -Encoding utf8
`.trim();
}

async function installOneWindows(target: RemoteInstallTarget, pack: ReturnType<typeof packOpenPanel>): Promise<RemoteInstallHostResult> {
    const password = passwordFromEnv(target.passwordEnv);
    const probe = await probeRemoteHost({ host: target.host, targetOs: target.targetOs ?? 'windows' });
    const channel = target.channel && target.channel !== 'auto' ? target.channel : probe.recommendedChannel;

    if (channel !== 'rdp') {
        return {
            host: target.host,
            ok: false,
            channel,
            probe,
            pack,
            error:
                channel === 'none'
                    ? `no usable channel (RDP/SSH/WinRM). probe notes: ${probe.notes.join(' ')}`
                    : `channel \`${channel}\` is optional only. Windows install requires RDP(3389) for first-class delivery.`,
        };
    }

    const rdpOpen = probe.ports.find((port) => port.name === 'rdp')?.open;
    if (!rdpOpen) {
        return {
            host: target.host,
            ok: false,
            channel: 'rdp',
            probe,
            pack,
            error: 'RDP(3389) is not reachable from this controller',
        };
    }

    const serverPort = target.serverPort ?? 17890;
    const tarballName = path.basename(pack.tarballPath);
    const script = buildInstallScript({
        tarballName,
        packageName: pack.packageName,
        serverPort,
        startServer: !target.skipStart,
    });

    let rdp: RdpJobResult;
    try {
        rdp = await runRdpJob({
            host: target.host,
            username: target.user,
            password,
            script,
            timeoutMs: target.timeoutMs,
            files: {
                [tarballName]: { copyFrom: pack.tarballPath },
            },
        });
    }
    catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        const bootstrapPath = path.join(os.tmpdir(), `open-panel-bootstrap-${process.pid}.ps1`);
        const bootstrap = windowsBootstrapScript(pack.tarballPath);
        fs.writeFileSync(bootstrapPath, bootstrap, 'utf8');
        return {
            host: target.host,
            ok: false,
            channel: 'rdp',
            probe,
            pack,
            error:
                `RDP automation unavailable (${message}). ` +
                `Wrote interactive bootstrap script to \`${bootstrapPath}\`. ` +
                'Open an RDP session, copy the local pack tarball + script, run elevated, then re-probe for WinRM/SSH.',
        };
    }

    const url = typeof rdp.result.url === 'string'
        ? rdp.result.url.replace('127.0.0.1', target.host).replace('localhost', target.host)
        : `http://${target.host}:${serverPort}/`;
    const rosterId = (target.serverId?.trim() || target.host.replace(/[^A-Za-z0-9._:-]/g, '-')).slice(0, 64);
    upsertServer({
        id: rosterId,
        label: target.label?.trim() || target.host,
        url,
    });

    return {
        host: target.host,
        ok: true,
        channel: 'rdp',
        probe,
        pack,
        rdp,
        rosterId,
        url,
    };
}

/** Write a standalone Windows bootstrap script for interactive RDP sessions. */
export function windowsBootstrapScript(packageSpec = '@wae/open-panel'): string {
    const escaped = packageSpec.replace(/'/g, "''");
    return `# OpenPanel remote bootstrap (run elevated on the target Windows host)
$ErrorActionPreference = 'Stop'
$spec = '${escaped}'
if (-not (Get-Command node -ErrorAction SilentlyContinue)) {
  throw 'Node.js is required on the target. Install Node >= 22, then re-run.'
}
npm install -g $spec
open-panel version
`;
}

/** Install / upgrade `@wae/open-panel` on one or more hosts. */
export async function installRemote(targets: RemoteInstallTarget[]): Promise<RemoteInstallReport> {
    if (!targets.length) {
        throw new AppError({ code: 'invalid_argument', message: 'at least one `--host` is required' });
    }
    const pack = packOpenPanel();
    const results: RemoteInstallHostResult[] = [];
    for (const target of targets) {
        try {
            const os = target.targetOs ?? 'windows';
            if (os === 'linux') {
                results.push({
                    host: target.host,
                    ok: false,
                    pack,
                    error: 'Linux SSH install path is reserved. This change set delivers Windows RDP install first.',
                });
                continue;
            }
            results.push(await installOneWindows(target, pack));
        }
        catch (error) {
            const app = error instanceof AppError ? error : new AppError({
                code: 'remote_install_failed',
                capability: 'remote-install',
                message: error instanceof Error ? error.message : String(error),
            });
            results.push({
                host: target.host,
                ok: false,
                pack,
                error: app.message,
            });
        }
    }
    return {
        ok: results.every((item) => item.ok),
        pack,
        results,
    };
}
