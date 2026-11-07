/**
 * Typed library surface over the N-API boundary and the WebUI listener.
 *
 * One installable owns both entries:
 * - `runClient` opens the desktop client (WAE WebView + `@wae/open-panel-gui`)
 *   as a multi-server control plane (see `ClientServerRoster`)
 * - `runServer` hosts `@wae/open-panel-webui` as a single-machine WebUI endpoint
 */
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { AppError, fromNativeError } from './errors.js';
import {
    getActiveServer,
    rosterSnapshot,
    useServer,
} from './managed-servers.js';
import { loadNative, workspaceRoot } from './native.js';
import { startServer, type ServerHandle, type ServerOptions } from './server.js';

export {
    emptyServerRoster,
    getActiveServer,
    loadServerRoster,
    removeServer,
    resolveServerUrl,
    rosterSnapshot,
    saveServerRoster,
    upsertServer,
    useServer,
    type ClientServerRoster,
    type ManagedServerId,
    type ManagedServerRef,
    type RosterSnapshot,
    type UpsertServerInput,
} from './managed-servers.js';
export { ensureConfigDir, resolveConfigDir, rosterPath } from './paths.js';
export {
    DEFAULT_WEBUI_PORT,
    WEBUI_PASSWORD_LENGTH,
    WEBUI_PASSWORD_MIN_LENGTH,
    WEBUI_USERNAME,
    webuiLoginUrl,
} from './defaults.js';
export {
    assertPasswordStrength,
    credentialsExist,
    credentialsPath,
    generateStrongPassword,
    installCredentials,
    loadCredentials,
    passwordEntropyBits,
    type InstallCredentialsResult,
    type WebuiCredentials,
} from './credentials.js';
export {
    formatInstallReport,
    runInstall,
    type InstallReport,
    type RunInstallOptions,
} from './install.js';
export {
    probeRemoteHost,
    type RemoteChannel,
    type RemotePortProbe,
    type RemoteProbeReport,
} from './remote-probe.js';
export {
    installRemote,
    type RemoteInstallHostResult,
    type RemoteInstallReport,
    type RemoteInstallTarget,
} from './remote-install.js';
export { packOpenPanel, passwordFromEnv } from './remote-pack.js';
export {
    rdpExec,
    rdpHandshake,
    rdpProbe,
    resolveRdpBin,
    type RdpExecInput,
    type RdpExecReport,
    type RdpHandshakeInput,
    type RdpHandshakeReport,
} from './remote-ironrdp.js';
export {
    buildAlternateShellBootstrap,
    startPackStageServer,
    waitForTcp,
} from './remote-stage.js';
export { hostStatus, resolveListenPort } from './server.js';
export {
    listNativeServices,
    nativeServiceStatus,
    startNativeService,
    stopNativeService,
    restartNativeService,
    listWebSites,
    upsertWebSite,
    removeWebSite,
    setWebSiteEnabled,
    startWebSite,
    stopWebSite,
    type NativeServiceInfo,
    type NativeServiceState,
    type SiteRuntimeState,
    type WebSiteInput,
    type WebSiteKind,
    type WebSiteView,
} from './panel-ops.js';

export type CapabilityState = 'unsupported_platform' | 'not_implemented' | 'dependency_blocked';

export interface CapabilityItem {
    id: string;
    state: CapabilityState | null;
    detail: string | null;
}

export interface CapabilitiesReport {
    platform: string;
    arch: string;
    items: CapabilityItem[];
    notes: string[];
}

export interface ClientOptions {
    contentDir?: string;
    title?: string;
    /** Select this roster id before opening the desktop client. */
    serverId?: string;
}

export interface AuditActor {
    actorKind?: string;
    actorId?: string;
    actorSource?: string;
    session?: string;
    host?: string;
}

export interface AuditQuery {
    kind?: string;
    id?: string;
    fromMs?: number;
    toMs?: number;
    capability?: string;
    limit?: number;
}

function invoke<T>(body: () => T): T {
    try {
        return body();
    }
    catch (error) {
        throw fromNativeError(error);
    }
}

export function version(): string {
    return invoke(() => loadNative().version());
}

export function capabilities(): CapabilitiesReport {
    return invoke(() => loadNative().capabilities() as CapabilitiesReport);
}

export function platform(): string {
    return invoke(() => loadNative().platform());
}

export function configureAudit(actor: AuditActor): { ok: boolean } {
    return invoke(() => loadNative().configureAudit(actor) as { ok: boolean });
}

export function auditQuery(filter: AuditQuery = {}): Record<string, unknown>[] {
    const json = invoke(() => loadNative().auditQuery(filter));
    return invoke(() => JSON.parse(json) as Record<string, unknown>[]);
}

export function openGui(options: ClientOptions = {}): { backend: string } {
    return invoke(() => loadNative().openGui(options) as { backend: string });
}

export function pumpEvents(): boolean {
    return invoke(() => loadNative().pumpEvents());
}

export function closeGui(): boolean {
    return invoke(() => loadNative().closeGui());
}

export function isGuiOpen(): boolean {
    return invoke(() => loadNative().isGuiOpen());
}

function writeRosterSnapshot(contentDir?: string): void {
    const here = path.dirname(fileURLToPath(import.meta.url));
    const candidates = [
        contentDir,
        process.env.OPEN_PANEL_GUI,
        path.join(here, '..', 'dist', 'gui'),
        path.join(workspaceRoot(), 'packages', 'open-panel-gui', 'dist', 'static'),
    ].filter((value): value is string => typeof value === 'string' && value.trim().length > 0);
    const snapshot = `${JSON.stringify(rosterSnapshot(), null, 2)}\n`;
    for (const dir of candidates) {
        try {
            fs.mkdirSync(dir, { recursive: true });
            fs.writeFileSync(path.join(dir, 'roster.snapshot.json'), snapshot, 'utf8');
            return;
        }
        catch {
            // try next candidate
        }
    }
}

/** Open the desktop client and pump until the window closes. */
export async function runClient(options: ClientOptions = {}): Promise<string> {
    if (options.serverId?.trim()) {
        useServer(options.serverId);
    }
    const active = getActiveServer();
    const title = options.title
        ?? (active ? `OpenPanel — ${active.label ?? active.id}` : undefined);
    writeRosterSnapshot(options.contentDir);
    const opened = openGui({
        contentDir: options.contentDir,
        ...(title ? { title } : {}),
    });
    while (pumpEvents()) {
        await new Promise((resolve) => setTimeout(resolve, 16));
    }
    closeGui();
    return opened.backend;
}

/**
 * Start the local WebUI listener for browser access.
 *
 * This is the server entry. It hosts `@wae/open-panel-webui`, not the desktop
 * GUI package.
 */
export async function runServer(options: ServerOptions = {}): Promise<ServerHandle> {
    try {
        return await startServer(options);
    }
    catch (error) {
        if (error instanceof AppError) throw error;
        throw fromNativeError(error);
    }
}

export type { ServerHandle, ServerOptions };
export { AppError, EXIT, fromNativeError } from './errors.js';
