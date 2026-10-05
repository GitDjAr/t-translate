# t — 命令输出对照翻译

```
t git -h
t --lang ja docker logs -f web
t codex --help
```

## 构建

```
cargo build --release        # 产物 target/release/t(.exe)
cargo test                   # 过滤/ANSI 单元测试
```

Windows 把 `t.exe` 放进 PATH 即可；Linux/macOS 放到 `/usr/local/bin`。

## 配置（环境变量）

| 变量 | 说明 | 默认 |
|---|---|---|
| T_LANG | 目标语言 | zh-CN |
| T_BACKEND | `google`（免 Key 非官方接口）/ `openai`（兼容接口，DeepSeek、Ollama 也行） | google |
| T_API_BASE | openai 后端地址 | https://api.openai.com/v1 |
| T_API_KEY / T_MODEL | openai 后端的 Key / 模型 | - / gpt-4o-mini |

缓存：`~/.t-translate/cache.json`，`--no-cache` 关闭。

## 行为

- 普通/流式输出：按行（200ms 空闲或满 30 行为一批）批量翻译，原文下方灰色缩进显示译文。
- 交互提示（无换行结尾的行，空闲 200ms）：在提示后追加 `(译文)`，你的输入照常转发。
- 全屏 TUI（进入 alternate screen）：自动原样透传，不翻译。
- 跳过：已含中文、过短、单个词/路径/哈希等。

## 已知限制 / TODO

- 免费 Google 接口无 SLA，发布时建议默认让用户配置自己的后端。
- Windows 下方向键等转义输入依赖 ConPTY，需实测。
- 窗口 resize 暂未同步给子进程。
- 行内 `--flag` / `` `code` `` 占位符保护、术语表尚未实现。
- TODO（低优先级）：颜色可配置（T_COLOR）、原文压暗以突出译文、`t --update` 自更新（查 GitHub Releases，Windows 先改名旧 exe 再替换）。
