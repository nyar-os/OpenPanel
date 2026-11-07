/**
 * Lazy loader for the platform-native addon.
 */
import { existsSync } from 'node:fs';
import { createRequire } from 'node:module';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

import { AppError } from './errors.js';

const require = createRequire(import.meta.url);
const PLATFORM_PACKAGE = '@wae/open-panel-win32-x64';
const PLATFORM_DIR = PLATFORM_PACKAGE.replace('@wae/', '');
const WORKSPACE_ROOT = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..', '..', '..');

export interface BuildInfo {
    version: string;
    commit: string;
    builtAtUnix: number;
    target: string;
}

export interface NativeBindings {
    version(): string;
    buildInfo(): BuildInfo;
    capabilities(): unknown;
    platform(): string;
    openGui(options?: { contentDir?: string; title?: string }): unknown;
    pumpEvents(): boolean;
    closeGui(): boolean;
    isGuiOpen(): boolean;
    configureAudit(options: {
        actorKind?: string;
        actorId?: string;
        actorSource?: string;
        session?: string;
        host?: string;
    }): unknown;
    auditQuery(filter?: {
        kind?: string;
        id?: string;
        fromMs?: number;
        toMs?: number;
        capability?: string;
        limit?: number;
    }): string;
    listNativeServices(): unknown;
    nativeServiceStatus(options: { name: string }): unknown;
    startNativeService(options: { name: string }): unknown;
    stopNativeService(options: { name: string }): unknown;
    restartNativeService(options: { name: string }): unknown;
    listWebSites(): unknown;
    upsertWebSite(options: Record<string, string>): unknown;
    removeWebSite(options: { id: string }): boolean;
    setWebSiteEnabled(options: { id: string; enabled: string }): unknown;
    startWebSite(options: { id: string }): unknown;
    stopWebSite(options: { id: string }): unknown;
}

let cached: NativeBindings | null = null;

export function loadNative(): NativeBindings {
    if (cached) return cached;
    try {
        cached = require(PLATFORM_PACKAGE) as NativeBindings;
        return cached;
    }
    catch (error) {
        cached = null;
        if (error instanceof AppError) throw error;
        const message = error instanceof Error ? error.message : String(error);
        throw new AppError({
            code: 'native_unavailable',
            platform: `${process.platform}-${process.arch}`,
            message:
                `cannot load ${PLATFORM_PACKAGE}: ${message}\n` +
                'Run `pnpm run build:napi` from the repository root, ' +
                'or install the matching platform package.',
        });
    }
}

export function isWorkspace(): boolean {
    return existsSync(path.join(WORKSPACE_ROOT, 'pnpm-workspace.yaml'));
}

export function workspaceRoot(): string {
    return WORKSPACE_ROOT;
}

export function platformPackageDir(): string {
    return path.join(WORKSPACE_ROOT, 'packages', PLATFORM_DIR);
}
