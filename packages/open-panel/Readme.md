# `@wae/open-panel`

Local ops panel CLI. One installable owns both entries:

- `open-panel client` — multi-server control plane (WAE WebView + `@wae/open-panel-gui`)
- `open-panel server` — single-machine WebUI endpoint (`@wae/open-panel-webui`)

```bash
pnpm --filter @wae/open-panel exec node ./dist/cli.js capabilities --json
pnpm --filter @wae/open-panel exec node ./dist/cli.js client
pnpm --filter @wae/open-panel exec node ./dist/cli.js server --port 17890
```
