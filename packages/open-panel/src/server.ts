/**
 * Local WebUI listener for `open-panel server`.
 *
 * This is the server entry of the same installable. It hosts the staged WebUI
 * assets over HTTP so a browser can open the ops surface. Desktop clients keep
 * using `open-panel client` through the WebView host and `@wae/open-panel-gui`.
 *
 * Credentials and the default listen port come from `open-panel install`
 * (`webui-credentials.json`). This module never generates a password.
 */
import crypto from 'node:crypto';
import fs from 'node:fs';
import http from 'node:http';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { loadCredentials, type WebuiCredentials } from './credentials.js';
import { DEFAULT_WEBUI_PORT, WEBUI_USERNAME, webuiLoginUrl } from './defaults.js';
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
    loginUrl: string;
    contentDir: string;
    username: string;
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

const SESSION_COOKIE = 'open_panel_session';
const SESSION_TTL_MS = 12 * 60 * 60 * 1000;

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
    const status = appError.code === 'invalid_argument' || appError.code === 'not_found'
        ? 400
        : appError.code === 'unauthorized'
            ? 401
            : appError.code === 'not_installed'
                ? 503
                : 500;
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

function parseCookies(header: string | undefined): Record<string, string> {
    const out: Record<string, string> = {};
    if (!header) return out;
    for (const part of header.split(';')) {
        const idx = part.indexOf('=');
        if (idx <= 0) continue;
        const key = part.slice(0, idx).trim();
        const value = part.slice(idx + 1).trim();
        if (key) out[key] = decodeURIComponent(value);
    }
    return out;
}

function timingSafeEqualString(a: string, b: string): boolean {
    const left = Buffer.from(a);
    const right = Buffer.from(b);
    if (left.length !== right.length) {
        crypto.timingSafeEqual(left, left);
        return false;
    }
    return crypto.timingSafeEqual(left, right);
}

function parseBasicAuth(header: string | undefined): { username: string; password: string } | null {
    if (!header || !header.startsWith('Basic ')) return null;
    try {
        const decoded = Buffer.from(header.slice('Basic '.length).trim(), 'base64').toString('utf8');
        const idx = decoded.indexOf(':');
        if (idx < 0) return null;
        return { username: decoded.slice(0, idx), password: decoded.slice(idx + 1) };
    }
    catch {
        return null;
    }
}

function isPublicPath(pathname: string): boolean {
    if (pathname === '/login' || pathname === '/login/') return true;
    if (pathname === '/api/v0/login' || pathname === '/api/login') return true;
    if (pathname === '/api/v0/logout' || pathname === '/api/logout') return true;
    if (pathname.startsWith('/assets/')) return true;
    if (pathname.startsWith('/_vmz/')) return true;
    if (pathname === '/favicon.ico' || pathname === '/robots.txt') return true;
    return false;
}

function resolveLoginFile(contentDir: string): string | null {
    const candidates = [
        path.join(contentDir, 'login.html'),
        path.join(contentDir, 'login', 'index.html'),
    ];
    for (const candidate of candidates) {
        if (fs.existsSync(candidate) && fs.statSync(candidate).isFile()) return candidate;
    }
    return null;
}

function fallbackLoginHtml(): string {
    return `<!DOCTYPE html>
<html lang="en">
<head>
  <meta charset="utf-8" />
  <meta name="viewport" content="width=device-width, initial-scale=1" />
  <title>OpenPanel Login</title>
  <style>
    body { margin: 0; min-height: 100vh; display: grid; place-items: center;
      font-family: "Segoe UI", system-ui, sans-serif; color: #e7f2ec;
      background: radial-gradient(circle at 88% 12%, rgba(61,155,110,.18), transparent 40%),
        linear-gradient(160deg, #101a16, #0b1210 60%, #121c18); }
    form { width: min(22rem, calc(100% - 2rem)); display: grid; gap: .75rem; }
    label { font-size: .85rem; color: #9bb5a8; }
    input { padding: .65rem .75rem; border-radius: .45rem; border: 1px solid #2e4038;
      background: rgba(8,14,12,.55); color: inherit; }
    button { padding: .7rem .9rem; border: 0; border-radius: .45rem; cursor: pointer;
      background: #3d9b6e; color: #06110c; font-weight: 700; }
    .brand { margin: 0 0 .5rem; color: #54ad7f; letter-spacing: .08em;
      text-transform: uppercase; font-size: .8rem; font-weight: 700; }
    .err { color: #f0a5a5; min-height: 1.2rem; font-size: .85rem; }
  </style>
</head>
<body>
  <form id="login">
    <p class="brand">OpenPanel</p>
    <h1 style="margin:0 0 .25rem;font-size:1.6rem">Sign in</h1>
    <p style="margin:0 0 .5rem;color:#9bb5a8;font-size:.9rem">User is fixed as <code>administrator</code>.</p>
    <label for="username">Username</label>
    <input id="username" name="username" autocomplete="username" value="administrator" required />
    <label for="password">Password</label>
    <input id="password" name="password" type="password" autocomplete="current-password" required />
    <p class="err" id="err"></p>
    <button type="submit">Sign in</button>
  </form>
  <script>
    const form = document.getElementById('login');
    const err = document.getElementById('err');
    form.addEventListener('submit', async (event) => {
      event.preventDefault();
      err.textContent = '';
      const body = {
        username: document.getElementById('username').value,
        password: document.getElementById('password').value,
      };
      try {
        const response = await fetch('/api/v0/login', {
          method: 'POST',
          headers: { 'content-type': 'application/json' },
          credentials: 'same-origin',
          body: JSON.stringify(body),
        });
        const payload = await response.json();
        if (!response.ok || !payload.ok) {
          err.textContent = (payload && payload.error && payload.error.message) || 'login failed';
          return;
        }
        location.href = '/';
      } catch (error) {
        err.textContent = error instanceof Error ? error.message : String(error);
      }
    });
  </script>
</body>
</html>
`;
}

function setSessionCookie(response: http.ServerResponse, token: string): void {
    const maxAge = Math.floor(SESSION_TTL_MS / 1000);
    response.setHeader(
        'set-cookie',
        `${SESSION_COOKIE}=${encodeURIComponent(token)}; Path=/; HttpOnly; SameSite=Strict; Max-Age=${maxAge}`,
    );
}

function clearSessionCookie(response: http.ServerResponse): void {
    response.setHeader(
        'set-cookie',
        `${SESSION_COOKIE}=; Path=/; HttpOnly; SameSite=Strict; Max-Age=0`,
    );
}

type SessionStore = Map<string, number>;

function createSession(store: SessionStore): string {
    const token = crypto.randomBytes(32).toString('base64url');
    store.set(token, Date.now() + SESSION_TTL_MS);
    return token;
}

function sessionValid(store: SessionStore, token: string | undefined): boolean {
    if (!token) return false;
    const expires = store.get(token);
    if (expires === undefined) return false;
    if (expires < Date.now()) {
        store.delete(token);
        return false;
    }
    return true;
}

function credentialsMatch(creds: WebuiCredentials, username: string, password: string): boolean {
    return timingSafeEqualString(username, creds.username)
        && timingSafeEqualString(password, creds.password);
}

function isAuthorized(
    request: http.IncomingMessage,
    creds: WebuiCredentials,
    sessions: SessionStore,
): boolean {
    const cookies = parseCookies(request.headers.cookie);
    if (sessionValid(sessions, cookies[SESSION_COOKIE])) return true;
    const basic = parseBasicAuth(request.headers.authorization);
    if (basic && credentialsMatch(creds, basic.username, basic.password)) return true;
    return false;
}

function redirectLogin(response: http.ServerResponse): void {
    response.writeHead(302, {
        location: '/login',
        'cache-control': 'no-store',
    });
    response.end();
}

function challengeUnauthorized(response: http.ServerResponse, api: boolean): void {
    if (api) {
        writeJson(response, 401, {
            ok: false,
            error: { code: 'unauthorized', message: 'authentication required' },
        });
        return;
    }
    redirectLogin(response);
}

async function handleApi(
    request: http.IncomingMessage,
    response: http.ServerResponse,
    pathname: string,
    creds: WebuiCredentials,
    sessions: SessionStore,
): Promise<boolean> {
    if (!pathname.startsWith('/api/')) return false;
    const apiPath = normalizeApiPath(pathname);
    const method = request.method ?? 'GET';

    try {
        if (method === 'POST' && apiPath === '/login') {
            const body = await readJsonBody(request);
            const username = typeof body.username === 'string' ? body.username : '';
            const password = typeof body.password === 'string' ? body.password : '';
            if (!credentialsMatch(creds, username, password)) {
                writeJson(response, 401, {
                    ok: false,
                    error: { code: 'unauthorized', message: 'invalid username or password' },
                });
                return true;
            }
            const token = createSession(sessions);
            setSessionCookie(response, token);
            writeJson(response, 200, {
                ok: true,
                data: { username: WEBUI_USERNAME },
            });
            return true;
        }
        if (method === 'POST' && apiPath === '/logout') {
            const cookies = parseCookies(request.headers.cookie);
            const token = cookies[SESSION_COOKIE];
            if (token) sessions.delete(token);
            clearSessionCookie(response);
            writeJson(response, 200, { ok: true, data: { loggedOut: true } });
            return true;
        }

        if (!isAuthorized(request, creds, sessions)) {
            challengeUnauthorized(response, true);
            return true;
        }

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

function serveStaticFile(response: http.ServerResponse, file: string): void {
    response.writeHead(200, { 'content-type': contentType(file) });
    fs.createReadStream(file).pipe(response);
}

/** Resolve the listen port: CLI override, else install-time port, else product default. */
export function resolveListenPort(cliPort: number | undefined, installedPort: number): number {
    if (cliPort !== undefined) return cliPort;
    if (Number.isInteger(installedPort) && installedPort >= 1 && installedPort <= 65535) {
        return installedPort;
    }
    return DEFAULT_WEBUI_PORT;
}

/** Start the local WebUI listener and return a handle. */
export async function startServer(options: ServerOptions = {}): Promise<ServerHandle> {
    const creds = loadCredentials();
    const host = options.host?.trim() || '127.0.0.1';
    const port = resolveListenPort(options.port, creds.port);
    if (!Number.isInteger(port) || port < 1 || port > 65535) {
        throw new AppError({ code: 'invalid_argument', message: '`--port` must be an integer between 1 and 65535' });
    }
    const contentDir = resolveContentDir(options.contentDir);
    const sessions: SessionStore = new Map();

    const server = http.createServer((request, response) => {
        const pathname = requestPath(request.url);

        if (pathname.startsWith('/api/')) {
            void handleApi(request, response, pathname, creds, sessions);
            return;
        }

        if (pathname === '/login' || pathname === '/login/') {
            const loginFile = resolveLoginFile(contentDir);
            if (loginFile) {
                serveStaticFile(response, loginFile);
                return;
            }
            response.writeHead(200, {
                'content-type': 'text/html; charset=utf-8',
                'cache-control': 'no-store',
            });
            response.end(fallbackLoginHtml());
            return;
        }

        if (!isPublicPath(pathname) && !isAuthorized(request, creds, sessions)) {
            redirectLogin(response);
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
        serveStaticFile(response, file);
    });

    await new Promise<void>((resolve, reject) => {
        server.once('error', reject);
        server.listen(port, host, () => resolve());
    });

    const address = server.address();
    const boundPort = typeof address === 'object' && address ? address.port : port;
    const loginUrl = webuiLoginUrl(host, boundPort);
    return {
        host,
        port: boundPort,
        url: `http://${host}:${boundPort}/`,
        loginUrl,
        contentDir,
        username: WEBUI_USERNAME,
        close: () => new Promise((resolve, reject) => {
            server.close((error) => (error ? reject(error) : resolve()));
        }),
    };
}
