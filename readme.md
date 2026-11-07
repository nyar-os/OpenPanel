# OpenPanel

本机运维面板（类 BT 面板 / 1Panel）：管理本机服务、站点、进程与日志。基于 WAE 元工具包，**不是** API 中转站或网关。

同一安装物 `@wae/open-panel`（CLI 名 `open-panel`）提供这些入口：

| 命令 | 角色 | 界面包 |
|------|------|--------|
| `open-panel install` | 写入本机 WebUI 凭据（用户 `administrator`、高强度随机密码、默认端口 `8888`） | — |
| `open-panel client` | 多 server 控制面（桌面） | `@wae/open-panel-gui`（WAE WebView） |
| `open-panel server` | 单机运维端点（WebUI，只读取 install 已写入的凭据） | `@wae/open-panel-webui`（HTTP 静态托管） |

这些是同一 CLI 的子命令分流，不要拆成两套安装包或两个主程序。两套前端彼此独立，不要再合并成单一 `*-ui` 包。一份 `client` 可同时管理多个本机或远端 `server`，不是 1:1 绑定单机。

**安装与凭据**（`install` / `npx @wae/open-panel install` / 包 `postinstall` 走同一实现）：

```bash
# 推荐：显式安装（npx 与全局装后同命令）
npx @wae/open-panel install
# 或
pnpm --filter @wae/open-panel exec node ./dist/cli.js install
```

首次 install 会在配置目录写入 `webui-credentials.json`（Windows `%APPDATA%/open-panel/`，其它平台 `~/.config/open-panel/`，可用 `OPEN_PANEL_CONFIG_DIR` 覆盖），控制台打印一次密码与登录地址 `http://<host>:8888/login`。`open-panel server` **不会**现场生成密码；缺少凭据时会报错并提示先运行 install。

客户端用本地 JSON 名册管理多台 `server`（不含密码等秘密）：

```bash
pnpm --filter @wae/open-panel exec node ./dist/cli.js servers add --id local --url http://127.0.0.1:8888/ --label "Local"
pnpm --filter @wae/open-panel exec node ./dist/cli.js servers list
pnpm --filter @wae/open-panel exec node ./dist/cli.js servers use --id local
pnpm --filter @wae/open-panel exec node ./dist/cli.js client --server-id local
```

名册默认路径：Windows `%APPDATA%/open-panel/servers.json`，其它平台 `~/.config/open-panel/servers.json`（可用 `OPEN_PANEL_CONFIG_DIR` 覆盖）。

Windows 远端安装主通道是 **RDP(3389)**，由 Rust crate `open-panel-rdp`（IronRDP）实现协议栈；**禁止**依赖 `mstsc` / FreeRDP / `wfreerdp`，也**不**要求远端先装 OpenSSH。

已打通：TCP→X.224/MCS/GCC→TLS→CredSSP 握手，以及 force-fresh logon 后的 Win+R 注入（lab 上 AlternateShell 单独无副作用）。安装产物投递策略：

1. **默认**：控制器起临时 HTTP，远端 Win+R 注入短 `cmd /c curl.exe -fsSL -o C:/Users/Public/op-i.ps1 …`（与已证 ntfy 路径同形，≤512 字节）拉取 `i.ps1` + tarball。远端必须能访问该 origin（局域网不可达时设 `--pack-base-url` / `OPEN_PANEL_PACK_BASE_URL` 为公网 URL）。可选 `OPEN_PANEL_INSTALL_NTFY` 收进度。
2. **GitHub raw（公网兜底）**：`scripts/remote/op.cmd` + `install.ps1` + `wae-open-panel-0.0.0.tgz` 随 `dev` 分支发布；Win+R：`cmd /c curl -fsSL https://raw.githubusercontent.com/nyar-os/OpenPanel/dev/scripts/remote/op.cmd -o C:/Users/Public/op.cmd&call C:/Users/Public/op.cmd`（进度 topic `op-gh-raw-1107`，poll `https://ntfy.sh/<topic>/json`）。
3. **已知远端路径**：`--remote-tarball` 指向主机上已有的 `.tgz`。
4. **已有引导 URL**：`--package-url` 指向远端可拉的脚本。
5. **RDPDR**：仅 `OPEN_PANEL_USE_RDPDR=1` 且显式关闭 HTTP 时启用；lab 上裸 `\\tsclient\OP\a.cmd` 从未执行成功。公网优先 npm；包仍为 `0.0.0` 时本地 `npm pack`。

```bash
cargo build -p open-panel-rdp
# password is NEVER passed as a CLI flag / never committed
$env:OPEN_PANEL_REMOTE_PASSWORD = '...'   # local shell only
cargo run -p open-panel-rdp -- handshake --host 203.0.113.10 --user Administrator --password-env OPEN_PANEL_REMOTE_PASSWORD
cargo run -p open-panel-rdp -- exec --host 203.0.113.10 --user Administrator --password-env OPEN_PANEL_REMOTE_PASSWORD --shell "powershell.exe -NoP -c \"[Net.Sockets.TcpListener]17999|%{$_.Start();$_.AcceptTcpClient()|Out-Null;$_.Stop()}\""
pnpm --filter @wae/open-panel exec node ./dist/cli.js remote probe --host 203.0.113.10
pnpm --filter @wae/open-panel exec node ./dist/cli.js remote install --host 203.0.113.10 --user Administrator --password-env OPEN_PANEL_REMOTE_PASSWORD
```

WebUI 登录入口：`http://<host>:8888/login`（有 TLS 时为 `https://…/login`）。未登录访问其它页面会导向 `/login`。

`open-panel server` HTTP API（`/api/v0/...`，兼 `/api/...`，需登录会话或 HTTP Basic）：

- `POST /login` / `POST /logout` — 会话登录与登出
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
pnpm --filter @wae/open-panel exec node ./dist/cli.js install
pnpm --filter @wae/open-panel exec node ./dist/cli.js server --host 127.0.0.1
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
