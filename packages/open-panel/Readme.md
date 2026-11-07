# `@wae/open-panel`

Local ops panel CLI. One installable owns both entries:

- `open-panel client` — multi-server control plane (WAE WebView + `@wae/open-panel-gui`)
- `open-panel server` — single-machine WebUI endpoint (`@wae/open-panel-webui`)
- `open-panel servers` — local roster CRUD (`servers.json` under the user config dir)

Secrets never belong in the roster file or the git tree.

```bash
pnpm --filter @wae/open-panel exec node ./dist/cli.js capabilities --json
pnpm --filter @wae/open-panel exec node ./dist/cli.js servers add --id local --url http://127.0.0.1:17890/
pnpm --filter @wae/open-panel exec node ./dist/cli.js servers list
pnpm --filter @wae/open-panel exec node ./dist/cli.js servers use --id local
pnpm --filter @wae/open-panel exec node ./dist/cli.js client --server-id local
pnpm --filter @wae/open-panel exec node ./dist/cli.js server --port 17890
```

Override config directory with `OPEN_PANEL_CONFIG_DIR`.
