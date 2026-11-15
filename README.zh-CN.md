# Daily

[![CI](https://github.com/JinXingJX/daily/actions/workflows/ci.yml/badge.svg)](https://github.com/JinXingJX/daily/actions/workflows/ci.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)

[English](README.md)

Daily 是一个本地桌面工作台，用来让 AI 智能体在你电脑上的某个文件夹里工作。界面基于 Electron + React，智能体循环、模型提供商、持久化和沙箱插件系统由一个 Rust 运行时负责。

<!-- 截图：添加 docs/images/workbench.png 后在此引用。 -->

## 功能

- **持久化的任务**：每个工作区把对话、轮次、工具调用和记忆保存在 `<workspace>/.daily/` 下，多个会话可以同时运行。
- **可追溯的运行过程**：每一次模型请求、工具调用和结果都记录为可展开的步骤，并显示每轮用掉的 token；失败或被停止的轮次可以从断点继续。
- **自带模型**：支持 OpenAI 兼容接口（OpenAI、本地服务、网关）、Anthropic 和 Gemini 的 API Key，均使用原生工具调用和流式输出；另有可选的非官方适配器，复用本机 Codex CLI 或 Antigravity 的登录。
- **智能体与钩子**：为智能体单独配置模型、提示词、上下文目录和插件；用 `@智能体名` 指定由谁处理，并在每轮开始前或完成后自动运行其他智能体。
- **工作区工具**：智能体可以列出、读取和搜索工作区文件（只读，私有的 `.daily` 目录对其不可见），同一步里的多次读取会并行执行。
- **独立的多智能体 SDK**：[`daily-agent-core`](crates/daily-agent-core) 提供运行管理、串行/并行/DAG 编排、递归代理、共享预算和版本化检查点；[`daily-agent-providers`](crates/daily-agent-providers) 提供模型协议。运行 `cargo run -p daily-agent-core --example framework` 可离线体验。桌面端保留 `daily_core::sdk` 兼容入口。
- **插件**：插件可以提供沙箱视图（主视图或后台视图）、内联渲染器和智能体工具，并通过类型化 SDK 使用宿主能力：存储、工作区文件、网络请求、模型调用，以及可在工具处暂停与恢复的结构化智能体任务。
- **中英文界面**。

## 状态

Daily 仍处于早期阶段。本地验证使用 Apple Silicon macOS，CI 覆盖 Linux、macOS 和 Windows；构建产物未签名，也没有自动更新。1.0 之前磁盘数据格式可能发生不兼容的变化。

## 快速开始

环境要求：

- Windows 10/11 x64，或 Apple Silicon 上的 macOS 14 及以上
- Node.js 22.12 及以上（仓库包含 `.nvmrc`）
- 当前稳定版 Rust 工具链（[rustup](https://rustup.rs)）
- Windows：Visual Studio Build Tools，安装“使用 C++ 的桌面开发”和 Windows SDK
- macOS：Xcode 命令行工具（`xcode-select --install`）

```bash
git clone https://github.com/JinXingJX/daily.git
cd daily
npm ci
npm run dev
```

`npm run dev` 会以调试模式构建 Rust sidecar，并启动带热更新的 Electron。首次启动时会让你选择一个工作区目录，之后会自动恢复上次打开的窗口。

App、Core、Writer、Timeline 的完整安装步骤见 [Windows 使用指南](docs/windows.md)。构建两个插件时，主应用的本地目录需命名为 `daliy`。

## 配置模型

打开 **设置 → 模型提供商**。

### API Key 提供商

点击 **添加 API 提供商**，选择 OpenAI 兼容、Anthropic 或 Gemini。填写模型 ID（例如 `gpt-4.1`、`claude-sonnet-4-5` 或 `gemini-2.5-pro`），然后粘贴 API Key，或者留空以读取环境变量：

| 提供商 | 默认地址 | 环境变量 |
| --- | --- | --- |
| OpenAI 兼容 | `https://api.openai.com/v1` | `OPENAI_API_KEY` |
| Anthropic | `https://api.anthropic.com/v1` | `ANTHROPIC_API_KEY` |
| Gemini | `https://generativelanguage.googleapis.com/v1beta` | `GEMINI_API_KEY`、`GOOGLE_API_KEY` |

修改 Base URL 即可接入兼容服务，例如 Ollama（`http://localhost:11434/v1`）或各类网关。在界面中填写的 Key 保存在 `~/.daily/settings.json`，不在任何工作区内。文件格式见 [docs/settings.md](docs/settings.md)。

### Codex 与 Antigravity（非官方）

Daily 也可以复用本机已安装的 Codex CLI 或 Antigravity 2.x 客户端的登录。**这两个适配器为非官方实现，与 OpenAI 或 Google 无关，可能违反相应服务的条款，并可能随时失效。** 启用前请先阅读 [docs/subscription-adapters.md](docs/subscription-adapters.md)；Antigravity 适配器还需要自行提供 Daily 不附带的 OAuth client secret。

## 工作区与数据

| 路径 | 内容 |
| --- | --- |
| `<workspace>/.daily/workspace.json` | 任务树与智能体 |
| `<workspace>/.daily/sessions/` | 每个会话一个文件 |
| `<workspace>/.daily/plugins.json` | 已安装和已启用的插件 |
| `<workspace>/.daily/settings.json` | 项目设置（可共享） |
| `<workspace>/.daily/settings.local.json` | 本地设置（不要提交） |
| `~/.daily/settings.json` | 用户设置，包括提供商密钥 |

除非有意共享，请把 `.daily/` 加入工作区的 `.gitignore`。设置优先级为 **本地 > 项目 > 用户**；提供商的地址与密钥只从用户和本地设置读取，共享仓库无法把你的凭据转发到别处。

## 插件

在 **设置 → 插件 → 安装本地插件** 中填写插件目录（或其 `.daily-plugin/plugin.json`）的绝对路径即可安装；如需热更新，可再填写一个 `localhost` 开发服务器地址。

已启用的插件可以使用宿主的全部 API，**请只安装你信任的插件**。开发插件请从 [docs/plugin-development.md](docs/plugin-development.md)、[`packages/plugin-sdk`](packages/plugin-sdk) 中的类型化 SDK 和 [`examples/local-writer-plugin`](examples/local-writer-plugin) 中的最小示例开始。

## 架构

```text
React 渲染进程 ──contextBridge──▶ Electron 主进程 ──stdio 上的 JSONL──▶ daily-app-server
可信插件 ES 模块 ──直接宿主 API──┘                                          │
                                                                          ▼
                                  daily-desktop-runtime：桌面插件、设置、会话、持久化
                                      ├── daily-agent-core：agent SDK
                                      └── daily-agent-providers：模型通信 → agent SDK
```

每个工作区窗口对应一个独立的 sidecar 进程。详见 [docs/architecture.md](docs/architecture.md) 与 [docs/protocol.md](docs/protocol.md)。

## 开发

| 命令 | 作用 |
| --- | --- |
| `npm run dev` | 构建调试版 sidecar 并启动带热更新的 Electron |
| `npm run dev:web` | 只运行渲染进程（桌面命令不可用） |
| `npm test` | Rust 测试、协议检查、渲染进程与 Electron 测试、生产构建 |
| `npm run typecheck` | 检查渲染进程、主进程和 preload 的类型 |
| `npm run gen:protocol` | 从 Rust 重新生成 TypeScript 协议类型、IPC 白名单和 SDK 方法列表 |
| `npm run package:agent-sdk` | 验证并打包独立 Rust SDK 源码 |
| `cargo test --workspace` | 只运行 Rust 测试 |
| `cargo clippy --workspace --all-targets -- -D warnings` | 严格的 Rust 静态检查 |
| `npm run package:win` | 在 `release/` 中构建未签名的 Windows x64 NSIS 安装包 |
| `npm run package:mac` | 在 `release/` 中构建未签名的 Apple Silicon ZIP 和 DMG |

做自动化界面检查时，开发构建支持 `DAILY_WORKSPACE=/absolute/path`（直接打开该工作区）和 `DAILY_CAPTURE_PATH=/tmp/shot.png`（加载后保存截图）。

提交 Pull Request 前请阅读 [CONTRIBUTING.md](CONTRIBUTING.md)。

## 已知限制

- 构建产物未签名。Windows CI 验证原生构建和 sidecar 行为，交互界面仍需 Windows 实机验收。
- Antigravity 登录依赖 macOS 钥匙串，Windows 请使用 API Key 提供商。
- 智能体系统提示词和内置模型提示词为英文。
- 插件市场目前只是静态示例列表。
- 每个工作区最多同时运行 4 个会话。
- 内置的智能体工具只能读取工作区。没有 MCP 客户端，也没有内置的写文件或执行命令的工具；插件可以提供自己的工具。
- 历史和工具结果的 token 预算是估算值，没有使用各服务商的分词器精确计数。

## 免责声明

Daily 按 MIT 许可证以“现状”提供。你需要自行负责所连接模型提供商的使用方式，并遵守其服务条款。Codex 与 Antigravity 适配器为非官方实现，不受相应厂商支持。

## 许可证

[MIT](LICENSE) © 2026 JinXingJX
