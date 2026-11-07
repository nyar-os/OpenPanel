# `@wae/open-panel`

Local ops panel CLI with `install`, `client`, `server`, `servers`, and `remote`.

## Local install (credentials)

`open-panel install`, `npx @wae/open-panel install`, and the package `postinstall` hook share one implementation. They write `administrator` + a strong random password + default port `8888` under the per-user config dir (`webui-credentials.json`). The password is printed once. `open-panel server` only reads that file and serves `http://<host>:8888/login`.

```bash
npx @wae/open-panel install
open-panel server
```

Set `OPEN_PANEL_SKIP_POSTINSTALL=1` to skip the automatic lifecycle install.

## Remote

Windows remote install uses Rust `open-panel-rdp` (IronRDP). It does not wrap `mstsc` / FreeRDP / `wfreerdp`, and does not require OpenSSH on the target.

```bash
cargo build -p open-panel-rdp
pnpm --filter @wae/open-panel exec node ./dist/cli.js remote probe --host 203.0.113.10 --json
$env:OPEN_PANEL_REMOTE_PASSWORD = '...'
pnpm --filter @wae/open-panel exec node ./dist/cli.js remote install --host 203.0.113.10 --user Administrator --password-env OPEN_PANEL_REMOTE_PASSWORD --json
```
