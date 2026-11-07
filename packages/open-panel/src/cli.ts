#!/usr/bin/env node
/**
 * Single installable CLI for OpenPanel.
 *
 * Entries:
 * - `open-panel install` write local WebUI credentials (also via postinstall / npx)
 * - `open-panel client` desktop client (WAE WebView + `@wae/open-panel-gui`)
 * - `open-panel server` local WebUI listener (`@wae/open-panel-webui`)
 * - `open-panel servers` client-side roster CRUD
 *
 * Runtime entries ship in the same `@wae/open-panel` package. There is no
 * second installer or parallel main binary. `npx @wae/open-panel install` and
 * package `postinstall` call the same `runInstall` as this CLI command.
 */
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { createCli, type CommandContext, type ParsedOptions } from '@wae/commander';

import { AppError, EXIT, fromNativeError } from './errors.js';
import {
    auditQuery,
    capabilities,
    configureAudit,
    getActiveServer,
    loadServerRoster,
    formatInstallReport,
    installRemote,
    probeRemoteHost,
    removeServer,
    resolveServerUrl,
    rosterPath,
    rosterSnapshot,
    runClient,
    runInstall,
    runServer,
    upsertServer,
    useServer,
    version,
    type CapabilitiesReport,
    type ClientServerRoster,
    type RemoteChannel,
} from './index.js';

const here = path.dirname(fileURLToPath(import.meta.url));
const localesDir = path.join(here, '..', 'locales');

const cli = createCli('open-panel')
    .locales(localesDir)
    .intro('cli.intro')
    .option('--actor <id>', 'cli.opt.actor')
    .option('--actor-kind <kind>', 'cli.opt.actor-kind')
    .option('--actor-source <source>', 'cli.opt.actor-source')
    .option('--json', 'cli.opt.json');

function stringOption(options: ParsedOptions, key: string): string | undefined {
    const value = options[key];
    return typeof value === 'string' && value.trim() ? value : undefined;
}

function flag(options: ParsedOptions, key: string): boolean {
    return options[key] === true;
}

function integerOption(options: ParsedOptions, key: string): number | undefined {
    const raw = stringOption(options, key);
    if (raw === undefined) return undefined;
    const value = Number(raw);
    if (!Number.isInteger(value) || value < 0) {
        throw new AppError({ code: 'invalid_argument', message: `\`--${key}\` must be a non-negative integer` });
    }
    return value;
}

function bootstrap(options: ParsedOptions, requireNative = true): void {
    if (options._.length) {
        throw new AppError({
            code: 'invalid_argument',
            message: `unexpected argument \`${options._.join(' ')}\`; every input is a named option`,
        });
    }
    const kind = stringOption(options, 'actor-kind');
    if (kind !== undefined && kind !== 'user' && kind !== 'agent') {
        throw new AppError({ code: 'invalid_argument', message: '`--actor-kind` must be `user` or `agent`' });
    }
    try {
        configureAudit({
            actorKind: kind === 'agent' ? 'agent' : 'user',
            actorId: stringOption(options, 'actor') ?? 'unknown',
            actorSource: stringOption(options, 'actor-source') ?? 'cli',
        });
    }
    catch (error) {
        if (requireNative) throw error;
        const appError = fromNativeError(error);
        if (appError.code !== 'native_unavailable') throw error;
    }
}

function emit(options: ParsedOptions, ctx: CommandContext, data: unknown, human: string): void {
    if (flag(options, 'json')) {
        ctx.stdout(JSON.stringify({ ok: true, data }, null, 2));
        return;
    }
    if (human) ctx.stdout(human);
}

function fail(options: ParsedOptions, ctx: CommandContext, error: unknown): number {
    const appError = fromNativeError(error);
    if (flag(options, 'json')) {
        ctx.stdout(JSON.stringify({ ok: false, error: appError.toJSON() }, null, 2));
    }
    else {
        ctx.stderr(`${appError.code}: ${appError.message}`);
    }
    return appError.exitCode;
}

function formatCapabilities(report: CapabilitiesReport): string {
    const lines = report.items.map((item) => {
        const state = item.state ?? 'supported';
        const detail = item.detail ? ` — ${item.detail}` : '';
        return `${item.id}: ${state}${detail}`;
    });
    if (report.notes.length) lines.push('', ...report.notes.map((note) => `note: ${note}`));
    return lines.join('\n');
}

function formatRoster(roster: ClientServerRoster): string {
    const lines = [`roster: ${rosterPath()}`, `active: ${roster.activeId ?? '(none)'}`, `servers: ${roster.servers.length}`];
    if (!roster.servers.length) {
        lines.push('(empty — use `open-panel servers add`)');
        return lines.join('\n');
    }
    for (const server of roster.servers) {
        const mark = server.id === roster.activeId ? '*' : ' ';
        const label = server.label ? ` "${server.label}"` : '';
        const url = resolveServerUrl(server) ?? '(no url)';
        lines.push(`${mark} ${server.id}${label}  ${url}`);
    }
    return lines.join('\n');
}

cli.command('capabilities', 'cli.cmd.capabilities')
    .actionCtx(async (options, _args, ctx) => {
        try {
            bootstrap(options);
            const report = capabilities();
            emit(options, ctx, report, formatCapabilities(report));
            return EXIT.OK;
        }
        catch (error) {
            return fail(options, ctx, error);
        }
    });

cli.command('version', 'cli.cmd.version')
    .actionCtx(async (options, _args, ctx) => {
        try {
            bootstrap(options);
            const value = version();
            emit(options, ctx, { version: value }, value);
            return EXIT.OK;
        }
        catch (error) {
            return fail(options, ctx, error);
        }
    });

cli.command('audit', 'cli.cmd.audit')
    .option('--kind <kind>', 'cli.opt.kind')
    .option('--id <id>', 'cli.opt.id')
    .option('--capability <id>', 'cli.opt.capability')
    .option('--limit <n>', 'cli.opt.limit')
    .actionCtx(async (options, _args, ctx) => {
        try {
            bootstrap(options);
            const entries = auditQuery({
                kind: stringOption(options, 'kind'),
                id: stringOption(options, 'id'),
                capability: stringOption(options, 'capability'),
                limit: integerOption(options, 'limit'),
            });
            emit(options, ctx, entries, entries.length ? JSON.stringify(entries, null, 2) : '(no audit entries)');
            return EXIT.OK;
        }
        catch (error) {
            return fail(options, ctx, error);
        }
    });

const servers = cli.command('servers', 'cli.cmd.servers');

servers.command('list', 'cli.cmd.servers-list')
    .actionCtx(async (options, _args, ctx) => {
        try {
            bootstrap(options, false);
            const snapshot = rosterSnapshot();
            emit(options, ctx, snapshot, formatRoster(loadServerRoster()));
            return EXIT.OK;
        }
        catch (error) {
            return fail(options, ctx, error);
        }
    });

servers.command('add', 'cli.cmd.servers-add')
    .option('--id <id>', 'cli.opt.server-id')
    .option('--label <text>', 'cli.opt.server-label')
    .option('--url <url>', 'cli.opt.server-url')
    .option('--host <host>', 'cli.opt.host')
    .option('--port <n>', 'cli.opt.port')
    .actionCtx(async (options, _args, ctx) => {
        try {
            bootstrap(options, false);
            const id = stringOption(options, 'id');
            if (!id) throw new AppError({ code: 'invalid_argument', message: '`--id` is required' });
            const roster = upsertServer({
                id,
                label: stringOption(options, 'label'),
                url: stringOption(options, 'url'),
                host: stringOption(options, 'host'),
                port: integerOption(options, 'port'),
            });
            emit(options, ctx, rosterSnapshot(roster), formatRoster(roster));
            return EXIT.OK;
        }
        catch (error) {
            return fail(options, ctx, error);
        }
    });

servers.command('remove', 'cli.cmd.servers-remove')
    .option('--id <id>', 'cli.opt.server-id')
    .actionCtx(async (options, _args, ctx) => {
        try {
            bootstrap(options, false);
            const id = stringOption(options, 'id');
            if (!id) throw new AppError({ code: 'invalid_argument', message: '`--id` is required' });
            const roster = removeServer(id);
            emit(options, ctx, rosterSnapshot(roster), formatRoster(roster));
            return EXIT.OK;
        }
        catch (error) {
            return fail(options, ctx, error);
        }
    });

servers.command('use', 'cli.cmd.servers-use')
    .option('--id <id>', 'cli.opt.server-id')
    .actionCtx(async (options, _args, ctx) => {
        try {
            bootstrap(options, false);
            const id = stringOption(options, 'id');
            if (!id) throw new AppError({ code: 'invalid_argument', message: '`--id` is required' });
            const roster = useServer(id);
            const active = getActiveServer(roster);
            emit(
                options,
                ctx,
                rosterSnapshot(roster),
                `active server: ${active?.id ?? id}${active ? ` → ${resolveServerUrl(active) ?? '(no url)'}` : ''}`,
            );
            return EXIT.OK;
        }
        catch (error) {
            return fail(options, ctx, error);
        }
    });

servers.command('path', 'cli.cmd.servers-path')
    .actionCtx(async (options, _args, ctx) => {
        try {
            bootstrap(options, false);
            const file = rosterPath();
            emit(options, ctx, { path: file }, file);
            return EXIT.OK;
        }
        catch (error) {
            return fail(options, ctx, error);
        }
    });

const remote = cli.command('remote', 'cli.cmd.remote');

remote.command('probe', 'cli.cmd.remote-probe')
    .option('--host <host>', 'cli.opt.host')
    .actionCtx(async (options, _args, ctx) => {
        try {
            bootstrap(options, false);
            const host = stringOption(options, 'host');
            if (!host) throw new AppError({ code: 'invalid_argument', message: '`--host` is required' });
            const report = await probeRemoteHost(host);
            const lines = [
                `host: ${report.host}`,
                `recommended: ${report.recommendedChannel}`,
                ...report.ports.map((port) => `${port.name}:${port.port} ${port.open ? 'open' : 'closed'}${port.latencyMs != null ? ` ${port.latencyMs}ms` : ''}`),
                ...report.notes.map((note) => `note: ${note}`),
            ];
            emit(options, ctx, report, lines.join('\n'));
            return EXIT.OK;
        }
        catch (error) {
            return fail(options, ctx, error);
        }
    });

remote.command('install', 'cli.cmd.remote-install')
    .option('--host <host>', 'cli.opt.host')
    .option('--user <name>', 'cli.opt.remote-user')
    .option('--password-env <name>', 'cli.opt.password-env')
    .option('--channel <name>', 'cli.opt.remote-channel')
    .option('--server-port <n>', 'cli.opt.port')
    .option('--server-id <id>', 'cli.opt.server-id')
    .option('--label <text>', 'cli.opt.server-label')
    .option('--skip-start', 'cli.opt.skip-start')
    .option('--package-url <url>', 'cli.opt.package-url')
    .option('--remote-tarball <path>', 'cli.opt.remote-tarball')
    .option('--pack-base-url <url>', 'cli.opt.pack-base-url')
    .actionCtx(async (options, _args, ctx) => {
        try {
            bootstrap(options, false);
            const host = stringOption(options, 'host');
            const user = stringOption(options, 'user');
            const passwordEnv = stringOption(options, 'password-env');
            if (!host) throw new AppError({ code: 'invalid_argument', message: '`--host` is required' });
            if (!user) throw new AppError({ code: 'invalid_argument', message: '`--user` is required' });
            if (!passwordEnv) throw new AppError({ code: 'invalid_argument', message: '`--password-env` is required' });
            const channelRaw = stringOption(options, 'channel') ?? 'auto';
            const allowed = ['auto', 'rdp', 'ssh', 'winrm', 'none'] as const;
            if (!(allowed as readonly string[]).includes(channelRaw)) {
                throw new AppError({ code: 'invalid_argument', message: '`--channel` must be `auto`, `rdp`, `ssh`, `winrm`, or `none`' });
            }
            const report = await installRemote([{
                host,
                user,
                passwordEnv,
                channel: channelRaw === 'auto' ? 'auto' : channelRaw as RemoteChannel,
                serverPort: integerOption(options, 'server-port'),
                serverId: stringOption(options, 'server-id'),
                label: stringOption(options, 'label'),
                skipStart: flag(options, 'skip-start'),
                packageUrl: stringOption(options, 'package-url'),
                remoteTarball: stringOption(options, 'remote-tarball'),
                packBaseUrl: stringOption(options, 'pack-base-url'),
            }]);
            const lines = [
                `pack: ${report.pack.tarballPath}`,
                `note: ${report.pack.note}`,
                ...report.results.map((item) => {
                    if (item.ok) {
                        return `ok ${item.host} channel=${item.channel} url=${item.url ?? ''} roster=${item.rosterId ?? ''}`;
                    }
                    const next = item.next ? ` next=${item.next}` : '';
                    return `fail ${item.host}: ${item.error ?? 'unknown error'}${next}`;
                }),
            ];
            emit(options, ctx, report, lines.join('\n'));
            return report.ok ? EXIT.OK : EXIT.FAILURE;
        }
        catch (error) {
            return fail(options, ctx, error);
        }
    });


cli.command('install', 'cli.cmd.install')
    .option('--force', 'cli.opt.install-force')
    .option('--port <n>', 'cli.opt.port')
    .option('--advertise-host <host>', 'cli.opt.advertise-host')
    .actionCtx(async (options, _args, ctx) => {
        try {
            bootstrap(options, false);
            const report = runInstall({
                force: flag(options, 'force'),
                port: integerOption(options, 'port'),
                advertiseHost: stringOption(options, 'advertise-host'),
            });
            const publicData = {
                ok: report.ok,
                created: report.created,
                reused: report.reused,
                path: report.path,
                configDir: report.configDir,
                username: report.username,
                port: report.port,
                loginUrl: report.loginUrl,
                note: report.note,
                ...(report.created && report.password ? { password: report.password } : {}),
            };
            emit(options, ctx, publicData, formatInstallReport(report));
            return EXIT.OK;
        }
        catch (error) {
            return fail(options, ctx, error);
        }
    });
cli.command('client', 'cli.cmd.client')
    .option('--content-dir <path>', 'cli.opt.content-dir')
    .option('--title <text>', 'cli.opt.title')
    .option('--server-id <id>', 'cli.opt.server-id')
    .actionCtx(async (options, _args, ctx) => {
        try {
            bootstrap(options);
            const backend = await runClient({
                contentDir: stringOption(options, 'content-dir'),
                title: stringOption(options, 'title'),
                serverId: stringOption(options, 'server-id'),
            });
            const active = getActiveServer();
            emit(
                options,
                ctx,
                { backend, active: active ? rosterSnapshot().active : undefined },
                `desktop client closed (backend=${backend}${active ? `, active=${active.id}` : ''})`,
            );
            return EXIT.OK;
        }
        catch (error) {
            return fail(options, ctx, error);
        }
    });

cli.command('server', 'cli.cmd.server')
    .option('--host <host>', 'cli.opt.host')
    .option('--port <n>', 'cli.opt.port')
    .option('--content-dir <path>', 'cli.opt.content-dir')
    .actionCtx(async (options, _args, ctx) => {
        try {
            // WebUI hosting is TypeScript-side. Native audit is best-effort so a
            // machine without the desktop addon can still run the server entry.
            bootstrap(options, false);
            const handle = await runServer({
                host: stringOption(options, 'host'),
                port: integerOption(options, 'port'),
                contentDir: stringOption(options, 'content-dir'),
            });
            emit(
                options,
                ctx,
                {
                    host: handle.host,
                    port: handle.port,
                    url: handle.url,
                    loginUrl: handle.loginUrl,
                    username: handle.username,
                    contentDir: handle.contentDir,
                },
                [
                    `OpenPanel WebUI listening on ${handle.loginUrl}`,
                    `user: ${handle.username}`,
                    `content: ${handle.contentDir}`,
                    'Press Ctrl+C to stop.',
                ].join('\n'),
            );
            await new Promise<void>((resolve) => {
                const stop = () => {
                    process.off('SIGINT', stop);
                    process.off('SIGTERM', stop);
                    resolve();
                };
                process.on('SIGINT', stop);
                process.on('SIGTERM', stop);
            });
            await handle.close();
            return EXIT.OK;
        }
        catch (error) {
            return fail(options, ctx, error);
        }
    });

const code = await cli.parse(process.argv);
process.exitCode = code;
