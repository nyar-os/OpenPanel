/**
 * npm lifecycle shim. Published tarballs include `dist/postinstall.js`.
 * Workspace checkouts may not have built yet — skip instead of failing install.
 */
import { existsSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const here = dirname(fileURLToPath(import.meta.url));
const entry = join(here, 'dist', 'postinstall.js');

if (!existsSync(entry)) {
    process.stdout.write(
        'open-panel postinstall: `dist/postinstall.js` missing — skip. ' +
            'After build or publish, run `npx @wae/open-panel install`.\n',
    );
    process.exit(0);
}

await import(pathToFileURL(entry).href);
