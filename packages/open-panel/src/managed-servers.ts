/**
 * Client-side managed server roster with local JSON persistence.
 *
 * Contract: `open-panel client` is a multi-server control plane. One desktop
 * client may track many `open-panel server` endpoints (local or remote). Each
 * `server` process remains a single-machine ops endpoint.
 *
 * Secrets must never appear in this file or in any tracked config.
 */
import fs from 'node:fs';

import { AppError } from './errors.js';
import { ensureConfigDir, rosterPath } from './paths.js';

/** Stable id for a registered panel server endpoint. */
export type ManagedServerId = string;

/**
 * One managed server entry.
 *
 * Prefer `url` when known; otherwise `host` + optional `port` may be composed.
 * Secrets must never appear here.
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

/** Fields accepted when adding or updating a server entry. */
export interface UpsertServerInput {
    id: ManagedServerId;
    label?: string;
    url?: string;
    host?: string;
    port?: number;
}

/** Local client config: a list of servers, not a single binding. */
export interface ClientServerRoster {
    /** Active selection; must refer to an entry in `servers` when set. */
    activeId?: ManagedServerId;
    servers: ManagedServerRef[];
}

/** Server entry enriched for GUI / JSON snapshots. */
export interface ManagedServerView extends ManagedServerRef {
    resolvedUrl?: string;
}

/** Machine-readable roster view for CLI `--json` and GUI IPC. */
export interface RosterSnapshot {
    path: string;
    activeId?: ManagedServerId;
    active?: ManagedServerRef;
    servers: ManagedServerView[];
}

/** Empty roster used when no config file exists yet. */
export function emptyServerRoster(): ClientServerRoster {
    return { servers: [] };
}

/** Compose a WebUI base URL from host/port when `url` is absent. */
export function resolveServerUrl(ref: Pick<ManagedServerRef, 'url' | 'host' | 'port'>): string | undefined {
    if (ref.url?.trim()) return normalizeBaseUrl(ref.url);
    const host = ref.host?.trim();
    if (!host) return undefined;
    const port = ref.port ?? 17890;
    return `http://${host}:${port}/`;
}

function normalizeBaseUrl(url: string): string {
    const trimmed = url.trim();
    return trimmed.endsWith('/') ? trimmed : `${trimmed}/`;
}

function assertId(id: string): ManagedServerId {
    const trimmed = id.trim();
    if (!trimmed) {
        throw new AppError({ code: 'invalid_argument', message: 'server `id` must be a non-empty string' });
    }
    if (!/^[A-Za-z0-9][A-Za-z0-9._:-]{0,63}$/.test(trimmed)) {
        throw new AppError({
            code: 'invalid_argument',
            message: 'server `id` must match `[A-Za-z0-9][A-Za-z0-9._:-]{0,63}`',
        });
    }
    return trimmed;
}

function normalizeRef(input: UpsertServerInput): ManagedServerRef {
    const id = assertId(input.id);
    const label = input.label?.trim() || undefined;
    const host = input.host?.trim() || undefined;
    const url = input.url?.trim() ? normalizeBaseUrl(input.url) : undefined;
    let port = input.port;
    if (port !== undefined) {
        if (!Number.isInteger(port) || port < 1 || port > 65535) {
            throw new AppError({ code: 'invalid_argument', message: '`port` must be an integer between 1 and 65535' });
        }
    }
    if (!url && !host) {
        throw new AppError({
            code: 'invalid_argument',
            message: 'server entry requires `url` or `host`',
        });
    }
    const ref: ManagedServerRef = { id };
    if (label) ref.label = label;
    if (url) ref.url = url;
    if (host) ref.host = host;
    if (port !== undefined) ref.port = port;
    return ref;
}

function cloneRoster(roster: ClientServerRoster): ClientServerRoster {
    return {
        ...(roster.activeId ? { activeId: roster.activeId } : {}),
        servers: roster.servers.map((server) => ({ ...server })),
    };
}

function parseRoster(raw: unknown): ClientServerRoster {
    if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
        throw new AppError({ code: 'invalid_argument', message: 'roster JSON must be an object' });
    }
    const record = raw as Record<string, unknown>;
    const serversRaw = record.servers;
    if (!Array.isArray(serversRaw)) {
        throw new AppError({ code: 'invalid_argument', message: 'roster JSON requires a `servers` array' });
    }
    const servers: ManagedServerRef[] = [];
    const seen = new Set<string>();
    for (const item of serversRaw) {
        if (!item || typeof item !== 'object' || Array.isArray(item)) {
            throw new AppError({ code: 'invalid_argument', message: 'each server entry must be an object' });
        }
        const entry = item as Record<string, unknown>;
        if (typeof entry.id !== 'string') {
            throw new AppError({ code: 'invalid_argument', message: 'each server entry requires a string `id`' });
        }
        const port = entry.port;
        const ref = normalizeRef({
            id: entry.id,
            label: typeof entry.label === 'string' ? entry.label : undefined,
            url: typeof entry.url === 'string' ? entry.url : undefined,
            host: typeof entry.host === 'string' ? entry.host : undefined,
            port: typeof port === 'number' ? port : undefined,
        });
        if (seen.has(ref.id)) {
            throw new AppError({ code: 'invalid_argument', message: `duplicate server id \`${ref.id}\`` });
        }
        seen.add(ref.id);
        servers.push(ref);
    }
    const activeId = typeof record.activeId === 'string' && record.activeId.trim()
        ? record.activeId.trim()
        : undefined;
    if (activeId && !servers.some((server) => server.id === activeId)) {
        throw new AppError({
            code: 'invalid_argument',
            message: `activeId \`${activeId}\` is not in the roster`,
        });
    }
    return activeId ? { activeId, servers } : { servers };
}

/** Load the roster from disk, or return an empty roster when missing. */
export function loadServerRoster(filePath = rosterPath()): ClientServerRoster {
    if (!fs.existsSync(filePath)) return emptyServerRoster();
    let text: string;
    try {
        text = fs.readFileSync(filePath, 'utf8');
    }
    catch (error) {
        throw new AppError({
            code: 'roster_read_failed',
            capability: 'client-roster',
            message: error instanceof Error ? error.message : String(error),
            path: filePath,
        });
    }
    if (!text.trim()) return emptyServerRoster();
    try {
        return parseRoster(JSON.parse(text) as unknown);
    }
    catch (error) {
        if (error instanceof AppError) throw error;
        throw new AppError({
            code: 'roster_invalid',
            capability: 'client-roster',
            message: error instanceof Error ? error.message : String(error),
            path: filePath,
        });
    }
}

/** Persist the roster atomically. */
export function saveServerRoster(roster: ClientServerRoster, filePath = rosterPath()): ClientServerRoster {
    const normalized = parseRoster(roster);
    ensureConfigDir();
    const tempPath = `${filePath}.${process.pid}.tmp`;
    const body = `${JSON.stringify(normalized, null, 2)}\n`;
    try {
        fs.writeFileSync(tempPath, body, 'utf8');
        fs.renameSync(tempPath, filePath);
    }
    catch (error) {
        try {
            fs.unlinkSync(tempPath);
        }
        catch {
            // Ignore cleanup failures.
        }
        throw new AppError({
            code: 'roster_write_failed',
            capability: 'client-roster',
            message: error instanceof Error ? error.message : String(error),
            path: filePath,
        });
    }
    return cloneRoster(normalized);
}

/** Add or update a server entry. New entries become active when none is selected. */
export function upsertServer(input: UpsertServerInput, filePath = rosterPath()): ClientServerRoster {
    const roster = loadServerRoster(filePath);
    const ref = normalizeRef(input);
    const index = roster.servers.findIndex((server) => server.id === ref.id);
    if (index >= 0) {
        const previous = roster.servers[index]!;
        roster.servers[index] = normalizeRef({
            id: ref.id,
            label: ref.label ?? previous.label,
            url: ref.url ?? previous.url,
            host: ref.host ?? previous.host,
            port: ref.port ?? previous.port,
        });
    }
    else {
        roster.servers.push(ref);
        if (!roster.activeId) roster.activeId = ref.id;
    }
    return saveServerRoster(roster, filePath);
}

/** Remove a server by id. Clears `activeId` when it pointed at the removed entry. */
export function removeServer(id: string, filePath = rosterPath()): ClientServerRoster {
    const roster = loadServerRoster(filePath);
    const target = assertId(id);
    const next = roster.servers.filter((server) => server.id !== target);
    if (next.length === roster.servers.length) {
        throw new AppError({ code: 'invalid_argument', message: `server id \`${target}\` not found` });
    }
    roster.servers = next;
    if (roster.activeId === target) {
        roster.activeId = next[0]?.id;
    }
    return saveServerRoster(roster, filePath);
}

/** Select the active server. */
export function useServer(id: string, filePath = rosterPath()): ClientServerRoster {
    const roster = loadServerRoster(filePath);
    const target = assertId(id);
    if (!roster.servers.some((server) => server.id === target)) {
        throw new AppError({ code: 'invalid_argument', message: `server id \`${target}\` not found` });
    }
    roster.activeId = target;
    return saveServerRoster(roster, filePath);
}

/** Return the active server entry, if any. */
export function getActiveServer(roster: ClientServerRoster = loadServerRoster()): ManagedServerRef | undefined {
    if (!roster.activeId) return undefined;
    return roster.servers.find((server) => server.id === roster.activeId);
}

/** Snapshot used by CLI `--json` and GUI. */
export function rosterSnapshot(roster: ClientServerRoster = loadServerRoster()): RosterSnapshot {
    const active = getActiveServer(roster);
    return {
        path: rosterPath(),
        ...(roster.activeId ? { activeId: roster.activeId } : {}),
        ...(active ? { active } : {}),
        servers: roster.servers.map((server) => {
            const resolvedUrl = resolveServerUrl(server);
            return resolvedUrl ? { ...server, resolvedUrl } : { ...server };
        }),
    };
}

export { rosterPath } from './paths.js';
