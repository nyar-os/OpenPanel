/**
 * Unit tests for install-time password strength and WebUI default port.
 *
 * Run: `node --experimental-strip-types --test src/credentials.test.ts`
 */
import assert from 'node:assert/strict';
import fs from 'node:fs';
import os from 'node:os';
import path from 'node:path';
import test from 'node:test';

import {
    assertPasswordStrength,
    generateStrongPassword,
    installCredentials,
    loadCredentials,
    passwordEntropyBits,
    saveCredentials,
} from './credentials.js';
import {
    DEFAULT_WEBUI_PORT,
    WEBUI_PASSWORD_ALPHABET,
    WEBUI_PASSWORD_LENGTH,
    WEBUI_PASSWORD_MIN_LENGTH,
    WEBUI_USERNAME,
    webuiLoginUrl,
} from './defaults.js';
import { resolveListenPort } from './server.js';

test('default WebUI port is 8888', () => {
    assert.equal(DEFAULT_WEBUI_PORT, 8888);
    assert.equal(resolveListenPort(undefined, DEFAULT_WEBUI_PORT), 8888);
    assert.equal(resolveListenPort(9001, DEFAULT_WEBUI_PORT), 9001);
});

test('login URL uses /login on the default port', () => {
    assert.equal(webuiLoginUrl('203.0.113.10'), 'http://203.0.113.10:8888/login');
    assert.equal(webuiLoginUrl('203.0.113.10', 8443, true), 'https://203.0.113.10:8443/login');
});

test('generated password meets length and entropy floors', () => {
    const password = generateStrongPassword();
    assert.equal(password.length, WEBUI_PASSWORD_LENGTH);
    assert.ok(password.length >= WEBUI_PASSWORD_MIN_LENGTH);
    const bits = passwordEntropyBits(password, WEBUI_PASSWORD_ALPHABET);
    assert.ok(bits >= 128, `expected >= 128 bits, got ${bits}`);
    assert.doesNotThrow(() => assertPasswordStrength(password));
});

test('weak passwords are rejected', () => {
    assert.throws(() => assertPasswordStrength('admin'), /at least/);
    assert.throws(() => assertPasswordStrength('password'), /at least/);
    assert.throws(() => generateStrongPassword({ length: 8 }), />=/);
});

test('install writes credentials that server can load', () => {
    const dir = fs.mkdtempSync(path.join(os.tmpdir(), 'open-panel-cred-'));
    const previous = process.env.OPEN_PANEL_CONFIG_DIR;
    process.env.OPEN_PANEL_CONFIG_DIR = dir;
    try {
        const first = installCredentials();
        assert.equal(first.created, true);
        assert.equal(first.credentials.username, WEBUI_USERNAME);
        assert.equal(first.credentials.port, DEFAULT_WEBUI_PORT);
        const loaded = loadCredentials();
        assert.equal(loaded.password, first.credentials.password);
        const second = installCredentials();
        assert.equal(second.created, false);
        assert.equal(second.credentials.password, first.credentials.password);
        const rotated = installCredentials({ force: true });
        assert.equal(rotated.created, true);
        assert.notEqual(rotated.credentials.password, first.credentials.password);
        saveCredentials({
            username: WEBUI_USERNAME,
            password: generateStrongPassword(),
            port: 9000,
            createdAt: new Date().toISOString(),
        });
        assert.equal(loadCredentials().port, 9000);
    }
    finally {
        if (previous === undefined) delete process.env.OPEN_PANEL_CONFIG_DIR;
        else process.env.OPEN_PANEL_CONFIG_DIR = previous;
        fs.rmSync(dir, { recursive: true, force: true });
    }
});
