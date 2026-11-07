/**
 * Remote install orchestration for `@wae/open-panel`.
 *
 * Windows primary channel: Rust RDP stack (`open-panel-rdp` / IronRDP).
 * OpenSSH on the target is not required. External RDP CLIs are forbidden.
 */
import { AppError } from './errors.js';
import { packOpenPanel, passwordFromEnv } from './remote-pack.js';
import { rdpHandshake, type RdpHandshakeReport } from './remote-ironrdp.js';
import { probeRemoteHost, type RemoteChannel, type RemoteProbeReport } from './remote-probe.js';

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
}

export interface RemoteInstallHostResult {
    host: string;
    ok: boolean;
    channel?: RemoteChannel;
    probe?: RemoteProbeReport;
    pack?: ReturnType<typeof packOpenPanel>;
    handshake?: RdpHandshakeReport;
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

    const handshake = rdpHandshake({
        host: target.host,
        user: target.user,
        passwordEnv: target.passwordEnv,
    });

    if (!handshake.ok) {
        return {
            host: target.host,
            ok: false,
            channel: 'rdp',
            probe,
            pack,
            handshake,
            error: handshake.error ?? 'RDP handshake failed',
        };
    }

    const serverPort = target.serverPort ?? 17890;
    const url = `http://${target.host}:${serverPort}/`;
    const rosterId = (target.serverId?.trim() || target.host.replace(/[^A-Za-z0-9._:-]/g, '-')).slice(0, 64);

    return {
        host: target.host,
        ok: false,
        channel: 'rdp',
        probe,
        pack,
        handshake,
        rosterId,
        url,
        error: 'RDP handshake succeeded. Remote package install via alternate_shell / RDPDR is not implemented yet.',
        next:
            `Packed tarball ready at \`${pack.tarballPath}\`. ` +
            'Next Rust layer: stage payload and set Config.alternate_shell to run the install script.',
    };
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
