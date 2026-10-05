# t — 命令输出对照翻译

```
t git -h
t --lang ja docker logs -f web
t codex --help
```

## 一键安装

Windows（PowerShell）：

```powershell
irm https://raw.githubusercontent.com/GitDjAr/t-translate/main/install.ps1 | iex
```

Linux / macOS（Apple Silicon）：

```sh
curl -fsSL https://raw.githubusercontent.com/GitDjAr/t-translate/main/install.sh | sh
```

安装到 `~/.t-translate/bin`（Linux/macOS 为 `~/.local/bin`）并自动加入 PATH。
GitHub 访问慢时，可设置镜像前缀：`$env:T_MIRROR="https://ghfast.top/"`（sh 里 `T_MIRROR=...`）。
之后升级：`t --update`。

## 命令

| 命令 | 说明 |
|---|---|
| `t <命令...>` | 运行命令并输出对照翻译 |
| `t --update` | 更新到最新 Release |
| `t --alias [名字]` | 创建别名（会检查 PATH 冲突），例如 `tt` |
| `t --cache [clear]` | 查看 / 清空缓存 |
| `t --version` | 版本 |

## 配置（环境变量）

| 变量 | 说明 | 默认 |
|---|---|---|
| T_LANG | 目标语言 | zh-CN |
| T_BACKEND | `auto`（先探测 Google，3 秒内通就用，否则用 Edge/Bing）/ `google` / `edge`（或 `bing`）/ `openai`（兼容接口，DeepSeek、Ollama 也行） | auto |
| T_API_BASE | openai 后端地址 | https://api.openai.com/v1 |
| T_API_KEY / T_MODEL | openai 后端的 Key / 模型 | - / gpt-4o-mini |

缓存：`~/.t-translate/cache.json`，有效期 30 天，`--no-cache` 关闭。

## 行为

- 普通输出：整行批量翻译，原文下方亮青色译文；帮助文档的两栏格式会把译文对齐到说明列。
- 流式输出（`tail -f`、`docker logs -f`）：每批最多等待 500ms 就翻译，持续不断的输出也不会卡住。
- 交互提示（以 `:` `?` `)` `]` `>` 结尾、空闲 200ms）：在提示后追加 `(译文)`，输入照常转发。
- 未结束且不像提示的半行（如逐字输出）：800ms 后原样输出，不会卡住。
- 进度条（`\r` 覆盖）和全屏 TUI（alternate screen）：原样透传。
- 跳过：已含中文、过短、单个词/路径/哈希等。

## 代码结构

```
src/
  main.rs        入口分发
  cli.rs         参数解析
  runner.rs      PTY + 流式处理（Pump）
  render.rs      输出 / 对齐 / 高亮
  text.rs        ANSI 剥离、翻译过滤启发式（含单元测试）
  update.rs      t --update
  alias.rs       t --alias
  translator/
    mod.rs       后端选择（auto）、批量翻译
    cache.rs     磁盘缓存（30 天）
    google.rs  edge.rs  openai.rs
install.ps1 / install.sh   一键安装脚本
```

## 发布

推送 `v*` tag 触发 Actions：编译三平台、发布 Release、只保留最近 3 个版本。

## TODO

- 颜色可配置（T_COLOR）、原文压暗。
- 窗口 resize 同步给子进程。
- 行内 `--flag` / `` `code` `` 占位符保护、术语表。
