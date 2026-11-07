/**
 * Local WebUI listener for `open-panel server`.
 *
 * This is the server entry of the same installable. It hosts the staged WebUI
 * assets over HTTP so a browser can open the ops surface. Desktop clients keep
 * using `open-panel client` through the WebView host and `@wae/open-panel-gui`.
 */
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { AppError, fromNativeError } from './errors.js';
import { workspaceRoot } from './native.js';
import {
    listNativeServices,
    listWebSites,
    nativeServiceStatus,
    removeWebSite,
    restartNativeService,
    setWebSiteEnabled,
    startNativeService,
    startWebSite,
    stopNativeService,
    stopWebSite,
    upsertWebSite,
    type WebSiteInput,
} from './panel-ops.js';

export interface ServerOptions {
    host?: string;
    port?: number;
    contentDir?: string;
}

export interface ServerHandle {
    host: string;
    port: number;
    url: string;
    contentDir: string;
    close(): Promise<void>;
}

const MIME: Record<string, string> = {
    '.html': 'text/html; charset=utf-8',
    '.js': 'text/javascript; charset=utf-8',
    '.css': 'text/css; charset=utf-8',
    '.json': 'application/json; charset=utf-8',
    '.svg': 'image/svg+xml',
    '.png': 'image/png',
    '.ico': 'image/x-icon',
    '.map': 'application/json; charset=utf-8',
    '.txt': 'text/plain; charset=utf-8',
};

function contentCandidates(explicit?: string): string[] {
    const roots: string[] = [];
    if (explicit && explicit.trim()) roots.push(path.resolve(explicit));
    if (process.env.OPEN_PANEL_WEBUI?.trim()) roots.push(path.resolve(process.env.OPEN_PANEL_WEBUI));
    const here = path.dirname(fileURLToPath(import.meta.url));
    roots.push(
        path.resolve(here, '..', 'dist', 'webui'),
        path.resolve(workspaceRoot(), 'packages', 'open-panel', 'dist', 'webui'),
        path.resolve(workspaceRoot(), 'packages', 'open-panel-webui', 'dist', 'static'),
    );
    return roots;
}

export function resolveContentDir(explicit?: string): string {
    for (const candidate of contentCandidates(explicit)) {
        if (fs.existsSync(path.join(candidate, 'index.html'))) return candidate;
    }
    throw new AppError({
        code: 'content_dir_missing',
        capability: 'server',
        message:
            'WebUI content directory not found. Run `pnpm run build:webui` first, ' +
            'pass `--content-dir`, or set `OPEN_PANEL_WEBUI`.',
    });
}

function contentType(file: string): string {
    return MIME[path.extname(file).toLowerCase()] ?? 'application/octet-stream';
}

function safeJoin(root: string, urlPath: string): string | null {
    const decoded = decodeURIComponent(urlPath.split('?')[0] || '/');
    const relative = decoded === '/' ? 'index.html' : decoded.replace(/^\/+/, '');
    const resolved = path.resolve(root, relative);
    const rootWithSep = root.endsWith(path.sep) ? root : `${root}${path.sep}`;
    if (resolved !== root && !resolved.startsWith(rootWithSep)) return null;
    return resolved;
}

function requestPath(url: string | undefined): string {
    return decodeURIComponent((url ?? '/').split('?')[0] || '/');
}

/** Read-only host identity for multi-server status pulls. */
export function hostStatus(): Record<string, unknown> {
    return {
        hostname: os.hostname(),
        platform: process.platform,
        arch: process.arch,
        release: os.release(),
        type: os.type(),
        uptimeSec: Math.round(os.uptime()),
        node: process.version,
        pid: process.pid,
        cwd: process.cwd(),
    };
}

function writeJson(response: http.ServerResponse, status: number, body: unknown): void {
    response.writeHead(status, {
        'content-type': 'application/json; charset=utf-8',
        'cache-control': 'no-store',
    });
    response.end(`${JSON.stringify(body, null, 2)}\n`);
}

function writeAppError(response: http.ServerResponse, error: unknown): void {
    const appError = error instanceof AppError ? error : fromNativeError(error);
    const status = appError.code === 'invalid_argument' || appError.code === 'not_found' ? 400 : 500;
    writeJson(response, status, { ok: false, error: appError.toJSON() });
}

function readBody(request: http.IncomingMessage): Promise<string> {
    return new Promise((resolve, reject) => {
        const chunks: Buffer[] = [];
        request.on('data', (chunk) => {
            chunks.push(Buffer.isBuffer(chunk) ? chunk : Buffer.from(chunk));
            if (chunks.reduce((sum, item) => sum + item.length, 0) > 1_000_000) {
                reject(new AppError({ code: 'invalid_argument', message: 'request body too large' }));
                request.destroy();
            }
        });
        request.on('end', () => resolve(Buffer.concat(chunks).toString('utf8')));
        request.on('error', reject);
    });
}

async function readJsonBody(request: http.IncomingMessage): Promise<Record<string, unknown>> {
    const raw = (await readBody(request)).trim();
    if (!raw) return {};
    try {
        const value = JSON.parse(raw) as unknown;
        if (!value || typeof value !== 'object' || Array.isArray(value)) {
            throw new AppError({ code: 'invalid_argument', message: 'JSON body must be an object' });
        }
        return value as Record<string, unknown>;
    }
    catch (error) {
        if (error instanceof AppError) throw error;
        throw new AppError({ code: 'invalid_argument', message: 'invalid JSON body' });
    }
}

function normalizeApiPath(pathname: string): string {
    if (pathname.startsWith('/api/v0/')) return pathname.slice('/api/v0'.length);
    if (pathname.startsWith('/api/')) return pathname.slice('/api'.length);
    return pathname;
}

async function handleApi(
    request: http.IncomingMessage,
    response: http.ServerResponse,
    pathname: string,
): Promise<boolean> {
    if (!pathname.startsWith('/api/')) return false;
    const apiPath = normalizeApiPath(pathname);
    const method = request.method ?? 'GET';

    try {
        if (method === 'GET' && apiPath === '/host') {
            writeJson(response, 200, { ok: true, data: hostStatus() });
            return true;
        }

        if (method === 'GET' && apiPath === '/services') {
            writeJson(response, 200, { ok: true, data: listNativeServices() });
            return true;
        }
        const serviceMatch = /^\/services\/([^/]+)(?:\/(start|stop|restart))?$/.exec(apiPath);
        if (serviceMatch) {
            const name = decodeURIComponent(serviceMatch[1] ?? '');
            const action = serviceMatch[2];
            if (method === 'GET' && !action) {
                writeJson(response, 200, { ok: true, data: nativeServiceStatus(name) });
                return true;
            }
            if (method === 'POST' && action === 'start') {
                writeJson(response, 200, { ok: true, data: startNativeService(name) });
                return true;
            }
            if (method === 'POST' && action === 'stop') {
                writeJson(response, 200, { ok: true, data: stopNativeService(name) });
                return true;
            }
            if (method === 'POST' && action === 'restart') {
                writeJson(response, 200, { ok: true, data: restartNativeService(name) });
                return true;
            }
        }

        if (method === 'GET' && apiPath === '/sites') {
            writeJson(response, 200, { ok: true, data: listWebSites() });
            return true;
        }
        if (method === 'POST' && apiPath === '/sites') {
            const body = await readJsonBody(request);
            const input = body as Partial<WebSiteInput>;
            if (typeof input.id !== 'string' || typeof input.domain !== 'string' || typeof input.rootDir !== 'string') {
                throw new AppError({
                    code: 'invalid_argument',
                    message: '`id`, `domain`, and `rootDir` are required',
                });
            }
            const port = typeof input.port === 'number' ? input.port : Number(input.port);
            if (!Number.isInteger(port) || port < 1 || port > 65535) {
                throw new AppError({ code: 'invalid_argument', message: '`port` must be an integer between 1 and 65535' });
            }
            const view = upsertWebSite({
                id: input.id,
                domain: input.domain,
                rootDir: input.rootDir,
                host: typeof input.host === 'string' ? input.host : undefined,
                port,
                enabled: typeof input.enabled === 'boolean' ? input.enabled : undefined,
                kind: input.kind === 'static-host' ? 'static-host' : undefined,
            });
            writeJson(response, 200, { ok: true, data: view });
            return true;
        }
        const siteMatch = /^\/sites\/([^/]+)(?:\/(enable|disable|start|stop))?$/.exec(apiPath);
        if (siteMatch) {
            const id = decodeURIComponent(siteMatch[1] ?? '');
            const action = siteMatch[2];
            if (method === 'DELETE' && !action) {
                writeJson(response, 200, { ok: true, data: { removed: removeWebSite(id) } });
                return true;
            }
            if (method === 'POST' && action === 'enable') {
                writeJson(response, 200, { ok: true, data: setWebSiteEnabled(id, true) });
                return true;
            }
            if (method === 'POST' && action === 'disable') {
                writeJson(response, 200, { ok: true, data: setWebSiteEnabled(id, false) });
                return true;
            }
            if (method === 'POST' && action === 'start') {
                writeJson(response, 200, { ok: true, data: startWebSite(id) });
                return true;
            }
            if (method === 'POST' && action === 'stop') {
                writeJson(response, 200, { ok: true, data: stopWebSite(id) });
                return true;
            }
        }

        writeJson(response, 404, {
            ok: false,
            error: { code: 'not_found', message: `unknown api path \`${pathname}\`` },
        });
        return true;
    }
    catch (error) {
        writeAppError(response, error);
        return true;
    }
}

/** Start the local WebUI listener and return a handle. */
export async function startServer(options: ServerOptions = {}): Promise<ServerHandle> {
    const host = options.host?.trim() || '127.0.0.1';
    const port = options.port ?? 17890;
    if (!Number.isInteger(port) || port < 1 || port > 65535) {
        throw new AppError({ code: 'invalid_argument', message: '`--port` must be an integer between 1 and 65535' });
    }
    const contentDir = resolveContentDir(options.contentDir);

    const server = http.createServer((request, response) => {
        const pathname = requestPath(request.url);
        if (pathname.startsWith('/api/')) {
            void handleApi(request, response, pathname);
            return;
        }
        const target = safeJoin(contentDir, request.url ?? '/');
        if (!target) {
            response.writeHead(403, { 'content-type': 'text/plain; charset=utf-8' });
            response.end('forbidden');
            return;
        }
        const file = fs.existsSync(target) && fs.statSync(target).isDirectory()
            ? path.join(target, 'index.html')
            : target;
        if (!fs.existsSync(file) || !fs.statSync(file).isFile()) {
            response.writeHead(404, { 'content-type': 'text/plain; charset=utf-8' });
            response.end('not found');
            return;
        }
        response.writeHead(200, { 'content-type': contentType(file) });
        fs.createReadStream(file).pipe(response);
    });

    await new Promise<void>((resolve, reject) => {
        server.once('error', reject);
        server.listen(port, host, () => resolve());
    });

    const address = server.address();
    const boundPort = typeof address === 'object' && address ? address.port : port;
    return {
        host,
        port: boundPort,
        url: `http://${host}:${boundPort}/`,
        contentDir,
        close: () => new Promise((resolve, reject) => {
            server.close((error) => (error ? reject(error) : resolve()));
        }),
    };
}
