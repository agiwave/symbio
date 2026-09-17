# Symbio 系统地图

> **文档类型：导航** — 一图胜千言，快速定位系统全貌。
> 本文件只画**结构与分层**；插件 × 挂载点 × 路由 × 工具的权威清单见 [CURRENT.md](./CURRENT.md)，文档索引见 [docs/README.md](./README.md#快速导航)。

## 系统边界

```mermaid
graph TD
    subgraph HOST["Host（宿主层）"]
        T["Tauri 桌面<br/>Vue 3 + IPC"]
        C["CLI"]
        G["Gateway 插件<br/>HTTP / WS 入站"]
    end

    subgraph CORE["Core Library（symbio/）"]
        REG["ObjectCreatorRegistry<br/>submit_object_creator! 静态注册"]
        subgraph TREE["Plugin Tree（运行时）"]
            HOME["Home /"]
            W["worker（Composite）<br/>扫描自身目录，每目录一个 PLUGIN.yml"]
            HOME --> W
            W --> GROUP["agent · session · model · local · web · vdfs · skill<br/>mcp · work · telegram · gateway · setting · hook · event_bus"]
            HOME --> HW["home/* · work/*（Home 自身终结）"]
        end
        subgraph PROV["Providers（基础设施）"]
            EM["Embedding（fastembed）"]
            VS["vdfs_service（单文件 / 目录 / 内存）"]
            SS["SessionStore（磁盘布局 + 进程内驻留）"]
        end
        REG --> HOME
    end

    T --> REG
    C --> REG
    G --> REG
```

> 插件清单（16 个）、各插件注册名 / VDFS 挂载点 / 自有路由 / 配置文件，见 [CURRENT.md](./CURRENT.md) §1。

## 请求流转（以 AI 对话为例）

```mermaid
flowchart LR
    CL["Client<br/>Tauri / CLI / HTTP"] -->|"route session/chat/send"| HM["Home<br/>路由"]
    HM --> CP["Composite<br/>剥离首段后转发"]
    CP --> SE["Session<br/>编排 · 工具循环"]
    SE -->|"execute_turn（直连，不经路由）"| MD["Model<br/>多协议适配"]
    MD --> LLM["LLM API"]
    SE -.->|"traverse 收集全树工具"| TOOLS["local / web / vdfs / mcp …"]
```

> 完整链路、流式帧与代码位置见 [DATA_FLOW.md](./architecture/DATA_FLOW.md)。

## 协议栈

| 层级 | 类型 | 用途 |
|------|------|------|
| **帧** | `PluginFrame` | 通道最小消息单位 (Data / Error) |
| **载荷** | `PluginPayload` | route() 返回 (Empty/Data/Native/Session) |
| **通道** | `PluginChannel` | 全双工流式会话 (mpsc pair) |
| **路由** | `InvokeRequest` | 上下文注入 (PATH / PAYLOAD / WORKDIR ...) |

> 定义与不变量见 [PROTOCOLS.md](./architecture/PROTOCOLS.md)。

---

> **维护原则**：本文档是系统的"地图"，与代码同步；架构变更时必须同步更新此图。
