---
name: workflow
description: OpenPanel local ops panel workflow for desktop client and WebUI server entries.
---

# OpenPanel workflow

Install one package: `@wae/open-panel`. Client and server are CLI subcommands of
the same binary. Frontend packages stay separate:
`@wae/open-panel-gui` for `client`, `@wae/open-panel-webui` for `server`.
Treat `client` as a multi-server control plane (many endpoints), not a 1:1
binding to a single host.

## Safety

- Confirm the target host before starting `server` on a non-loopback address.
- Prefer `127.0.0.1` unless remote browser access is explicitly required.
- Do not treat this tool as an API gateway or request proxy.

## Commands

Useful invocations:

- `open-panel install` (or `npx @wae/open-panel install`) — write WebUI credentials before server
- `open-panel capabilities --json`
- `open-panel version --json`
- `open-panel client --title <text>`
- `open-panel server --host <host>` — default port `8888`, login at `/login`
- `open-panel audit --limit <n> --json`

WebUI user is always `administrator`. The strong password is created only by `install` / postinstall, never by `server`.
