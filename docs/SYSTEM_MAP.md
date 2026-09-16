# Symbio 系统地图

> **文档类型：导航** — 一图胜千言，快速定位系统全貌。

## 系统边界

```
┌─────────────────────────────────────────────────────────────────────────────┐
│                            Host (宿主层)                                     │
│  ┌─────────────┐  ┌──────────────────┐  ┌─────────────────────────────┐    │
│  │  Tauri 桌面  │  │  CLI (seed_agents)│  │  Gateway 插件 (已实现)          │    │
│  │  Vue 3 + IPC │  │  批量灌入 Agent    │  │  HTTP/WebSocket Gateway     │    │
│  └──────┬───────┘  └────────┬─────────┘  └─────────────┬───────────────┘    │
│         │                   │                          │                    │
│         └───────────────────┴──────────────────────────┘                    │
│                             │                                               │
│                    route() / traverse()                                     │
└─────────────────────────────┼───────────────────────────────────────────────┘
                              ▼
┌─────────────────────────────────────────────────────────────────────────────┐
│                        Core Library (symbio/)                               │
│                                                                             │
│  ┌──────────────────────────────────────────────────────────────────────┐  │
│  │                        ObjectCreatorRegistry                          │  │
│  │              (inventory 静态注册 · submit_object_creator!)            │  │
│  └──────────────────────────────────────────────────────────────────────┘  │
│                              │                                              │
│                              ▼                                              │
│  ┌──────────────────────────────────────────────────────────────────────┐  │
│  │                     Plugin Tree (运行时)                             │  │
│  │                                                                      │  │
│  │   Home (/)                                                           │  │
│  │   └── worker (Composite) ─── 扫描 plugins/ 目录（每目录一个 PLUGIN.yml）动态挂载 │  │
│  │       ├── agent    ─── OAB Bundle 宿主 · 身份工具 · MCP 声明         │  │
│  │       ├── session  ─── 会话编排唯一入口 · 工具循环 · 压缩 · 直连 model   │  │
│  │       ├── model    ─── 无状态单轮 LLM 网关 · 多协议适配 │  │
│  │       ├── local    ─── shell / file_read|write|edit / glob / search  │  │
│  │       ├── web      ─── http_request / web_search / web_fetch         │  │
│  │       ├── vdfs     ─── 文件系统本身 · vdfs/* 协议 + vdfs_* 工具      │  │
│  │       ├── skill    ─── SKILL.md 加载与执行                           │  │
│  │       ├── mcp      ─── MCP Server 注册 (stdio/http) · 工具桥接       │  │
│  │       ├── telegram ─── 消息收发通道                                  │  │
│  │       ├── gateway  ─── HTTP/WS 入站网关 (route_v2 同构)              │  │
│  │       ├── setting  ─── 设置清单：各插件配置条目 + 自有分区            │  │
│  │       ├── hook     ─── 钩子注册与触发                                │  │
│  │       └── event_bus─── 进程内帧广播 (SSE 风格)                       │  │
│  │   Home 自身终结: home/* · work/*                                     │  │
│  └──────────────────────────────────────────────────────────────────────┘  │
│                                                                             │
│  ┌──────────────────────────────────────────────────────────────────────┐  │
│  │                    Providers (基础设施)                                │  │
│  │   ├── Embedding (fastembed)     ─── 向量嵌入                         │  │
│  │   ├── vdfs_service              ─── 资源存储 = 单文件/目录/内存      │  │
│  │   └── (插件内 SessionStore)     ─── 一种磁盘布局 + 进程内驻留       │  │
│  └──────────────────────────────────────────────────────────────────────┘  │
└─────────────────────────────────────────────────────────────────────────────┘
```

## 请求流转 (以 AI 对话为例)

```
Client (Tauri/CLI)
    │
    │  route("session/chat/send", {messages, agent_id})
    ▼
┌─────────┐    ┌───────────┐    ┌─────────┐    ┌─────────┐
│   Home  │───►│ Composite │───►│ Session │───►│  Model  │
│  (路由)  │    │  (转发)    │    │ (编排)   │    │ (推理)  │
└─────────┘    └───────────┘    └────┬────┘    └────┬────┘
                                     │              │
                                     │ traverse     │ HTTP/SSE
                                     │ (收集工具)    ▼
                                     │         ┌─────────┐
                                     ▼         │  LLM    │
                              ┌──────────┐     │  API    │
                              │ 全树插件  │     └─────────┘
                              │ (local,  │
                              │  web,    │
                              │  mcp...) │
                              └──────────┘
```

## 协议栈

| 层级 | 类型 | 用途 |
|------|------|------|
| **帧** | `PluginFrame` | 通道最小消息单位 (Data / Error) |
| **载荷** | `PluginPayload` | route() 返回 (Empty/Data/Native/Session) |
| **通道** | `PluginChannel` | 全双工流式会话 (mpsc pair) |
| **路由** | `InvokeRequest` | 上下文注入 (PATH / PAYLOAD / WORKDIR ...) |

## 快速定位

| 想了解... | 查阅文档 |
|-----------|----------|
| 如何开始 | [QUICK_START.md](./guides/QUICK_START.md) |
| 架构为什么这样设计 | [OVERVIEW.md](./architecture/OVERVIEW.md) |
| 请求/流式全链路与代码位置 | [DATA_FLOW.md](./architecture/DATA_FLOW.md) |
| 帧/载荷/通道协议 | [PROTOCOLS.md](./architecture/PROTOCOLS.md) |
| 所有可用路由 | [ROUTES.md](./reference/ROUTES.md) |
| 错误码含义 | [ERROR_CODES.md](./reference/ERROR_CODES.md) |
| 配置文件结构 | [CONFIGURATION.md](./reference/CONFIGURATION.md) |
| 开发新插件 | [PLUGIN_DEVELOPMENT.md](./guides/PLUGIN_DEVELOPMENT.md) |
| 排查问题 | [TROUBLESHOOTING.md](./guides/TROUBLESHOOTING.md) |
| 架构决策原因 | [DECISIONS.md](./DECISIONS.md) |
| 某个插件/前端的职责与内部机制 | 各模块 `README.md`（见 [模块文档地图](./README.md#模块文档地图)） |

---

> **维护原则**：本文档是系统的"地图"，必须与代码同步更新。任何架构变更必须先更新此图。
