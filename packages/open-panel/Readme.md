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

## Remote install

```bash
open-panel remote probe --host 203.0.113.10
open-panel remote install --host 203.0.113.10 --user Administrator --password-env OPEN_PANEL_REMOTE_PASSWORD --pack-local
open-panel remote bootstrap-script
```

Passwords only via `--password-env`. While `@wae/open-panel` remains unpublished / `0.0.0`, use `--pack-local`. Channels: `auto` (default), `rdp` (FreeRDP), `ssh`, `winrm`, `smb`. When FreeRDP is unavailable, install falls back to a bootstrap script for an interactive RDP session — it never pretends SSH worked.

## Server API

Under `/api/v0` (also `/api`):

- `GET /host` — hostname / OS / Node identity
- `GET /services`, `GET /services/:name`, `POST /services/:name/{start,stop,restart}` — Windows SCM native services
- `GET|POST /sites`, `DELETE /sites/:id`, `POST /sites/:id/{enable,disable,start,stop}` — web sites

Web sites currently use OpenPanel-hosted `static-host` listeners (document root + bind host/port). IIS/Nginx production config generation and Node/Python app runtimes are out of scope for now.
