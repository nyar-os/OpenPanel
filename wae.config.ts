/**
 * Build and release configuration for OpenPanel.
 *
 * Pure literals only: `wae` parses this file statically and never executes it.
 * Build order is fixed: native → ts → ui → skills → staging. The GUI package is
 * declared for `wae build`; the companion web UI is built by its own root script.
 */
export default {
    native: {
        crate: 'open-panel-napi',
        cdylib: 'open_panel_napi',
        targets: [
            {
                platform: 'win32',
                arch: 'x64',
                triple: 'x86_64-pc-windows-msvc',
                package: '@wae/open-panel-win32-x64',
                nodeFile: 'open-panel.win32-x64.node',
                dir: 'packages/open-panel-win32-x64',
            },
        ],
    },
    packages: [
        {
            name: '@wae/open-panel-win32-x64',
            dir: 'packages/open-panel-win32-x64',
            os: ['win32'],
            cpu: ['x64'],
            main: 'open-panel.win32-x64.node',
        },
    ],
    build: {
        ts: { packages: ['@wae/open-panel'] },
        ui: { package: '@wae/open-panel-gui' },
        stage: [
            { from: 'packages/open-panel-gui/dist/static', to: 'packages/open-panel/dist/gui' },
        ],
        outputs: ['packages/open-panel/dist/cli.js', 'packages/open-panel/dist/gui/index.html'],
    },
    skills: {
        packages: [
            {
                dir: 'packages/open-panel-skills/skills',
                packageJson: 'packages/open-panel-skills/package.json',
                appPackageJson: 'packages/open-panel/package.json',
                cliEntry: 'packages/open-panel/dist/cli.js',
                commandName: 'open-panel',
            },
        ],
    },
    release: {
        registry: 'https://registry.npmjs.org',
        tagPattern: '^v(\\d+\\.\\d+\\.\\d+)$',
        packages: [
            { name: '@wae/open-panel', dir: 'packages/open-panel' },
            { name: '@wae/open-panel-skills', dir: 'packages/open-panel-skills' },
            { name: '@wae/open-panel-win32-x64', dir: 'packages/open-panel-win32-x64' },
        ],
    },
    ci: {
        repo: 'nyar-os/OpenPanel',
        workflows: ['check-rust.yml', 'check-typescript.yml', 'publish-npm.yml'],
        requiredJobs: ['check-rust', 'check-typescript'],
    },
    install: {
        shortcutDefault: true,
    },
    audit: {
        actor: { kind: 'flag', id: 'flag', source: 'flag' },
    },
};
