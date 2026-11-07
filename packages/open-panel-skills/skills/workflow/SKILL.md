---
name: workflow
description: OpenPanel local ops panel workflow for desktop GUI and WebUI serve entries.
---

# OpenPanel workflow

Install one package: `@wae/open-panel`. Client and server are CLI subcommands of
the same binary.

## Safety

- Confirm the target host before starting `serve` on a non-loopback address.
- Prefer `127.0.0.1` unless remote browser access is explicitly required.
- Do not treat this tool as an API gateway or request proxy.

## Commands

Useful invocations:

- `open-panel capabilities --json`
- `open-panel version --json`
- `open-panel gui --title <text>`
- `open-panel serve --host <host> --port <n>`
- `open-panel audit --limit <n> --json`
