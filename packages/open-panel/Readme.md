# `@wae/open-panel`

Local ops panel CLI with `client`, `server`, `servers`, and `remote`.

Windows remote install uses Rust `open-panel-rdp` (IronRDP). It does not wrap `mstsc` / FreeRDP / `wfreerdp`, and does not require OpenSSH on the target.

```bash
cargo build -p open-panel-rdp
pnpm --filter @wae/open-panel exec node ./dist/cli.js remote probe --host 203.0.113.10 --json
$env:OPEN_PANEL_REMOTE_PASSWORD = '...'
pnpm --filter @wae/open-panel exec node ./dist/cli.js remote install --host 203.0.113.10 --user Administrator --password-env OPEN_PANEL_REMOTE_PASSWORD --json
```
