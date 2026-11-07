# OpenPanel

本机运维面板（类 BT 面板 / 1Panel）：管理本机服务、站点、进程与日志。基于 WAE 元工具包，**不是** API 中转站或网关。

同一安装物 `@wae/open-panel`（CLI 名 `open-panel`）提供两个入口：

| 命令 | 角色 | 界面 |
|------|------|------|
| `open-panel gui` | 客户端 | 桌面宿主（WAE WebView） |
| `open-panel serve` | 服务端 | 本机 / 远端浏览器访问的 WebUI |

二者是同一 CLI 的子命令分流，不要拆成两套安装包或两个主程序。

## 布局

| 路径 | 职责 |
|------|------|
| `crates/core` | 运维领域能力与三态失败合同 |
| `crates/napi` | 平台中立 N-API 面（含桌面 GUI 宿主） |
| `packages/open-panel` | 元包、类型库、CLI（`gui` / `serve`） |
| `packages/open-panel-ui` | 面板 UI（`gui` 与 `serve` 共用 staged 产物） |
| `packages/open-panel-skills` | Agent 技能 |
| `packages/open-panel-win32-x64` | Windows x64 原生平台包 |

## 开发

```bash
pnpm install
pnpm run build:napi
pnpm run build:ts
pnpm run build:ui
pnpm --filter @wae/open-panel exec node ./dist/cli.js capabilities --json
pnpm --filter @wae/open-panel exec node ./dist/cli.js serve --host 127.0.0.1 --port 17890
pnpm --filter @wae/open-panel exec node ./dist/cli.js gui
```

平台支持：首发 `win32-x64`。其它平台在未发布对应平台包前视为不支持。

## License

Apache-2.0. See `License.md`.
