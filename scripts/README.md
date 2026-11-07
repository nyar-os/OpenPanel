# scripts/

Root automation for build, native staging, skills checks, and npm publish support.

## Layout

```text
scripts/
  build-napi.ts      Build the Windows x64 N-API addon
  stage-native.ts    Copy `.node` into packages/open-panel-win32-x64
  check-skills.ts    Validate agent skill front matter
```

## npm placeholder (0.0.0)

Reserve package names before Trusted Publisher real releases (`@doki-land/nifty`).
Only non-`private` workspace packages are published:

- `@wae/open-panel-win32-x64`
- `@wae/open-panel-skills`
- `@wae/open-panel`

`@wae/open-panel-gui` and `@wae/open-panel-webui` stay `private` and are skipped.

```bash
pnpm run build                 # native + TS + UI staged into publish roots
pnpm placeholder               # nifty publish --placeholder --dry-run
pnpm placeholder:publish       # publish missing packages @0.0.0 (name claim)
pnpm placeholder:trust         # configure Trusted Publisher
```

Local secrets (gitignored): `.env.placeholder.local` with `NPM_TOKEN` / `NPM_TOTP_SECRET` as required by nifty.

`0.0.0` placeholder stubs are the only local npm publish exception. Real semver versions publish only through GitHub Actions Trusted Publisher (OIDC) via `.github/workflows/publish-npm.yml` (`environment: NPM_PUBLISH`), never with a local token.
