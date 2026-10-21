# NanoHermes

一个用 Rust 编写的轻量级、低内存终端编程智能体。NanoHermes 连接任意兼容 OpenAI 协议的大模型端点，为模型提供真实工具——读写编辑文件、搜索代码库、执行 shell 命令、应用补丁，以及通过模型上下文协议（MCP）调用外部工具。

设计目标是一个约 2 MB 的小体积二进制和极低的运行时内存：无异步运行时、阻塞式 HTTP 客户端、工具输出有上限、上下文自动压缩、无重型依赖。

## 功能特性

- **桌面应用** —— Tauri GUI，四区外壳（顶栏、任务栏、对话区、结果区）。任务既可在本机针对工作区文件夹运行，也可转发到远程 daemon；每个任务独立保存对话、权限模式、模型档案、人设与 token 计数，可置顶，也可用 `Ctrl+F` 在会话内查找。
- **人设与提示词片段** —— 人设是附加在内置系统提示词之后的一段指令，可保存多套并在输入区按任务切换；提示词片段是可一键插入输入框的可复用文本。
- **主题** —— 三套配色（石墨、午夜、素笺）加「跟随系统」，可在设置或 `Ctrl+K` 中切换；界面所有颜色都走 CSS 变量，换主题即换一套调色板。
- **兼容 OpenAI 协议** — 可对接 OpenAI、本地模型（Ollama、vLLM、llamafile）或任何支持 chat-completions 格式的端点。
- **流式输出** — 助手 token 边生成边打印。
- **容错** — 瞬时 API 故障（HTTP 429/5xx）按指数退避重试；重试耗尽后由可选的 `--fallback-model` 接管。
- **内置工具** — `read_file`、`write_file`、`edit_file`、`apply_patch`、`list_dir`、`grep`、`glob`、`run_command`、`web_fetch`、`web_search`、`todo_write`、`todo_read`、`git_status`、`git_diff`、`git_log`、`git_commit`、`task_output`、`task_list`、`task_kill`。条件性工具：`skill`（发现技能时）、`subagent` / `agent_list`（配置子代理时）、`memory_save` / `memory_search`（开启长期记忆时）。
- **计划清单** — `todo_write` / `todo_read` 让模型拟定并在多轮间跟踪任务清单。
- **Git** — `git_status` / `git_diff` / `git_log` 查看仓库状态；`git_commit` 可暂存并提交（需用户确认）。
- **用量统计** — 累计请求数与 token 用量，可通过 `/cost` 查看，并包含在 `json` / `stream-json` 输出中。
- **联网能力** — `web_fetch` 读取网址并返回纯文本（HTML 会被剥离为可读文本）；`web_search` 返回网页搜索结果，便于进一步抓取。
- **MCP 支持** — 通过 stdio 或 streamable HTTP 连接外部 MCP 服务器，自动发现并调用其工具。
- **MCP 资源与提示词** — 通过专用 agent 工具浏览和读取 MCP 资源、列出和渲染 MCP 提示词。
- **项目规则** — 将 `AGENTS.md`（全局 `~/.nanohermes/AGENTS.md` 以及工作区的每一级祖先目录）注入系统提示作为项目规则（上限 20 KB，自动去重）。
- **长期记忆** — 一轮对话完成后，agent 会把经验、偏好、纠正与事实沉淀到 `~/.nanohermes/memory.json`；下一轮按相关度检索并注入系统提示。打分使用本地 BM25（英文按小写单词、中文按二元切分），不需要额外服务；模型也可以通过 `memory_save` / `memory_search` 主动读写记忆。
- **技能（Skills）** — 从 `.nanohermes/skills/<name>/SKILL.md` 发现的可复用指令包。系统提示中只放入名称与描述，模型通过 `skill` 工具按需加载完整指令（渐进式披露）。
- **生命周期钩子** — 在配置文件中定义 `PreToolUse` / `PostToolUse` shell 钩子。`PreToolUse` 钩子可拦截工具调用；`PostToolUse` 钩子只能追加警告。
- **子代理** — 将聚焦的子任务委派给已配置的子代理，并行执行（并发有上限），记录在任务板上，可用 `agent_list` 查看。
- **沙箱与审批** — 为派生的命令提供 Linux Landlock 文件系统沙箱（`read-only` / `workspace-write`），以及三级审批策略（`never` / `on-request` / `untrusted`），并内置只读命令识别。
- **并行工具调用** — 当模型在一轮中返回多个工具调用时，并发执行。
- **会话持久化** — 通过 `--session` 将对话历史保存和恢复到 JSON 文件。
- **多模态输入** — 通过 `--image`（单次执行）或 `/image`（REPL）附加图片，供支持视觉的模型使用。
- **上下文压缩** — 当对话超过预算时，较早的消息会由模型生成摘要替代，而非静默丢弃。
- **中断与回退** — Ctrl-C 中止当前轮次而不杀死进程（对话保留）；`/undo [n]` 可回退一个或多个完整用户轮次。
- **REPL** — 当 stdin 为 TTY 时进入交互模式。

## 构建 CLI（从源码）

需要较新的稳定版 Rust 工具链。

```bash
git clone <仓库地址>
cd agent
cargo build --release
# 二进制：target/release/agent
```

release 配置使用 `opt-level = "z"`、LTO 和 `panic = "abort"` 以保持二进制体积小。

## 仓库结构

| 路径 | 说明 |
|------|------|
| `agent/` | 编码智能体 crate（`agent`）：CLI 二进制、agent 主循环、工具、MCP 客户端、daemon 模式。 |
| `gui/` | 桌面应用 crate（`gui`）：Tauri v2 的 Rust 外壳 —— `tauri.conf.json`、`src/`、`icons/`、`capabilities/`。 |
| `ui/` | 桌面前端（Vite + React + TypeScript）：`index.html`、`src/`、`package.json`。 |
| `scripts/` | 项目级部署脚本（`deploy-debian.sh`）。 |
| `target/` | Cargo 构建产物，整个 workspace 共用。 |

根目录的 `package.json` 是项目级 npm 入口 —— `npm run dev` / `npm run build` 转发到 `ui/`。

## 测试

```bash
npm install --prefix ui      # 首次：安装前端开发依赖
npm run test                 # 仅前端（vitest + jsdom + testing-library）
npm run test:rust            # 仅 Rust（cargo test --workspace）
npm run test:all             # 前后端全跑
```

- **前端**（`ui/src/**/*.test.{ts,tsx}`）—— Vitest + jsdom 环境。覆盖纯逻辑（主题解析与持久化、store 状态迁移、Markdown 净化、图片附件处理、会话内查找、diff 变更块），以及设置页与命令面板的组件测试；另有端到端套件 `ui/src/App.e2e.test.tsx`，用真实 store 与组件、仅 mock Tauri IPC 桥，覆盖超长会话虚拟滚动、跨窗口的查找命中、变更跳转，以及运行 / 停止中的输入区状态。配置在 `ui/vite.config.ts` 的 `test` 块，公共装配在 `ui/src/test/setup.ts`。
- **Rust** —— 单元测试在各模块内（`#[cfg(test)]`）；另有端到端套件 `agent/tests/e2e_mock_llm.rs`，用本地 mock 的 OpenAI 兼容服务驱动真实 `Agent`（仅依赖标准库、不走网络），覆盖工具调用往返、流式与非流式回复、瞬时 5xx 重试与硬错误。

## 桌面应用（Tauri GUI）

`gui/` 把 agent 封装成桌面应用：四区外壳，每个任务既可在本机进程内针对本地工作区文件夹运行，也可转发给远程 NanoHermes daemon 执行。

```bash
npm --prefix ui install                  # 安装前端依赖
npm run build                            # 类型检查 + 打包 ui/ → ui/dist
cd gui && npx --no-install tauri build   # release 构建 + 打包安装程序
# 安装包：target/release/bundle/nsis/NanoHermes_<version>_x64-setup.exe
#         target/release/bundle/msi/NanoHermes_<version>_x64_en-US.msi
```

`npm run dev`（在仓库根执行）启动 Vite 开发服务器；在 `gui/` 下执行 `npx --no-install tauri dev` 打开应用并连到它。安装包未做代码签名，首次运行 Windows SmartScreen 会提示。

### 窗口布局

| 区域 | 内容 |
|------|------|
| 顶栏 | 自绘而非系统标题栏：品牌标识、模型/执行位置胶囊（点击打开设置）、任务栏与结果区开关、设置、窗口按钮。 |
| 任务栏（左侧，可折叠） | 全部任务及其执行位置和累计 token 用量；支持按标题/内容搜索、置顶（置顶任务单独成组排在最前）、重命名、删除、导出。 |
| 对话区（中间） | 对话上方有任务条，显示任务名、运行状态（`就绪` / `执行中` / `停止中`）、累计 token，以及导出与清空上下文。下方是流式 Markdown：用户提问、助手回复、可展开输出的工具调用、单条消息重试。 |
| 结果区（右侧，可折叠） | 工作区文件树与本任务产物（可按全部 / 已修改 / 已读取筛选）、只读文件预览，以及变更文件的 diff（含词级高亮与上一处 / 下一处跳转）。 |

### 任务、工作区与执行位置

- 每个任务拥有自己的对话和自己的工作区文件夹。本机任务绑定文件夹选择器里选中的目录（也可直接输入路径），所有工具调用的相对路径都相对它解析。
- 执行位置是任务级的：`local` 在应用进程内运行，`cloud` 把这一轮转发给远程 daemon（见 [HTTP daemon 模式](#http-daemon-模式)）并带上 `X-Session-Id`，因此每个任务在服务端也有独立对话。
- 本机任务可在步骤边界停止；云端任务无法取消，因为整次请求由 daemon 持有。
- 置顶的任务会单独成一组、固定排在列表最前，不参与「今天／昨天／本周」的日期分组；在任务菜单或 `Ctrl+K` 里切换。

### 权限模式

| 值 | 界面标签 | 行为 |
|----|---------|------|
| `default` | Agent | 全部工具可用，文件修改与命令自动执行。 |
| `accept-edits` | Accept | 自动执行文件修改，命令仍需确认。 |
| `plan` | Plan | 只读分析：不写文件、不执行命令。 |
| `goal` | Goal | 目标模式：无人值守，所有工具自动执行直到完成。 |
| `debug` | Debug | 诊断模式：可读文件、可执行命令以复现问题，但不修改文件。 |

模式存在设置里作为新任务默认值，也可以在输入区或命令面板切换。这仅对本机任务生效：云端任务使用 daemon 启动时固定的权限模式（见 [在 VPS 上部署 daemon](#在-vps-上部署-daemon)）。

### 输入区

- `Enter` 发送，`Shift+Enter` 换行。
- `↑` / `↓` 在光标位于首行 / 末行时翻阅输入历史；翻回最新一条时恢复原先未发送的草稿。
- `@` 打开工作区文件选择器，并把选中路径插入光标处。
- `+` 打开工作区文件选择器，选中的文件会内联进提示词。
- 运行中发送按钮变为**停止**。停止请求在下一个步骤边界生效：按钮保持转圈禁用，任务条显示 `停止中`，本轮收尾时提示「任务已停止」并把焦点交回输入框。运行中按 `Enter` 不会被静默丢弃，而是提示先完成或停止。
- 模式切换左侧的模型胶囊用来选本任务使用哪个模型档案；每个任务各自记住自己的选择，选项来自**设置 → 模型**里的列表。
- 模型胶囊旁的人设胶囊用来选本任务使用哪套人设（附加系统提示词）；附加了指令的人设会高亮，选项来自**设置 → 人设**里的列表。
- 闪电按钮打开**提示词片段**菜单，点一下就把该段文字插到当前输入末尾；片段在**设置 → 提示词片段**里维护。
- 图片：可粘贴截图、拖入文件或选择文件。每次最多 4 张，单张按最长边缩到 1024 px；非可解码图片或超过 12 MB 会带原因拒绝。图片以 data URL 传入，因此需要支持视觉的模型。
- 模型、人设与「高级」里的运行参数只对本机任务生效；云端任务的行为由远端 daemon 决定，因此云端任务的输入区不展示模型/人设胶囊。

### 结果区

- **文件** —— 任务目录的文件树，点击文件打开只读预览（上限 200 KB），行号列固定、横向滚动只作用在代码本体。
- **产物** —— 本任务写入或读取过的文件，可按全部 / 已修改 / 已读取筛选。
- **变更** —— 打开被 `write_file` / `edit_file` / `apply_patch` 改过的文件时显示 diff：行级增删背景、行内语法着色、同行内词级变化高亮，以及 `n / m` 变更计数和**上一处 / 下一处变更**按钮（滚动到对应变更块，越界回绕）。不在 Git 仓库内时面板给出提示而不是报错。

### 设置

`Ctrl+,` 打开设置。所有字段输入即存（700 ms 去抖，失焦与关闭页面时兜底落盘）；设置页顶部显示 `保存中…` / `已自动保存` / `保存失败`，**保存**按钮可立即落盘。

| 分组 | 字段 |
|------|------|
| 主题 | 石墨（深色，默认）、午夜（深色）、素笺（浅色），或跟随系统；即时生效并记住选择。 |
| 模型 | 一组模型档案（名称、API Key、Base URL、模型名）—— 想加几个加几个，在输入区按任务切换。**设为默认**标记新任务使用的那个；**测试连接**用屏幕上当前的值探测该端点的 `/models`。 |
| 人设 | 一组人设（名称 + 附加系统提示词）—— 附加指令拼在内置提示词之后，内置的规则与安全约束仍然生效。留空的人设不附加任何内容；**当前默认**标记新任务使用的那套，任务也可以各选各的。 |
| 提示词片段 | 可复用的短语或模板，在输入区点闪电即可插入。 |
| 高级 | 本机任务的运行参数：深度思考、读取项目指南（AGENTS.md / NANOHERMES.md）、长期记忆（默认开启；关闭后不再提炼新记忆，已有条目保留）、温度、最大迭代、最大输出 tokens。 |
| 执行位置 | 在本机进程内运行，或转发到远程 daemon；含 daemon 基地址与 Bearer Token —— **检查**探测 `GET /health`。 |
| 工作区 | 默认项目目录；每个任务可单独覆盖。 |
| 默认项 | 新任务的默认执行位置、默认权限模式。 |

### 快捷键

| 快捷键 | 作用 |
|--------|------|
| `Ctrl+K` | 命令面板 —— 覆盖所有动作（新建任务、模式、面板、导出、设置……）。 |
| `Ctrl+N` | 新建任务；任务栏可见时先询问本机还是云端。 |
| `Ctrl+F` | 在会话内查找；`Enter` / `Shift+Enter` 跳到下一个 / 上一个命中，`Esc` 关闭查找条。 |
| `Ctrl+,` | 设置。 |
| `Ctrl+B` / `Ctrl+J` | 切换任务栏 / 结果区。 |
| `Ctrl+/` | 快捷键面板。 |
| `↑` / `↓`（输入区） | 从首行 / 末行翻阅输入历史。 |
| `@`（输入区） | 唤起文件选择器并插入选中路径。 |
| `Esc` | 逐层关闭最上层浮层：命令面板 → 设置 → 菜单 → 查找条 → 预览。 |
| `Tab`、`←` / `→`、`Esc`（设置页） | 在字段间移动、在选项组内切换、保存并关闭。 |

### 本地状态

| 项 | 位置 |
|----|------|
| GUI 设置（含模型档案、人设、提示词片段、高级参数） | `~/.nanohermes/gui-settings.json` |
| 长期记忆 | `~/.nanohermes/memory.json` |
| 任务对话 | `~/.nanohermes/sessions/<task-id>.json` |
| 导出的 Markdown | `~/.nanohermes/exports/` |
| 界面状态（任务列表、草稿、面板宽度） | `localStorage`，键 `nanohermes.state.v1` |
| 主题 | `localStorage`，键 `nanohermes.theme.v1` |

流式文本通过 `nanohermes://delta|tool|note|usage` 事件通道送达前端，每 40 ms 合并一次。它走独立的订阅通道而不是全局 store，只有正在输出的那条消息会重渲染，每个阶段结束时才把文本定稿写进对话；运行期间界面推迟落盘、只在阶段边界提交，因此长篇回答不会反复写 `localStorage`。

## 快速开始

```bash
# 单次执行
agent "为 src/tools/search.rs 添加单元测试"

# 交互 REPL
agent

# 从 stdin 读取提示
echo "解释 main.rs" | agent

# 本地模型
agent --base-url http://localhost:11434/v1 -m llama3 "列出文件"
```

## 配置

NanoHermes 通过环境变量和 CLI 参数配置，CLI 参数优先。

| 环境变量 | 等价参数 | 默认值 | 说明 |
|----------|---------|--------|------|
| `NANOHERMES_API_KEY` / `OPENAI_API_KEY` | `-k, --api-key` | _(空)_ | 端点的 API key |
| `NANOHERMES_BASE_URL` / `OPENAI_BASE_URL` | `-u, --base-url` | `https://api.openai.com/v1` | 兼容 OpenAI 的 base URL |
| `NANOHERMES_MODEL` / `OPENAI_MODEL` | `-m, --model` | `gpt-4o-mini` | 模型名称 |

### CLI 参数

```
agent [OPTIONS] [PROMPT...]      单次执行：运行 PROMPT 后退出
agent [OPTIONS]                  交互 REPL（当 stdin 为 TTY 时）
echo "PROMPT" | agent [OPTIONS]  从 stdin 读取提示并单次执行
```

| 参数 | 说明 |
|------|------|
| `-m, --model <NAME>` | 模型名称 |
| `-u, --base-url <URL>` | 兼容 OpenAI 的 base URL（不含 `/chat/completions`） |
| `-k, --api-key <KEY>` | API key |
| `-C, --cwd <DIR>` | 工作区根目录（默认：当前目录） |
| `-i, --image <PATH>` | 附加图片（可重复）；需要支持视觉的模型 |
| `--mcp-config <FILE>` | MCP 服务器配置 JSON 路径（启用 MCP 工具） |
| `--subagents <FILE>` | 子代理配置 JSON 路径（启用 `subagent` / `agent_list` 工具） |
| `--session <FILE>` | 将对话历史持久化到 FILE（启动时若存在则加载） |
| `-n, --max-iterations <N>` | 每轮模型↔工具的最大往返次数（默认 40） |
| `--max-tokens <N>` | 最大响应 token 数（默认 4096） |
| `--temperature <F>` | 采样温度（默认 0.0） |
| `--timeout <SECS>` | HTTP 请求超时（默认 120） |
| `--max-retries <N>` | 瞬时 API 故障重试次数（默认 2，环境变量 `NANOHERMES_MAX_RETRIES`） |
| `--retry-base-ms <MS>` | 重试的基础退避间隔（默认 500，环境变量 `NANOHERMES_RETRY_BASE_MS`） |
| `--no-stream` | 关闭流式输出（缓冲响应） |
| `--ask` | 每条 shell 命令执行前确认（`untrusted` 策略） |
| `--approval-policy <POLICY>` | 何时确认 shell 命令：`never` \| `on-request` \| `untrusted`（默认 `never`，即自动批准） |
| `--sandbox <MODE>` | 派生命令的文件系统沙箱：`off` \| `read-only` \| `workspace-write`（Linux/Landlock） |
| `-y, --yes` | 自动批准 shell 命令（`never` 策略；默认） |
| `--no-project-docs` | 不将 `AGENTS.md` 加载进系统提示 |
| `--daemon` | 以 HTTP daemon 运行：`POST {"prompt":"..."}` → `{"response":"..."}` |
| `--daemon-port <PORT>` | daemon 监听端口（默认 8787） |
| `--daemon-bind <ADDR>` | daemon 绑定地址（默认 `127.0.0.1`） |
| `--daemon-token <TOKEN>` | daemon 请求所需的 Bearer token（或环境变量 `NANOHERMES_DAEMON_TOKEN`） |
| `-h, --help` | 显示帮助 |
| `-V, --version` | 显示版本 |

> HTTP 请求遵循标准代理环境变量（`HTTPS_PROXY`、`HTTP_PROXY`、`ALL_PROXY`、`NO_PROXY`）。`localhost` 和 `NO_PROXY` 中的主机自动绕过代理。

## REPL 命令

交互模式下可用：

| 命令 | 作用 |
|------|------|
| `/clear` | 重置对话（保留系统提示） |
| `/undo [n]` | 回退最近 `n` 个完整用户轮次（默认 1） |
| `/model <name>` | 切换当前模型 |
| `/mode <mode>` | 切换权限模式（`default`、`accept-edits`、`plan`） |
| `/format <fmt>` | 切换输出格式（`text`、`json`、`stream-json`） |
| `/tools` | 列出可用工具 |
| `/status` | 显示会话统计 |
| `/cost` | 显示累计 token 用量 |
| `/compact` | 汇总较早消息以释放上下文 |
| `/image <path>` | 为下一个任务附加图片（需支持视觉的模型） |
| `/images` | 列出已排队图片 |
| `/drop-images` | 清空已排队图片 |
| `/save <file>`、`/load <file>` | 保存 / 加载对话 |
| `/help` | 显示命令 |
| `/exit`、`/quit`、`:q` | 退出 |

其他输入均作为任务发送给智能体。以 `!` 开头的行会直接执行 shell 命令。

## 内置工具

所有路径相对于工作区根目录解析，除非是绝对路径。

### `read_file`
读取文本文件并带行号。

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `path` | string | _(必填)_ | 文件路径 |
| `offset` | integer | 1 | 从第几行开始读（1-based） |
| `limit` | integer | 2000 | 最多读取行数 |

超过 4 MB 的文件会被拒绝；请用 `offset`/`limit` 分块读取。

### `write_file`
创建或覆盖文件。自动创建父目录。

| 参数 | 类型 | 说明 |
|------|------|------|
| `path` | string | 文件路径 |
| `content` | string | 完整文件内容 |

### `edit_file`
替换文件中的精确字符串。`old_string` 必须逐字节匹配，包括缩进。

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `path` | string | _(必填)_ | 文件路径 |
| `old_string` | string | _(必填)_ | 要查找的精确文本 |
| `new_string` | string | _(必填)_ | 替换文本 |
| `replace_all` | boolean | false | 替换所有匹配项 |

如果 `old_string` 出现多次且 `replace_all` 为 false，调用会失败——请提供更多上下文或设置 `replace_all=true`。

### `apply_patch`
对文件应用 unified diff（补丁）。diff 按内容匹配（非行号），因此对小范围漂移有容错性。

| 参数 | 类型 | 说明 |
|------|------|------|
| `path` | string | 要打补丁的文件路径 |
| `diff` | string | 含 `@@` hunk 的 unified diff 文本 |

### `list_dir`
列出目录条目（目录以 `/` 结尾，文件显示大小）。

| 参数 | 类型 | 默认值 |
|------|------|--------|
| `path` | string | `.`（工作区根目录） |

### `grep`
递归搜索文件内容中的字面子串。自动跳过二进制文件和构建/工具目录（`.git`、`target`、`node_modules` 等）。

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `pattern` | string | _(必填)_ | 要搜索的字面文本 |
| `path` | string | 工作区根目录 | 要搜索的文件或目录 |
| `include` | string | _(无)_ | glob 过滤，如 `*.rs` 或 `src/**/*.ts` |
| `case_insensitive` | boolean | false | 忽略大小写 |

结果上限为 200 条匹配。

### `glob`
按 glob 模式在工作区根目录下查找文件。支持 `*`、`?` 和 `**`。

| 参数 | 类型 | 说明 |
|------|------|------|
| `pattern` | string | glob 模式，如 `src/**/*.rs` |

结果上限为 500 个文件。

### `run_command`
通过 `sh -c` 在工作区执行 shell 命令。

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `command` | string | _(必填)_ | 要执行的 shell 命令 |
| `cwd` | string | 工作区根目录 | 工作目录 |
| `timeout_secs` | integer | 120 | 超时秒数后杀死命令（最大 3600） |

stdout 和 stderr 会被捕获（各上限 256 KB）并连同退出码一起返回。由于终端无 TTY，请使用非交互式参数。设置 `background=true` 可将长任务放入后台，再用 `task_output` 轮询。

### `web_fetch`
通过 HTTP(S) 抓取网址并返回可读纯文本。HTML 标签、`<script>`/`<style>` 内容与实体都会移除，空白会被折叠。请求遵循代理环境变量。

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `url` | string | _(必填)_ | `http://` 或 `https://` 网址 |
| `max_chars` | integer | 20000 | 返回文本的最大字符数（500–200000） |

响应体上限为 2 MB，超出部分会被截断。

### `web_search`
联网搜索（DuckDuckGo HTML 端点），返回若干结果的标题 / 网址 / 摘要。可对结果网址继续调用 `web_fetch` 阅读页面。

| 参数 | 类型 | 默认值 | 说明 |
|------|------|--------|------|
| `query` | string | _(必填)_ | 搜索关键词 |
| `count` | integer | 5 | 结果数量（1–10） |

### `todo_write` / `todo_read`
维护当前会话的有序任务清单。`todo_write` 会整体替换清单（并在终端回显），让模型拟定计划并更新进度；`todo_read` 返回当前清单。

| 参数 | 类型 | 说明 |
|------|------|------|
| `todos` | `{content, status}` 数组 | 按顺序排列的完整清单（`status` 为 `pending`、`in_progress` 或 `completed`，默认 `pending`） |

最多保留 50 项；`content` 为空会被拒绝。

### `git_status` / `git_diff` / `git_log`
在工作区根目录执行只读的 git 查询（调用 `git` 二进制）。

| 工具 | 参数 | 说明 |
|------|------|------|
| `git_status` | `path`（可选） | 简短的工作区状态与当前分支 |
| `git_diff` | `staged`（布尔，可选）、`path`（可选） | 统一 diff；`staged=true` 比较暂存区 |
| `git_log` | `count`（整数，默认 10，最大 100）、`path`（可选） | 近期提交（每行一条） |

### `git_commit`
可选地暂存并创建提交。除非开启自动批准，否则会先请求用户确认；在 `plan` 权限模式下被禁用。

| 参数 | 类型 | 说明 |
|------|------|------|
| `message` | string | 提交信息（必填） |
| `all` | boolean | 提交前暂存全部改动（默认 false） |
| `files` | string 数组 | 提交前要暂存的路径（可选） |

### `task_output` / `task_list` / `task_kill`
获取、列出或终止由 `run_command`（`background=true`）启动的后台任务。

### `memory_save` / `memory_search`
开启长期记忆（配置项 `"memory": true`，默认开启）时可用。

| 工具 | 参数 | 说明 |
|------|------|------|
| `memory_save` | `kind`（`lesson` / `preference` / `correction` / `fact`）、`text` | 记录一条长期有效的信息；文本完全重复时视为重复、不新增 |
| `memory_search` | `query`、`top_k`（默认 5，范围 1–20） | 检索记忆库并返回带分数的条目 |

条目存放在 `~/.nanohermes/memory.json`（可在配置文件中用 `memory_dir` 覆盖）。检索用 BM25 对英文小写单词与中文二元切分打分，完全本地运行，不需要向量化服务。

## MCP（模型上下文协议）

NanoHermes 内置一个精简的 MCP 客户端，支持两种传输方式。用 JSON 文件配置服务器：

```json
{
  "servers": {
    "filesystem": {
      "command": "npx",
      "args": ["-y", "@modelcontextprotocol/server-filesystem", "/path/to/dir"]
    },
    "custom": {
      "command": "python3",
      "args": ["my_mcp_server.py"]
    },
    "remote": {
      "url": "https://host.example/mcp",
      "headers": { "Authorization": "Bearer <token>" }
    }
  }
}
```

```bash
agent --mcp-config mcp.json "列出 MCP 服务器暴露的文件"
```

带 `command`（及可选 `args`）的条目作为子进程启动，通过 JSON-RPC 2.0 配合 LSP 风格的 `Content-Length` 帧通信。带 `url` 的条目使用 **streamable-HTTP** 传输：每个请求是一次同步 POST（可带 `headers` 映射），返回的会话 id 会在后续调用中复用。`tools/list` 返回的工具会合并到 agent 的工具列表中，并按名称自动分发。

### MCP 资源与提示词

除工具外，NanoHermes 还提供四个辅助 agent 工具用于 MCP 资源和提示词：

| 工具 | 说明 |
|------|------|
| `mcp_list_resources` | 列出所有 MCP 服务器的资源 |
| `mcp_read_resource` | 按 URI 读取 MCP 资源的文本内容 |
| `mcp_list_prompts` | 列出所有 MCP 服务器的提示词 |
| `mcp_get_prompt` | 按名称渲染 MCP 提示词，可带参数 |

这些工具仅在配置了至少一个 MCP 服务器时可用。

## 并行工具调用

当模型在一条 assistant 消息中返回多个 `tool_calls` 时，NanoHermes 使用 scoped 线程**并发**执行它们。结果按调用顺序收集后统一追加，因此每个 `tool_call_id` 都与正确的结果配对，不受执行完成顺序影响。

- 内置工具并行运行（它们不共享可变状态）。
- MCP 工具调用在同一服务器内串行（在服务器的 stdin 锁上），但跨不同服务器并行。
- 单个工具调用走直接路径，无线程开销。

## 项目规则（AGENTS.md）

当 `project_docs` 启用（默认）时，NanoHermes 会读取 `AGENTS.md` 文件并注入系统提示作为项目规则。来源顺序：

1. 全局规则：`~/.nanohermes/AGENTS.md`。
2. 从文件系统根目录到工作区根目录的每一级目录，由外向内，因此更具体的规则出现在后面。

内容上限 20 KB，重复文件会自动去重。可用 `--no-project-docs` 或配置文件中 `"project_docs": false` 关闭。

## 技能（Skills）

一个技能是包含 `SKILL.md` 的目录，`SKILL.md` 可带 YAML frontmatter：

```markdown
---
name: release
description: 发布流程：升版本号、更新 changelog、打 tag 并推送。
---

第 1 步：运行 `cargo test` ……
```

技能从工作区的 `.nanohermes/skills/<name>/SKILL.md` 以及 `~/.nanohermes/skills/<name>/SKILL.md` 发现；同名时工作区技能覆盖 home 技能。当至少发现一个技能时，NanoHermes 会注册 `skill` 工具，并在系统提示中列出每个技能的名称与描述。模型只有在需要时才调用 `skill` 并传入名称，加载该技能的完整指令（上限 20 KB）——渐进式披露让基础提示保持精简。若无 frontmatter，则用目录名作为技能名、正文第一行作为描述。

## 生命周期钩子

钩子是围绕每次工具调用运行的 shell 命令，在配置文件中定义：

```json
{
  "hooks": [
    { "event": "PreToolUse",  "matcher": "run_command", "command": "echo checking >&2" },
    { "event": "PostToolUse", "matcher": "*",           "command": "./scripts/audit.sh" }
  ]
}
```

- `event` 为 `PreToolUse` 或 `PostToolUse`。
- `matcher` 为 `*` / 空（匹配所有工具），或以逗号分隔的工具名列表。
- 钩子从 stdin 接收工具名与 JSON 参数，在工作区根目录运行。

`PreToolUse` 钩子以非零退出会**拦截**该工具调用，并把其输出返回给模型。`PostToolUse` 钩子以非零退出只在工具结果中追加一条警告。

## 子代理

在 JSON 文件中定义专用子代理，用 `--subagents` 传入：

```json
{
  "subagents": {
    "reviewer": {
      "prompt": "你是一名严谨的代码审查员，只报告具体问题。",
      "allowed_tools": ["read_file", "grep", "list_dir"]
    },
    "tester": {
      "prompt": "你负责编写并运行聚焦的测试。",
      "model": "gpt-4o-mini"
    }
  }
}
```

配置后会出现两个工具：

| 工具 | 说明 |
|------|------|
| `subagent` | 将 `task` 委派给指定子代理；它独立运行并返回最终结果 |
| `agent_list` | 查看本会话的子代理任务板（id、状态、名称、任务、耗时、结果预览） |

子代理并行运行，受并发闸门限制（同时最多 4 个）。每个任务在开始时即登记到任务板，因此 `agent_list` 能报告实时状态。子代理可用 `allowed_tools` 限制其工具集，并可用 `model` 覆盖模型。

## 沙箱与审批策略

两个独立设置控制派生的 shell 命令能做什么。

**审批策略**决定何时向用户确认命令：

| 策略 | 行为 |
|------|------|
| `never` | 所有命令直接运行，不询问（等同自动批准；`-y` 时的默认值） |
| `on-request` | 仅对**非**只读命令询问 |
| `untrusted` | 所有命令都询问，包括只读命令 |

只读识别覆盖一小组安全程序（`ls`、`cat`、`grep`、`git status`/`diff`/`log` 等）。任何 shell 元字符（`|`、`&`、`;`、重定向）或命令替换都会使命令变为不可信，无论开头是什么程序。

**沙箱模式**限制派生命令的文件系统视图（仅 Linux，通过 **Landlock**）：

| 模式 | 行为 |
|------|------|
| `off` | 无限制（默认） |
| `read-only` | 可任意读 + 执行；完全不可写 |
| `workspace-write` | 可任意读 + 执行；仅允许在工作区内写 |

沙箱在子进程 `fork` 之后、`exec` 之前安装，因此同样覆盖该命令派生的所有进程。若内核不支持 Landlock，NanoHermes 会**拒绝**运行命令，而非静默降级。仅使用裸系统调用——无外部 crate。

## 中断与回退

在一轮进行中按 Ctrl-C 会设置标志位，而不是杀死进程。agent 循环在迭代边界轮询该标志，因此当前轮次干净地停止、控制权回到 REPL，且对话完整保留（不会出现孤立的工具调用）。空闲时，Ctrl-C 仍照常退出。嵌套轮次（例如工具调用中运行的子代理）通过引用计数共用一个处理器。

用 `/undo [n]` 回退：从对话历史中移除最近 `n` 个完整用户轮次（每轮的 user 消息及其之后的所有内容），并持久化结果。`/undo` 单独使用即回退一轮。

## HTTP daemon 模式

将 NanoHermes 作为常驻 HTTP 服务运行，而不是单次 CLI 或 REPL：

```bash
agent --daemon --daemon-port 8787 \
       --daemon-token "$(openssl rand -hex 16)" \
       --session /data/session.json
```

| 端点 | 方法 | 鉴权 | 说明 |
|------|------|------|------|
| `/` | `POST` | Bearer token（如设置） | 请求体 `{"prompt": "..."}` → `{"response": "...", "session": "<id>"}` |
| `/health` | `GET` | 无 | 返回 `{"status":"ok","sessions":N}`，用于存活探针 |

- 对话**按会话隔离**。发送 `X-Session-Id: <名称>` 头选择一个对话；不同 id 的请求绝不共享上下文。省略该头则使用 `default` 会话。每个会话持久化为 `--session` 目录下独立的 JSON 文件。
- daemon 默认绑定 `127.0.0.1`。仅在受信任的反向代理/隧道之后才设置 `--daemon-bind 0.0.0.0`。
- 当端口可被 localhost 以外访问时，**务必设置 token**（`--daemon-token` 或 `NANOHERMES_DAEMON_TOKEN`）。没有有效 `Authorization: Bearer <token>` 头的请求返回 `401`。
- 生产环境建议放在反向代理（如 Cloudflare Tunnel、Caddy、nginx）后面终止 TLS，daemon 保持 loopback。

```bash
# 存活探针
curl http://127.0.0.1:8787/health

# 执行任务
curl -X POST http://127.0.0.1:8787/ \
  -H "Authorization: Bearer $TOKEN" \
  -H "Content-Type: application/json" \
  -d '{"prompt": "列出 src 目录"}'

# 不同 X-Session-Id = 隔离的对话
curl -X POST http://127.0.0.1:8787/ \
  -H "Authorization: Bearer $TOKEN" \
  -H "X-Session-Id: project-alpha" \
  -H "Content-Type: application/json" \
  -d '{"prompt": "继续重构"}'
```

## 在 VPS 上部署 daemon

### 一键安装

`scripts/deploy-debian.sh` 在 Debian/Ubuntu 上一键装好 daemon（需 root）：

```bash
sudo bash scripts/deploy-debian.sh                 # 从源码构建 agent（缺 Rust 会自动安装）
sudo bash scripts/deploy-debian.sh /path/to/agent  # 或安装现成的二进制
```

| 步骤 | 说明 |
|------|------|
| 安装 | 二进制 `/opt/nanohermes/nanohermes`，工作区 `/opt/nanohermes/workspace`，会话目录 `/opt/nanohermes/sessions`（权限 700）。 |
| Token | 生成 `openssl rand -hex 24` 写入 `/opt/nanohermes/.daemon_token`（重跑时复用），并作为 `NANOHERMES_DAEMON_TOKEN` 注入。 |
| 服务 | 写入 `/etc/systemd/system/nanohermes.service`：`--daemon --daemon-port 8787 -C <workspace> --session <dir>/default.json --sandbox workspace-write`，`Restart=always`，并带 `NoNewPrivileges` 与 `ProtectSystem=strict` + `ReadWritePaths`。 |
| 启动 | `systemctl enable --now`，随后 curl `http://127.0.0.1:8787/health` 并打印 token。 |

脚本会提示输入大模型 API key（`NANOHERMES_API_KEY`），除非该环境变量已存在。1 GB 内存的机器在编译前先加 swap——整个部署中只有编译吃内存：

```bash
fallocate -l 2G /swapfile && chmod 600 /swapfile && mkswap /swapfile && swapon /swapfile
echo '/swapfile none swap sw 0 0' >> /etc/fstab
```

### 让 daemon 指向模型

脚本生成的 unit 只带 API Key；用非 OpenAI 端点时，通过 drop-in 覆盖 Base URL 与模型，再重启：

```bash
sudo mkdir -p /etc/systemd/system/nanohermes.service.d /root/.nanohermes
sudo tee /etc/systemd/system/nanohermes.service.d/env.conf >/dev/null <<'EOF'
[Service]
Environment="NANOHERMES_BASE_URL=https://openrouter.ai/api/v1"
Environment="NANOHERMES_MODEL=anthropic/claude-sonnet-4"
# 长期记忆写入 ~/.nanohermes/memory.json，会被 ProtectSystem=strict 拦住。
ReadWritePaths=/root/.nanohermes
EOF
sudo systemctl daemon-reload && sudo systemctl restart nanohermes
```

关闭长期记忆（`"memory": false`）时可以去掉 `ReadWritePaths` 这行。同一个 drop-in 也接受其他 `NANOHERMES_*` 变量，以及用 `ExecStart=` 覆盖启动参数（例如追加 `--permission-mode goal`）。

### 安全地暴露

daemon 只讲明文 HTTP，默认监听 `127.0.0.1:8787`。保持监听在本机，前面套一层：

- **SSH 隧道（单用户、不开端口）** —— `ssh -N -L 8787:127.0.0.1:8787 root@<host>`，桌面端填 `http://127.0.0.1:8787`。
- **空闲端口上的 TLS 反代** —— 443 已被占用（另一个代理、Xray、Web 服务器）时改用 8443。Let's Encrypt 的 HTTP-01 仍要走 80 端口，需保持可达：
  ```bash
  sudo apt install -y caddy
  sudo tee /etc/caddy/Caddyfile >/dev/null <<'EOF'
  daemon.example.com:8443 {
      reverse_proxy 127.0.0.1:8787
  }
  EOF
  sudo systemctl reload caddy
  ```
  没有域名时，`<IP-用横线连接>.sslip.io` 会自动解析到该主机（例如 `203-0-113-7.sslip.io`）。
- **Cloudflare Tunnel** —— 把 `127.0.0.1:8787` 映射到域名，不开放任何入站端口。

不要把 8787 直接暴露到公网：它唯一的凭证就是 Bearer Token，而且流量是明文的。

### 连接桌面应用

1. **设置 → 执行位置**：选**云端**，填地址（`https://daemon.example.com:8443` 或隧道地址）和 `/opt/nanohermes/.daemon_token` 里的 Token，点**检查**（`GET /health`）。
2. 之后新建任务可选云端；每个任务带自己的 `X-Session-Id`，对话在 daemon 上彼此隔离。
3. 云端一轮完全按 daemon 的配置执行：模型档案、人设、高级参数与权限模式**都不会**从桌面端发过去。权限模式在 daemon 启动时由 `--permission-mode` 固定，要改只能重启服务。云端任务也无法中途取消——整次请求由 daemon 持有。

### 日常运维

| 事项 | 命令 |
|------|------|
| 状态 / 日志 | `systemctl status nanohermes` · `journalctl -u nanohermes -f` |
| 更新 | 替换 `/opt/nanohermes/nanohermes` 后 `systemctl restart nanohermes` |
| 备份 | 复制 `/opt/nanohermes/sessions/` 与 `/opt/nanohermes/workspace/` |
| 轮换 Token | `rm /opt/nanohermes/.daemon_token`，重跑部署脚本，再更新桌面端设置 |
| 查看 Token | `cat /opt/nanohermes/.daemon_token`（权限 600，仅 root） |

> daemon 以 root 运行，其工具可在工作区内读文件、写文件、执行命令。任何拿到 Token 的人都有同等权限，因此 Token 不要外传，入口尽量不要公开，并及时给主机打补丁。

> 源码 crate 名为 `agent`，因此本机构建出的二进制是 `target/release/agent`；部署脚本在服务器上将其安装为 `/opt/nanohermes/nanohermes`，与 systemd 服务名 `nanohermes.service` 及 `NANOHERMES_*` 环境变量保持一致。

## 会话持久化

```bash
# 开始新会话（首轮结束后创建文件）
agent --session work.json "重构 search 模块"

# 之后恢复——历史从文件加载
agent --session work.json
```

- 会话文件是一个 OpenAI 风格消息（system、user、assistant、tool）的 JSON 数组。
- 写入是原子的（先写 `.tmp` 临时文件再 rename），因此写入中途崩溃不会截断会话。
- 每轮结束后及 `/clear` 时保存文件。
- 如果启动时无法加载文件，NanoHermes 回退到全新对话并打印警告。

## 上下文管理

对话以 OpenAI 消息对象形式保存在内存中。当总大小超过约 120 KB 时，NanoHermes 会压缩：最旧的消息（保留最近 10 条之外）被替换为一条系统消息，内容是模型对被丢弃区域生成的摘要。如果摘要生成失败，则使用纯截断提示。随时可用 `/compact` 手动触发。

## 多模态输入

为支持视觉的模型附加一张或多张图片：

```bash
agent -m gpt-4o -i screenshot.png "这个界面显示了什么？"
agent -m gpt-4o -i a.png -i b.png "比较这两个图"
```

图片从磁盘读取、base64 编码后，作为 `image_url` 内容片段与文本提示一起发送。

在 REPL 中可用 `/image <path>` 排队图片，它们会附加到下一个任务：

```
› /image screenshot.png
› 这个界面显示了什么？
```

## 架构与设计说明

- **无异步运行时。** HTTP 使用阻塞式 `ureq` 客户端；MCP I/O 每个服务器使用一个专用读取线程；工具调用在 scoped 系统线程上运行。这避免了 tokio/异步运行时的内存开销。
- **极简依赖。** 仅 `serde_json`、`ureq`（含 TLS）和 `base64`。不使用 `serde` 派生宏（值以 `serde_json::Value` 处理）。不依赖 `libc`：Landlock 沙箱与 SIGINT 处理器通过一个极小的裸系统调用垫片（`agent/src/sys.rs`）直达内核。
- **输出有界。** 工具结果限制为 20 KB；命令输出每个流上限 256 KB；文件读取拒绝超过 4 MB 的文件；项目规则与技能正文各上限 20 KB。这保证了上下文窗口和进程内存有界。
- **线程安全。** `Server` 将其 `Child` 进程和读取线程的 `JoinHandle` 包在 `Mutex` 中，使 `Manager`（进而 `Agent`）为 `Sync`，从而支持安全的并发工具执行。

## 桌面前端性能说明

- **Tauri 命令不阻塞主线程。** 同步的 `#[tauri::command]` 会在窗口主线程上执行，任何磁盘读写或网络请求都会让整个界面卡住。因此凡是要碰文件系统、起子进程或发 HTTP 请求的命令都写成 `async fn`，把工作交给 `tauri::async_runtime::spawn_blocking`；只有纯内存的 `get_settings` 与 `stop_task` 保持同步。并发写入同一份设置时用静态互斥锁串行化，避免两次落盘乱序覆盖。
- **流式输出隔离。** 运行期间增量不写入全局 store，而是走独立订阅通道，只有正在输出的气泡订阅它；工具调用、整轮收尾等阶段边界才把文本一次性定稿，因此长篇回答不会重渲染侧栏、面板与顶栏。
- **虚拟滚动。** 只挂载视口内（及邻近）的消息；最后一条气泡增长时跳过高度回测，跟随底部由流式通道直接设置 `scrollTop`，不触发 React 重渲染。
- **增量 Markdown。** 流式期间按代码围栏外的空行切块：已稳定的块只渲染一次并保留 DOM，未闭合围栏只更新 `code` 文本节点；语法高亮在收尾时按代码块逐个补齐，而不是对整条回答一次性执行。

## Rust 测试覆盖

```bash
npm run test:all         # Rust workspace + 前端（在仓库根执行）
npm run build            # 前端：类型检查 + 打包
```

Rust 测试套件包括：
- 补丁应用（简单、多 hunk、插入、缺失上下文报错）
- glob 匹配
- 针对 mock stdio 服务器的 MCP 资源/提示词列出与读取
- MCP 工具自动分发
- MCP streamable-HTTP 握手与工具调用
- MCP 并发安全（并发混合请求、压力突发）
- Agent `execute()` 并发（内置工具、混合 MCP + 内置）
- HTML 转文本与搜索结果解析（联网工具）
- 会话任务清单（`todo_write` / `todo_read`）
- Git 工具（status / diff / commit / log 往返）
- token 用量解析与重试退避计算
- 多模态内容组装（文本 + 图片片段）
- 会话保存/加载往返
- 从 `AGENTS.md` 注入项目规则（及关闭）
- 技能发现（frontmatter、回退命名、工作区覆盖）
- 钩子执行（退出码、工具名匹配、stdin 载荷）
- 子代理任务板顺序与并发闸门
- 审批策略决策与可信命令识别
- Landlock 工作区边界强制
- SIGINT 标志位设置/清除
- 长期记忆（中文二元切分、BM25 排序、去重、提炼门槛）
- 桌面外壳：NDJSON 流解析（增量、工具事件、错误与截断识别）、目录浏览、模型/daemon 地址归一化、云端位置解析、导出文件名清洗、输出截断，以及断言长耗时命令不会同步跑在主线程上的回归用例

测试使用的 mock MCP 服务器位于 `agent/tests/mock_mcp_server.py`。

## 许可证

MIT
