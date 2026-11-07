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

规划中：远端安装/升级同一套 `@wae/open-panel`（公网优先 npm；包仍为 `0.0.0` 未发布时用本地 `npm pack` / tarball 验收）。

## 布局

| 路径 | 职责 |
|------|------|
| `crates/core` | 运维领域能力与三态失败合同 |
| `crates/napi` | 平台中立 N-API 面（含桌面 GUI 宿主） |
| `packages/open-panel` | 元包、类型库、CLI（`client` / `server`） |
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

## License

Apache-2.0. See `License.md`.
