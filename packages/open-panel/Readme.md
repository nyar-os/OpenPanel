# `@wae/open-panel`

Local ops panel CLI. One installable owns both entries:

- `open-panel gui` — desktop client (WAE WebView)
- `open-panel serve` — local WebUI for browsers

```bash
pnpm --filter @wae/open-panel exec node ./dist/cli.js capabilities --json
pnpm --filter @wae/open-panel exec node ./dist/cli.js gui
pnpm --filter @wae/open-panel exec node ./dist/cli.js serve --port 17890
```
