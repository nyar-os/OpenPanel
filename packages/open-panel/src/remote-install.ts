/**
 * Remote install orchestration for `@wae/open-panel`.
 *
 * Windows primary channel: Rust RDP stack (`open-panel-rdp` / IronRDP) with
 * force-fresh logon + Win+R inject of a short `cmd /c curl ?` bootstrap (same
 * shape as the proven ntfy proof). Default payload delivery is HTTP staging
 * (`i.ps1` + tarball); pass a remote-reachable `--pack-base-url` when the host
 * cannot dial the controller LAN. RDPDR remains an opt-in fallback
 * (`OPEN_PANEL_USE_RDPDR=1`) ? bare UNC in Run has never executed on lab hosts.
 * OpenSSH is not required. External RDP CLIs are forbidden.
 */
import { DEFAULT_WEBUI_PORT } from './defaults.js';
import { AppError } from './errors.js';
import { upsertServer } from './managed-servers.js';
import { rdpExec, type RdpExecReport } from './remote-ironrdp.js';
import { packOpenPanel, passwordFromEnv } from './remote-pack.js';
import { probeRemoteHost, type RemoteChannel, type RemoteProbeReport } from './remote-probe.js';
import {
    buildAlternateShellBootstrap,
    buildInjectRdpdrLauncherShell,
    defaultWorkDir,
    prepareRdpdrStageDir,
    RDPDR_ACK_FILE,
    startPackStageServer,
    waitForTcp,
    type RdpdrStageDir,
    type StageServer,
} from './remote-stage.js';

export interface RemoteInstallTarget {
    host: string;
    user: string;
    passwordEnv: string;
    channel?: RemoteChannel | 'auto';
    targetOs?: 'windows' | 'linux' | 'auto';
    serverPort?: number;
    serverId?: string;
    label?: string;
    skipStart?: boolean;
    timeoutMs?: number;
    /**
     * Full URL to `i.ps1` (or any bootstrap script) already reachable from the
     * remote host. Skips the controller ephemeral HTTP stage when set.
     */
    packageUrl?: string;
    /**
     * Known remote filesystem path to a packed `.tgz` already present on the
     * host. Builds a short local-file bootstrap instead of HTTP staging.
     */
    remoteTarball?: string;
    /** Override controller pack base URL (e.g. public IP:port the remote can reach). */
    packBaseUrl?: string;
    /**
     * Force HTTP pack staging (default). Set `false` only with
     * `OPEN_PANEL_USE_RDPDR=1` for the legacy redirected-share path.
     */
    preferHttpStage?: boolean;
    /** Optional ntfy.sh topic for remote progress (`boot`/`npm-ok`/`pw:?`/`up:8888`). */
    ntfyTopic?: string;
}

export interface RemoteInstallHostResult {
    host: string;
    ok: boolean;
    channel?: RemoteChannel;
    probe?: RemoteProbeReport;
    pack?: ReturnType<typeof packOpenPanel>;
    exec?: RdpExecReport;
    stage?: { baseUrl: string, scriptUrl: string, tarballUrl: string };
    rosterId?: string;
    url?: string;
    error?: string;
    next?: string;
}

export interface RemoteInstallReport {
    ok: boolean;
    pack: ReturnType<typeof packOpenPanel>;
    results: RemoteInstallHostResult[];
}

function buildRemotePathShell(remoteTarball: string, serverPort: number, skipStart: boolean): string {
    const pathLiteral = remoteTarball.replace(/'/g, "''");
    // Keep under the AlternateShell 512-byte cap. Credentials come from the
    // package `postinstall` ¡ú `open-panel install` path (same as npx / local).
    const start = skipStart
        ? ''
        : `;$c=Get-Command open-panel;Start-Process $c.Source -ArgumentList @('server','--host','0.0.0.0','--port','${serverPort}') -WindowStyle Hidden`;
    const command =
        `powershell.exe -NoP -EP Bypass -w Hidden -c "npm i -g --omit=dev '${pathLiteral}'${start}"`;
    if (Buffer.byteLength(command, 'utf8') + 1 > 510) {
        throw new AppError({
            code: 'invalid_argument',
            message: '`--remote-tarball` bootstrap exceeds the AlternateShell 512-byte wire limit',
        });
    }
    return command;
}

async function installOneWindows(
    target: RemoteInstallTarget,
    pack: ReturnType<typeof packOpenPanel>,
): Promise<RemoteInstallHostResult> {
    passwordFromEnv(target.passwordEnv);

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
                    ? `no usable channel. probe notes: ${probe.notes.join(' ')}`
                    : `Windows install requires RDP(3389). Optional channel \`${channel}\` is not the primary path.`,
        };
    }

    if (!probe.ports.find((port) => port.name === 'rdp')?.open) {
        return {
            host: target.host,
            ok: false,
            channel: 'rdp',
            probe,
            pack,
            error: 'RDP(3389) is not reachable from this controller',
        };
    }

    const serverPort = target.serverPort ?? DEFAULT_WEBUI_PORT;
    const url = `http://${target.host}:${serverPort}/login`;
    const rosterId = (target.serverId?.trim() || target.host.replace(/[^A-Za-z0-9._:-]/g, '-')).slice(0, 64);
    // Node bootstrap + npm install -g + server start needs a generous ActiveStage hold.
    const sessionTimeoutSecs = Math.max(300, Math.ceil((target.timeoutMs ?? 420_000) / 1000));

    let stage: StageServer | undefined;
    let rdpdrStage: RdpdrStageDir | undefined;
    let shell: string;
    let stageInfo: RemoteInstallHostResult['stage'];
    let rdpdrDir: string | undefined;

    try {
        if (target.remoteTarball?.trim()) {
            shell = buildRemotePathShell(target.remoteTarball.trim(), serverPort, Boolean(target.skipStart));
        }
        else if (target.packageUrl?.trim()) {
            shell = buildAlternateShellBootstrap(target.packageUrl.trim());
        }
        else if (process.env.OPEN_PANEL_USE_RDPDR === '1' && target.preferHttpStage === false) {
            // Opt-in only: lab hosts never executed bare UNC from Win+R.
            rdpdrStage = prepareRdpdrStageDir({
                pack,
                serverPort,
                skipStart: target.skipStart,
            });
            rdpdrDir = rdpdrStage.dir;
            shell = buildInjectRdpdrLauncherShell();
            stageInfo = {
                baseUrl: `\\\\tsclient\\OP`,
                scriptUrl: `\\\\tsclient\\OP\\a.cmd`,
                tarballUrl: `\\\\tsclient\\OP\\${rdpdrStage.tarballName}`,
            };
        }
        else {
            // Default: HTTP stage + short curl Win+R bootstrap (ntfy-proven shape).
            const packBaseUrl = target.packBaseUrl?.trim();
            let stageListenPort: number | undefined;
            if (packBaseUrl) {
                try {
                    const parsed = new URL(packBaseUrl.includes('://') ? packBaseUrl : `http://${packBaseUrl}`);
                    const portNum = Number(parsed.port);
                    if (Number.isFinite(portNum) && portNum > 0) {
                        stageListenPort = portNum;
                    }
                }
                catch {
                    // Invalid override URLs still flow into startPackStageServer for a clear error.
                }
            }
            const ntfyTopic = (target.ntfyTopic ?? process.env.OPEN_PANEL_INSTALL_NTFY ?? '').trim();
            stage = await startPackStageServer({
                pack,
                baseUrl: packBaseUrl,
                port: stageListenPort,
                serverPort,
                skipStart: target.skipStart,
                ntfyTopic: ntfyTopic || undefined,
            });
            shell = buildAlternateShellBootstrap(stage.scriptUrl);
            stageInfo = {
                baseUrl: stage.baseUrl,
                scriptUrl: stage.scriptUrl,
                tarballUrl: stage.tarballUrl,
            };
        }

        const exec = rdpExec({
            host: target.host,
            user: target.user,
            passwordEnv: target.passwordEnv,
            shell,
            workDir: defaultWorkDir(),
            sessionTimeoutSecs,
            rdpdrDir,
            rdpdrAckFile: rdpdrDir ? RDPDR_ACK_FILE : undefined,
            // Force-fresh then Win+R inject. AlternateShell alone is silent on lab hosts.
            forceFreshLogon: true,
            injectRun: true,
        });

        if (!exec.ok) {
            return {
                host: target.host,
                ok: false,
                channel: 'rdp',
                probe,
                pack,
                exec,
                stage: stageInfo,
                rosterId,
                url,
                error: exec.error ?? 'RDP alternate_shell session failed',
                next: rdpdrDir
                    ? 'RDPDR staging was used. Prefer HTTP short curl Win+R (default); bare UNC has not proven on lab hosts.'
                    : stageInfo
                    ? `Ensure the remote can reach staging URL \`${stageInfo.baseUrl}\` (set \`OPEN_PANEL_PACK_BASE_URL\` / \`--pack-base-url\` to a public origin).`
                    : 'Check Win+R inject on the host and retry.',
            };
        }

        let serverUp = true;
        if (!target.skipStart) {
            serverUp = await waitForTcp(target.host, serverPort, Math.min(sessionTimeoutSecs * 1000, 240_000));
        }

        if (!serverUp) {
            return {
                host: target.host,
                ok: false,
                channel: 'rdp',
                probe,
                pack,
                exec,
                stage: stageInfo,
                rosterId,
                url,
                error: `RDP exec finished (${exec.endReason ?? 'unknown'}) but TCP ${serverPort} did not open`,
                next:
                    'Remote may lack Node/npm, could not reach the pack URL, or failed to start `open-panel server`. ' +
                    'Confirm staging reachability or pre-place a tarball and pass `--remote-tarball`.',
            };
        }

        upsertServer({
            id: rosterId,
            label: target.label?.trim() || target.host,
            url,
            host: target.host,
            port: serverPort,
        });

        return {
            host: target.host,
            ok: true,
            channel: 'rdp',
            probe,
            pack,
            exec,
            stage: stageInfo,
            rosterId,
            url,
        };
    }
    finally {
        if (stage) {
            try {
                await stage.close();
            }
            catch {
                // Staging teardown must not mask install results.
            }
        }
        if (rdpdrStage) {
            rdpdrStage.close();
        }
    }
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
            if ((target.targetOs ?? 'windows') === 'linux') {
                results.push({
                    host: target.host,
                    ok: false,
                    pack,
                    error: 'Linux SSH install is reserved. This change set delivers the Windows IronRDP path first.',
                });
                continue;
            }
            results.push(await installOneWindows(target, pack));
        }
        catch (error) {
            const app = error instanceof AppError
                ? error
                : new AppError({
                    code: 'remote_install_failed',
                    capability: 'remote-install',
                    message: error instanceof Error ? error.message : String(error),
                });
            results.push({ host: target.host, ok: false, pack, error: app.message });
        }
    }
    return { ok: results.every((item) => item.ok), pack, results };
}
