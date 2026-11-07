/**
 * Client-side managed server list (skeleton).
 *
 * Contract: `open-panel client` is a multi-server control plane. One desktop
 * client may track many `open-panel server` endpoints (local or remote). Each
 * `server` process remains a single-machine ops endpoint.
 *
 * This module only reserves the data shape. It does not persist config, open
 * remote sessions, or perform SSH install / upgrade.
 */

/** Stable id for a registered panel server endpoint. */
export type ManagedServerId = string;

/**
 * Placeholder fields for one managed server entry.
 *
 * Prefer `url` when known; otherwise `host` + optional `port` may be filled later.
 * Secrets must never appear here or in any tracked config file.
 */
export interface ManagedServerRef {
    id: ManagedServerId;
    /** Human label shown in the client UI. */
    label?: string;
    /** Full base URL of the WebUI, e.g. `http://127.0.0.1:17890/`. */
    url?: string;
    /** Hostname or address when `url` is not yet composed. */
    host?: string;
    /** Listen port when composing from `host`. */
    port?: number;
}

/** Local client config skeleton: a list of servers, not a single binding. */
export interface ClientServerRoster {
    /** Active selection; must refer to an entry in `servers` when set. */
    activeId?: ManagedServerId;
    servers: ManagedServerRef[];
}

/** Empty roster used until persistence lands. */
export function emptyServerRoster(): ClientServerRoster {
    return { servers: [] };
}
