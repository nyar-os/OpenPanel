import { defineConfig } from '@vmz/vmz';

/**
 * `open-panel server` hosts local static HTML under packages/open-panel/dist/webui.
 * Builtin profile `static` → assembly `web-static`.
 */
export default defineConfig({
    delivery: {
        default: 'static',
        profiles: {
            static: {
                host: 'browser',
                assembly: 'web-static',
            },
        },
    },
});
