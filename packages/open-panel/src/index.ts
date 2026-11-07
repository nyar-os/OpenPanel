/**
 * Typed library surface over the N-API boundary and the WebUI listener.
 *
 * One installable owns both entries:
 * - `runClient` opens the desktop client (WAE WebView + `@wae/open-panel-gui`)
 * - `runServer` hosts `@wae/open-panel-webui` as a local WebUI
 */
import { AppError, fromNativeError } from './errors.js';
import { loadNative } from './native.js';
import { startServer, type ServerHandle, type ServerOptions } from './server.js';

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

/** Open the desktop client and pump until the window closes. */
export async function runClient(options: ClientOptions = {}): Promise<string> {
    const opened = openGui(options);
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
