/**
 * Stage the desktop GUI shell into `dist/static`.
 *
 * The layout matches Open* UI packages so a later VMZ build can replace this
 * step without renaming packages or staging paths.
 */
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const here = path.dirname(fileURLToPath(import.meta.url));
const uiRoot = path.resolve(here, '..');
const from = path.join(uiRoot, 'static');
const to = path.join(uiRoot, 'dist', 'static');

function copyDir(src: string, dest: string): void {
    fs.mkdirSync(dest, { recursive: true });
    for (const entry of fs.readdirSync(src, { withFileTypes: true })) {
        const s = path.join(src, entry.name);
        const d = path.join(dest, entry.name);
        if (entry.isDirectory()) copyDir(s, d);
        else fs.copyFileSync(s, d);
    }
}

if (!fs.existsSync(path.join(from, 'index.html'))) {
    console.error(`missing static shell at ${from}`);
    process.exit(1);
}

fs.rmSync(to, { recursive: true, force: true });
copyDir(from, to);
console.log(`built desktop GUI -> ${to}`);
