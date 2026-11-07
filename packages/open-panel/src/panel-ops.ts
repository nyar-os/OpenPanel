/**
 * Typed wrappers for native OS services and web site panel ops.
 *
 * Application runtimes (Node / Python / Java) are intentionally absent.
 */
import { fromNativeError } from './errors.js';
import { loadNative } from './native.js';

export type NativeServiceState =
    | 'stopped'
    | 'start-pending'
    | 'stop-pending'
    | 'running'
    | 'continue-pending'
    | 'pause-pending'
    | 'paused'
    | 'unknown';

export interface NativeServiceInfo {
    name: string;
    displayName: string;
    state: NativeServiceState;
}

export type WebSiteKind = 'static-host';
export type SiteRuntimeState = 'stopped' | 'running';

export interface WebSiteView {
    id: string;
    domain: string;
    rootDir: string;
    host: string;
    port: number;
    enabled: boolean;
    kind: WebSiteKind;
    runtime: SiteRuntimeState;
}

export interface WebSiteInput {
    id: string;
    domain: string;
    rootDir: string;
    host?: string;
    port: number;
    enabled?: boolean;
    kind?: WebSiteKind;
}

function invoke<T>(body: () => T): T {
    try {
        return body();
    }
    catch (error) {
        throw fromNativeError(error);
    }
}

function asNativeObject(input: Record<string, string | undefined>): Record<string, string> {
    const out: Record<string, string> = {};
    for (const [key, value] of Object.entries(input)) {
        if (value !== undefined) out[key] = value;
    }
    return out;
}

/** List native OS services (Windows SCM). */
export function listNativeServices(): NativeServiceInfo[] {
    return invoke(() => loadNative().listNativeServices() as NativeServiceInfo[]);
}

/** Query one native OS service. */
export function nativeServiceStatus(name: string): NativeServiceInfo {
    return invoke(() => loadNative().nativeServiceStatus({ name }) as NativeServiceInfo);
}

/** Start a native OS service. */
export function startNativeService(name: string): NativeServiceInfo {
    return invoke(() => loadNative().startNativeService({ name }) as NativeServiceInfo);
}

/** Stop a native OS service. */
export function stopNativeService(name: string): NativeServiceInfo {
    return invoke(() => loadNative().stopNativeService({ name }) as NativeServiceInfo);
}

/** Restart a native OS service. */
export function restartNativeService(name: string): NativeServiceInfo {
    return invoke(() => loadNative().restartNativeService({ name }) as NativeServiceInfo);
}

/** List registered web sites. */
export function listWebSites(): WebSiteView[] {
    return invoke(() => loadNative().listWebSites() as WebSiteView[]);
}

/** Register or replace a web site (`kind=static-host`). */
export function upsertWebSite(input: WebSiteInput): WebSiteView {
    return invoke(() => loadNative().upsertWebSite(asNativeObject({
        id: input.id,
        domain: input.domain,
        rootDir: input.rootDir,
        host: input.host,
        port: String(input.port),
        enabled: input.enabled === undefined ? undefined : (input.enabled ? 'true' : 'false'),
        kind: input.kind,
    })) as WebSiteView);
}

/** Remove a web site and stop its listener. */
export function removeWebSite(id: string): boolean {
    return invoke(() => loadNative().removeWebSite({ id }) as boolean);
}

/** Enable or disable a web site. */
export function setWebSiteEnabled(id: string, enabled: boolean): WebSiteView {
    return invoke(() => loadNative().setWebSiteEnabled({
        id,
        enabled: enabled ? 'true' : 'false',
    }) as WebSiteView);
}

/** Start the OpenPanel-hosted static listener for a site. */
export function startWebSite(id: string): WebSiteView {
    return invoke(() => loadNative().startWebSite({ id }) as WebSiteView);
}

/** Stop the OpenPanel-hosted static listener for a site. */
export function stopWebSite(id: string): WebSiteView {
    return invoke(() => loadNative().stopWebSite({ id }) as WebSiteView);
}
