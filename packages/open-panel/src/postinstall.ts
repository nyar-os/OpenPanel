/**
 * npm / pnpm / npx lifecycle entry.
 *
 * Invoked as `node ./dist/postinstall.js` from `package.json#scripts.postinstall`.
 * Delegates to the same `runInstall` used by `open-panel install` and
 * `npx @wae/open-panel install`.
 */
import { formatInstallReport, runInstall } from './install.js';

function shouldSkip(): boolean {
    if (process.env.OPEN_PANEL_SKIP_POSTINSTALL === '1') return true;
    if (process.env.OPEN_PANEL_SKIP_INSTALL === '1') return true;
    return false;
}

async function main(): Promise<number> {
    if (shouldSkip()) {
        process.stdout.write('open-panel postinstall: skipped (`OPEN_PANEL_SKIP_POSTINSTALL`)\n');
        return 0;
    }
    try {
        const report = runInstall({ fromPostinstall: true });
        if (report.created) {
            process.stdout.write(`${formatInstallReport(report)}\n`);
        }
        else {
            process.stdout.write(
                `open-panel postinstall: credentials already at ${report.path}\n`,
            );
        }
        return 0;
    }
    catch (error) {
        const message = error instanceof Error ? error.message : String(error);
        // Lifecycle must not brick `npm install` in incomplete workspaces.
        // Operators can always run `npx @wae/open-panel install` explicitly.
        process.stderr.write(`open-panel postinstall: ${message}\n`);
        process.stderr.write('open-panel postinstall: run `npx @wae/open-panel install` after install.\n');
        return 0;
    }
}

const code = await main();
process.exitCode = code;
