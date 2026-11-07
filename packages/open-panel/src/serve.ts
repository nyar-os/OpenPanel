/**
 * Local WebUI listener for `open-panel serve`.
 *
 * This is the server entry of the same installable. It hosts the staged panel
 * assets over HTTP so a browser can open the ops surface. Desktop clients keep
 * using `open-panel gui` through the WebView host.
 */
import fs from 'node:fs';
import http from 'node:http';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { AppError } from './errors.js';
import { workspaceRoot } from './native.js';

export interface ServeOptions {
    host?: string;
    port?: number;
    contentDir?: string;
}

export interface ServeHandle {
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
    if (process.env.OPEN_PANEL_GUI?.trim()) roots.push(path.resolve(process.env.OPEN_PANEL_GUI));
    const here = path.dirname(fileURLToPath(import.meta.url));
    roots.push(
        path.resolve(here, '..', 'dist', 'gui'),
        path.resolve(workspaceRoot(), 'packages', 'open-panel', 'dist', 'gui'),
        path.resolve(workspaceRoot(), 'packages', 'open-panel-ui', 'dist', 'static'),
    );
    return roots;
}

export function resolveContentDir(explicit?: string): string {
    for (const candidate of contentCandidates(explicit)) {
        if (fs.existsSync(path.join(candidate, 'index.html'))) return candidate;
    }
    throw new AppError({
        code: 'content_dir_missing',
        capability: 'serve',
        message:
            'WebUI content directory not found. Run `pnpm run build:ui` first, ' +
            'pass `--content-dir`, or set `OPEN_PANEL_GUI`.',
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

/** Start the local WebUI listener and return a handle. */
export async function startServe(options: ServeOptions = {}): Promise<ServeHandle> {
    const host = options.host?.trim() || '127.0.0.1';
    const port = options.port ?? 17890;
    if (!Number.isInteger(port) || port < 1 || port > 65535) {
        throw new AppError({ code: 'invalid_argument', message: '`--port` must be an integer between 1 and 65535' });
    }
    const contentDir = resolveContentDir(options.contentDir);

    const server = http.createServer((request, response) => {
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
