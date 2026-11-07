/**
 * TCP reachability probe for remote install channel selection.
 *
 * Windows first-connect prefers RDP(3389). OpenSSH / WinRM are optional.
 * Linux prefers SSH(22).
 */
import fs from 'node:fs';
import net from 'node:net';

export type RemotePortName = 'ssh' | 'rdp' | 'winrmHttp' | 'winrmHttps' | 'smb';

export type RemoteChannel = 'rdp' | 'ssh' | 'winrm' | 'none';

export interface RemotePortProbe {
    name: RemotePortName;
    port: number;
    open: boolean;
    latencyMs: number | null;
    error?: string;
}

export interface RemoteProbeReport {
    host: string;
    checkedAt: string;
    ports: RemotePortProbe[];
    /** Preferred install channel given open ports and OS policy. */
    recommendedChannel: RemoteChannel;
    notes: string[];
}

const DEFAULT_PORTS: Array<{ name: RemotePortName; port: number }> = [
    { name: 'rdp', port: 3389 },
    { name: 'ssh', port: 22 },
    { name: 'winrmHttp', port: 5985 },
    { name: 'winrmHttps', port: 5986 },
    { name: 'smb', port: 445 },
];

function probePort(host: string, port: number, timeoutMs: number): Promise<Omit<RemotePortProbe, 'name'>> {
    const started = Date.now();
    return new Promise((resolve) => {
        const socket = new net.Socket();
        let settled = false;
        const finish = (result: Omit<RemotePortProbe, 'name' | 'port'>) => {
            if (settled) return;
            settled = true;
            socket.destroy();
            resolve({ port, ...result });
        };
        socket.setTimeout(timeoutMs);
        socket.once('connect', () => {
            finish({ open: true, latencyMs: Date.now() - started });
        });
        socket.once('timeout', () => {
            finish({ open: false, latencyMs: null, error: 'timeout' });
        });
        socket.once('error', (error) => {
            finish({ open: false, latencyMs: null, error: error.message });
        });
        socket.connect(port, host);
    });
}

function hasLocalSsh(): boolean {
    if (process.env.OPEN_PANEL_SSH_PATH?.trim()) return true;
    if (process.platform === 'win32') {
        return fs.existsSync('C:\\Windows\\System32\\OpenSSH\\ssh.exe');
    }
    return true;
}

export interface ProbeOptions {
    host: string;
    /** Hint for channel preference. Defaults to `windows` on win32 controllers. */
    targetOs?: 'windows' | 'linux' | 'auto';
    timeoutMs?: number;
}

/** Probe remoting ports and recommend a channel without requiring OpenSSH on the target. */
export async function probeRemoteHost(options: ProbeOptions | string): Promise<RemoteProbeReport> {
    const opts = typeof options === 'string' ? { host: options } : options;
    const trimmed = opts.host.trim();
    const timeoutMs = opts.timeoutMs ?? 2500;
    const targetOs = opts.targetOs
        ?? (process.platform === 'win32' ? 'windows' : 'linux');

    if (!trimmed) {
        return {
            host: '',
            checkedAt: new Date().toISOString(),
            ports: [],
            recommendedChannel: 'none',
            notes: ['host is required'],
        };
    }

    const ports: RemotePortProbe[] = [];
    for (const item of DEFAULT_PORTS) {
        const result = await probePort(trimmed, item.port, timeoutMs);
        ports.push({
            name: item.name,
            port: item.port,
            open: result.open,
            latencyMs: result.latencyMs,
            ...(result.error ? { error: result.error } : {}),
        });
    }

    const open = new Set(ports.filter((port) => port.open).map((port) => port.name));
    const notes: string[] = [];
    let recommendedChannel: RemoteChannel = 'none';

    if (targetOs === 'windows' || targetOs === 'auto') {
        if (open.has('rdp')) {
            recommendedChannel = 'rdp';
            notes.push('RDP(3389) is reachable. Windows install uses RDP remote execution as the primary channel.');
        }
        if (open.has('winrmHttp') || open.has('winrmHttps')) {
            notes.push('WinRM is optional and available if explicitly selected with `--channel winrm`.');
        }
        if (open.has('ssh')) {
            notes.push('SSH is optional on Windows and is not required for install.');
        }
    }

    if (recommendedChannel === 'none' && (targetOs === 'linux' || targetOs === 'auto')) {
        if (open.has('ssh') && hasLocalSsh()) {
            recommendedChannel = 'ssh';
            notes.push('SSH(22) is reachable. Linux install uses SSH.');
        }
        else if (open.has('ssh')) {
            notes.push('SSH port is open, but no local OpenSSH client was found. Set `OPEN_PANEL_SSH_PATH` or install OpenSSH Client.');
        }
    }

    if (recommendedChannel === 'none' && (open.has('winrmHttp') || open.has('winrmHttps'))) {
        recommendedChannel = 'winrm';
        notes.push('Falling back to WinRM because RDP/SSH were not selected.');
    }

    if (recommendedChannel === 'none') {
        notes.push('No usable remoting port is open from this controller.');
    }

    return {
        host: trimmed,
        checkedAt: new Date().toISOString(),
        ports,
        recommendedChannel,
        notes,
    };
}
