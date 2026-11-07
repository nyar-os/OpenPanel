#!/usr/bin/env node
/**
 * Single installable CLI for OpenPanel.
 *
 * Entries:
 * - `open-panel gui`   desktop client (WAE WebView)
 * - `open-panel serve` local WebUI listener for browsers
 *
 * Both commands ship in the same `@wae/open-panel` package. There is no second
 * installer or parallel main binary.
 */
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { createCli, type CommandContext, type ParsedOptions } from '@wae/commander';

import { AppError, EXIT, fromNativeError } from './errors.js';
import {
    auditQuery,
    capabilities,
    configureAudit,
    runGui,
    runServe,
    version,
    type CapabilitiesReport,
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

cli.command('gui', 'cli.cmd.gui')
    .option('--content-dir <path>', 'cli.opt.content-dir')
    .option('--title <text>', 'cli.opt.title')
    .actionCtx(async (options, _args, ctx) => {
        try {
            bootstrap(options);
            const backend = await runGui({
                contentDir: stringOption(options, 'content-dir'),
                title: stringOption(options, 'title'),
            });
            emit(options, ctx, { backend }, `desktop client closed (backend=${backend})`);
            return EXIT.OK;
        }
        catch (error) {
            return fail(options, ctx, error);
        }
    });

cli.command('serve', 'cli.cmd.serve')
    .option('--host <host>', 'cli.opt.host')
    .option('--port <n>', 'cli.opt.port')
    .option('--content-dir <path>', 'cli.opt.content-dir')
    .actionCtx(async (options, _args, ctx) => {
        try {
            // WebUI hosting is TypeScript-side. Native audit is best-effort so a
            // machine without the desktop addon can still run the server entry.
            bootstrap(options, false);
            const handle = await runServe({
                host: stringOption(options, 'host'),
                port: integerOption(options, 'port'),
                contentDir: stringOption(options, 'content-dir'),
            });
            emit(
                options,
                ctx,
                { host: handle.host, port: handle.port, url: handle.url, contentDir: handle.contentDir },
                `OpenPanel WebUI listening on ${handle.url}\ncontent: ${handle.contentDir}\nPress Ctrl+C to stop.`,
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
