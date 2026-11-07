/**
 * WebUI credentials written by `open-panel install` and read by `open-panel server`.
 *
 * Secrets live only under the per-user config directory — never in the repo tree.
 */
import crypto from 'node:crypto';
import fs from 'node:fs';
import path from 'node:path';

import {
    DEFAULT_WEBUI_PORT,
    WEBUI_PASSWORD_ALPHABET,
    WEBUI_PASSWORD_LENGTH,
    WEBUI_PASSWORD_MIN_LENGTH,
    WEBUI_USERNAME,
} from './defaults.js';
import { AppError } from './errors.js';
import { ensureConfigDir, resolveConfigDir } from './paths.js';

const CREDENTIALS_FILE = 'webui-credentials.json';

/** On-disk shape produced by install. */
export interface WebuiCredentials {
    username: string;
    password: string;
    port: number;
    createdAt: string;
}

export interface GeneratePasswordOptions {
    length?: number;
    alphabet?: string;
}

/** Path of the persisted WebUI credentials file. */
export function credentialsPath(): string {
    return path.join(resolveConfigDir(), CREDENTIALS_FILE);
}

/**
 * Generate a high-entropy password suitable for first-time install.
 *
 * Uses `crypto.randomInt` over a curated alphabet. Rejects short / tiny alphabets.
 */
export function generateStrongPassword(options: GeneratePasswordOptions = {}): string {
    const length = options.length ?? WEBUI_PASSWORD_LENGTH;
    const alphabet = options.alphabet ?? WEBUI_PASSWORD_ALPHABET;
    if (!Number.isInteger(length) || length < WEBUI_PASSWORD_MIN_LENGTH) {
        throw new AppError({
            code: 'invalid_argument',
            message: `password length must be an integer >= ${WEBUI_PASSWORD_MIN_LENGTH}`,
        });
    }
    if (alphabet.length < 32) {
        throw new AppError({
            code: 'invalid_argument',
            message: 'password alphabet must contain at least 32 distinct characters',
        });
    }
    let out = '';
    for (let i = 0; i < length; i += 1) {
        out += alphabet[crypto.randomInt(0, alphabet.length)]!;
    }
    return out;
}

/** Estimate bits of entropy for a uniform random password over `alphabet`. */
export function passwordEntropyBits(password: string, alphabet: string = WEBUI_PASSWORD_ALPHABET): number {
    if (!password.length || alphabet.length < 2) return 0;
    return password.length * Math.log2(alphabet.length);
}

/** Assert a candidate meets the install strength floor (length + entropy). */
export function assertPasswordStrength(password: string, alphabet: string = WEBUI_PASSWORD_ALPHABET): void {
    if (password.length < WEBUI_PASSWORD_MIN_LENGTH) {
        throw new AppError({
            code: 'invalid_argument',
            message: `WebUI password must be at least ${WEBUI_PASSWORD_MIN_LENGTH} characters`,
        });
    }
    const bits = passwordEntropyBits(password, alphabet);
    // 24 chars × log2(72) ≈ 148 bits; require a conservative floor.
    if (bits < 128) {
        throw new AppError({
            code: 'invalid_argument',
            message: 'WebUI password entropy is below the 128-bit install floor',
        });
    }
}

function lockDownFile(file: string): void {
    if (process.platform === 'win32') return;
    try {
        fs.chmodSync(file, 0o600);
    }
    catch {
        // Best-effort on hosts that reject chmod.
    }
}

/** Write credentials atomically under the config directory. */
export function saveCredentials(credentials: WebuiCredentials): string {
    assertPasswordStrength(credentials.password);
    if (credentials.username !== WEBUI_USERNAME) {
        throw new AppError({
            code: 'invalid_argument',
            message: `WebUI username must be \`${WEBUI_USERNAME}\``,
        });
    }
    if (!Number.isInteger(credentials.port) || credentials.port < 1 || credentials.port > 65535) {
        throw new AppError({
            code: 'invalid_argument',
            message: 'WebUI port must be an integer between 1 and 65535',
        });
    }
    ensureConfigDir();
    const file = credentialsPath();
    const tmp = `${file}.${process.pid}.tmp`;
    const body = `${JSON.stringify(credentials, null, 2)}\n`;
    fs.writeFileSync(tmp, body, { encoding: 'utf8', mode: 0o600 });
    fs.renameSync(tmp, file);
    lockDownFile(file);
    return file;
}

/**
 * Load credentials written by install.
 *
 * Throws when the file is missing or corrupt — `server` must not invent secrets.
 */
export function loadCredentials(): WebuiCredentials {
    const file = credentialsPath();
    if (!fs.existsSync(file)) {
        throw new AppError({
            code: 'not_installed',
            capability: 'server',
            path: file,
            message:
                'WebUI credentials are missing. Run `open-panel install` or ' +
                '`npx @wae/open-panel install` before starting the server.',
        });
    }
    let raw: unknown;
    try {
        raw = JSON.parse(fs.readFileSync(file, 'utf8')) as unknown;
    }
    catch {
        throw new AppError({
            code: 'invalid_argument',
            capability: 'server',
            path: file,
            message: `cannot parse WebUI credentials at \`${file}\``,
        });
    }
    if (!raw || typeof raw !== 'object' || Array.isArray(raw)) {
        throw new AppError({
            code: 'invalid_argument',
            capability: 'server',
            path: file,
            message: `invalid WebUI credentials shape at \`${file}\``,
        });
    }
    const record = raw as Record<string, unknown>;
    const username = typeof record.username === 'string' ? record.username : '';
    const password = typeof record.password === 'string' ? record.password : '';
    const port = typeof record.port === 'number' ? record.port : Number(record.port);
    const createdAt = typeof record.createdAt === 'string' ? record.createdAt : '';
    if (username !== WEBUI_USERNAME || !password || !Number.isInteger(port) || !createdAt) {
        throw new AppError({
            code: 'invalid_argument',
            capability: 'server',
            path: file,
            message: `invalid WebUI credentials at \`${file}\` — re-run \`open-panel install --force\``,
        });
    }
    assertPasswordStrength(password);
    return { username, password, port, createdAt };
}

/** True when a credentials file already exists on disk. */
export function credentialsExist(): boolean {
    return fs.existsSync(credentialsPath());
}

export interface InstallCredentialsOptions {
    /** Replace an existing credentials file. */
    force?: boolean;
    /** Override listen port (default {@link DEFAULT_WEBUI_PORT}). */
    port?: number;
}

export interface InstallCredentialsResult {
    credentials: WebuiCredentials;
    path: string;
    created: boolean;
}

/**
 * Create (or reuse) install-time WebUI credentials.
 *
 * Generation happens here — never inside the `server` listen path.
 */
export function installCredentials(options: InstallCredentialsOptions = {}): InstallCredentialsResult {
    const port = options.port ?? DEFAULT_WEBUI_PORT;
    if (!options.force && credentialsExist()) {
        return {
            credentials: loadCredentials(),
            path: credentialsPath(),
            created: false,
        };
    }
    const credentials: WebuiCredentials = {
        username: WEBUI_USERNAME,
        password: generateStrongPassword(),
        port,
        createdAt: new Date().toISOString(),
    };
    const file = saveCredentials(credentials);
    return { credentials, path: file, created: true };
}
