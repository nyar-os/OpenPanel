/**
 * Build and release configuration for OpenPanel.
 *
 * This file is pure data so it can be parsed statically. `wae build` reads it
 * to order native → TypeScript → UI → skills validation → staging.
 */
export default {
    native: {
        crate: 'open-panel-napi',
        target: 'win32-x64',
        cdylib: 'open_panel_napi',
        artifact: 'open-panel.win32-x64.node',
        platformPackage: '@wae/open-panel-win32-x64',
    },
    packages: {
        native: {
            dir: 'packages/open-panel-win32-x64',
            os: ['win32'],
            cpu: ['x64'],
        },
    },
    build: {
        order: ['native', 'typescript', 'ui', 'skills'],
        ts: {
            packages: ['@wae/open-panel'],
        },
        ui: {
            packages: ['@wae/open-panel-gui', '@wae/open-panel-webui'],
        },
    },
    skills: {
        dir: 'packages/open-panel-skills/skills',
        packages: ['@wae/open-panel-skills'],
        check: 'scripts/check-skills.ts',
    },
    release: {
        packages: ['@wae/open-panel', '@wae/open-panel-skills', '@wae/open-panel-win32-x64'],
        tag: 'vX.Y.Z',
    },
    ci: {
        repo: 'nyar-os/OpenPanel',
        workflows: ['check-rust.yml', 'check-typescript.yml'],
    },
    install: {
        shortcut: true,
        perUser: true,
    },
    audit: {
        actorSources: {
            cli: 'cli',
            client: 'client',
            server: 'server',
            skill: 'skill',
        },
    },
};
