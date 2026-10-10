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
| `t --replace <命令...>`（或 `t -all` / `t --all`） | 译文替换原文，而非追加在原文下方 |
| `tt <命令...>` | 同上（`t --alias tt` 创建，命令名为 tt 时自动启用替换模式） |
| `t --update` | 更新到最新 Release |
| `t --alias [名字]` | 创建别名（会检查 PATH 冲突），例如 `tt` |
| `t --cache [clear]` | 查看 / 清空缓存 |
| `t --version` | 版本 |

## 配置（环境变量）

| 变量 | 说明 | 默认 |
|---|---|---|
| T_LANG | 目标语言 | zh-CN |
| T_BACKEND | `auto`（先探测 Google，3 秒内通就用，否则用 Bing 网页接口）/ `google` / `bing` / `openai`（兼容接口，DeepSeek、Ollama 也行） | auto |
| T_FOOTER | 屏幕模式翻译栏行数 | 6 |
| T_API_BASE | openai 后端地址 | https://api.openai.com/v1 |
| T_API_KEY / T_MODEL | openai 后端的 Key / 模型 | - / gpt-4o-mini |

缓存：`~/.t-translate/cache.json`，有效期 30 天，`--no-cache` 关闭。
术语保护：`` `code` `` 片段、`--flag` 参数、字母数字混合词（如 win10、pshell5）不会被翻译；
`~/.t-translate/no_translate.txt` 可加自定义词（每行一个，`#` 开头为注释）。

## 行为

- 普通输出：整行批量翻译，原文下方亮青色译文；帮助文档的两栏格式会把译文对齐到说明列。
- 替换模式（`t --replace` / `tt`）：只输出译文，不输出原文；帮助文档保留 flag 列、说明列换成译文。屏幕模式与交互提示不受影响。
- 流式输出（`tail -f`、`docker logs -f`）：每批最多等待 500ms 就翻译，持续不断的输出也不会卡住。
- 交互提示（以 `:` `?` `)` `]` `>` 结尾、空闲 200ms）：在提示后追加 `(译文)`，输入照常转发。
- 未结束且不像提示的半行（如逐字输出）：800ms 后原样输出，不会卡住。
- 进度条（`\r` 覆盖）：原样透传。
- **屏幕模式**（交互式 CLI：pi、codex、claude code 等会擦除重绘的程序，或进入全屏的程序）：
  检测到 `ESC[nA` / `ESC[2K` 等重绘指令或 alternate screen 后自动进入。程序输出原样透传，
  不插入任何行（否则会被它自己的重绘擦掉）；同时用虚拟终端还原当前屏幕，屏幕稳定 300ms 后
  把新出现的段落翻译，显示在终端底部的**翻译栏**（默认 6 行，`T_FOOTER` 可改，窗口小于 18 行时不启用）。
  程序看到的窗口会比实际少这几行。翻译在后台线程进行，不会卡住界面。
  旋转提示、计时器（`Working (12s)`）这类每帧都变的行会被识别并跳过；已翻译的段落走缓存，不会重复请求。
- 跳过：已含中文、过短、单个词/路径/哈希等。

## 代码结构

```
src/
  main.rs        入口分发
  cli.rs         参数解析
  runner.rs      PTY + 流式处理（Pump）+ 后台翻译线程
  screen.rs      屏幕模式：vt100 虚拟屏幕、分段、易变行过滤、翻译栏渲染
  render.rs      输出 / 对齐 / 高亮
  text.rs        ANSI 剥离、翻译过滤启发式（含单元测试）
  update.rs      t --update
  alias.rs       t --alias
  translator/
    mod.rs       后端选择（auto）、批量翻译
    cache.rs     磁盘缓存（30 天）
    google.rs  bing.rs  openai.rs
install.ps1 / install.sh   一键安装脚本
```

## 发布

推送 `v*` tag 触发 Actions：编译三平台、发布 Release、只保留最近 3 个版本。

## TODO

- 颜色可配置（T_COLOR）、原文压暗。
- 窗口 resize 同步给子进程。
- 隐私脱敏：输出含 token/key/password 等敏感信息时跳过翻译或打码。
