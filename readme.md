# OpenPanel

本机运维面板（类 BT 面板 / 1Panel）：管理本机服务、站点、进程与日志。基于 WAE 元工具包，**不是** API 中转站或网关。

同一安装物 `@wae/open-panel`（CLI 名 `open-panel`）提供两个入口：

| 命令 | 角色 | 界面包 |
|------|------|--------|
| `open-panel client` | 多 server 控制面（桌面） | `@wae/open-panel-gui`（WAE WebView） |
| `open-panel server` | 单机运维端点（WebUI） | `@wae/open-panel-webui`（HTTP 静态托管） |

二者是同一 CLI 的子命令分流，不要拆成两套安装包或两个主程序。两套前端彼此独立，不要再合并成单一 `*-ui` 包。一份 `client` 可同时管理多个本机或远端 `server`，不是 1:1 绑定单机。

客户端用本地 JSON 名册管理多台 `server`（不含密码等秘密）：

```bash
pnpm --filter @wae/open-panel exec node ./dist/cli.js servers add --id local --url http://127.0.0.1:17890/ --label "Local"
pnpm --filter @wae/open-panel exec node ./dist/cli.js servers list
pnpm --filter @wae/open-panel exec node ./dist/cli.js servers use --id local
pnpm --filter @wae/open-panel exec node ./dist/cli.js client --server-id local
```

名册默认路径：Windows `%APPDATA%/open-panel/servers.json`，其它平台 `~/.config/open-panel/servers.json`（可用 `OPEN_PANEL_CONFIG_DIR` 覆盖）。

Windows 远端安装主通道是 **RDP(3389)**，由 Rust crate `open-panel-rdp`（IronRDP）实现协议栈；**禁止**依赖 `mstsc` / FreeRDP / `wfreerdp`，也**不**要求远端先装 OpenSSH。当前已完成 TCP→X.224/MCS/GCC→TLS→CredSSP 握手；tarball 投递与 `alternate_shell` 执行安装是下一层。公网优先 npm；包仍为 `0.0.0` 时本地 `npm pack`。

```bash
cargo build -p open-panel-rdp
# password is NEVER passed as a CLI flag / never committed
$env:OPEN_PANEL_REMOTE_PASSWORD = '...'   # local shell only
cargo run -p open-panel-rdp -- handshake --host 203.0.113.10 --user Administrator --password-env OPEN_PANEL_REMOTE_PASSWORD
pnpm --filter @wae/open-panel exec node ./dist/cli.js remote probe --host 203.0.113.10
pnpm --filter @wae/open-panel exec node ./dist/cli.js remote install --host 203.0.113.10 --user Administrator --password-env OPEN_PANEL_REMOTE_PASSWORD
```

`open-panel server` HTTP API（`/api/v0/...`，兼 `/api/...`）：

- `GET /host` — 主机名/OS 只读状态
- `GET|POST /services...` — 本机 Native 服务（Windows Service Control Manager：列表、状态、启停、重启）
- `GET|POST|DELETE /sites...` — Web 站点登记与启停（当前为 OpenPanel 托管的 `static-host` 静态监听，不是 IIS/Nginx 配置生成；Node/Python 应用运行时托管尚未做）

## 布局

| 路径 | 职责 |
|------|------|
| `crates/core` | 运维领域能力与三态失败合同 |
| `crates/rdp` | Windows RDP 协议客户端（IronRDP，远程安装主通道） |
| `crates/napi` | 平台中立 N-API 面（含桌面 GUI 宿主） |
| `packages/open-panel` | 元包、类型库、CLI（`client` / `server` / `remote`） |
| `packages/open-panel-gui` | 桌面客户端 UI（仅 `client`） |
| `packages/open-panel-webui` | 服务端 WebUI（仅 `server`） |
| `packages/open-panel-skills` | Agent 技能 |
| `packages/open-panel-win32-x64` | Windows x64 原生平台包 |

## 开发

```bash
pnpm install
pnpm run build:napi
pnpm run build:ts
pnpm run build:gui
pnpm run build:webui
pnpm --filter @wae/open-panel exec node ./dist/cli.js capabilities --json
pnpm --filter @wae/open-panel exec node ./dist/cli.js server --host 127.0.0.1 --port 17890
pnpm --filter @wae/open-panel exec node ./dist/cli.js client
```

平台支持：首发 `win32-x64`。其它平台在未发布对应平台包前视为不支持。

## npm 占位（0.0.0）与真实发布

发布集合（非 `private`）：

| 包 | 说明 |
|----|------|
| `@wae/open-panel-win32-x64` | Windows x64 原生平台包（先发） |
| `@wae/open-panel-skills` | Agent skills |
| `@wae/open-panel` | 元包 / CLI |

`@wae/open-panel-gui` 与 `@wae/open-panel-webui` 为 `private`，不进 registry。

**`0.0.0` 占位**是硬规则允许的唯一本地 npm 发布例外（领取包名 + 配置 Trusted Publisher）。真实 semver 只能经 GitHub Actions OIDC（`.github/workflows/publish-npm.yml`，`environment: NPM_PUBLISH`），禁止本地 `npm publish` / `pnpm publish` / token 旁路。

```bash
# 1) 构建可打包产物（平台包需要 .node；元包需要 dist）
pnpm install
pnpm run build

# 2) 干跑，确认顺序与跳过的 private 包
pnpm placeholder

# 3) 本地发 0.0.0 占位（需 npm 登录或 .env.placeholder.local）
pnpm placeholder:publish

# 4) 配置 Trusted Publisher（指向本仓 publish-npm.yml / NPM_PUBLISH）
#    先在 GitHub 建好 environment: NPM_PUBLISH
pnpm placeholder:trust
```

真实版本：在 `dev` 上 bump 后打 tag `vX.Y.Z` 并 push tag（或经授权的 `workflow_dispatch`），由 OIDC workflow 发布。细节见 `scripts/README.md`。

## License

Apache-2.0. See `License.md`.
