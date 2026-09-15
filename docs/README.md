# Symbio 文档中心

> **文档驱动开发**：本文档体系是 Symbio 项目的"单一事实来源"。

## 文档下沉原则

文档**就近放置、单一来源**：

- **单模块文档**放在该模块目录内（`README.md` + 可选 `docs/`），只写该模块自身机制，不重复系统级内容；
- **系统级文档**（本目录）只保留**跨模块**的核心逻辑、边界与约定，涉及单模块内部实现时**引用**模块文档，不复制细节；
- **历史实施记录**一律进 `archive/`，现行文档只描述当前行为。

示例：会话上下文压缩的 L0-L6 分层总览在 [design/context-compression-design.md](./design/context-compression-design.md)，而各层阈值与代码实现在 [session/README.md](../symbio/src/plugins/session/README.md)。

## 快速导航

| 我想... | 查阅 |
|---------|------|
| 了解系统全貌 | [SYSTEM_MAP.md](./SYSTEM_MAP.md) |
| 理解架构设计 | [OVERVIEW.md](./architecture/OVERVIEW.md) |
| 查看协议规范 | [PROTOCOLS.md](./architecture/PROTOCOLS.md) |
| 追踪请求全链路 | [DATA_FLOW.md](./architecture/DATA_FLOW.md) |
| 查找路由路径 | [ROUTES.md](./reference/ROUTES.md) |
| 理解错误码 | [ERROR_CODES.md](./reference/ERROR_CODES.md) |
| 配置系统 | [CONFIGURATION.md](./reference/CONFIGURATION.md) |
| 快速上手 | [QUICK_START.md](./guides/QUICK_START.md) |
| 开发新插件 | [PLUGIN_DEVELOPMENT.md](./guides/PLUGIN_DEVELOPMENT.md) |
| 排查问题 | [TROUBLESHOOTING.md](./guides/TROUBLESHOOTING.md) |
| 了解设计决策 | [DECISIONS.md](./DECISIONS.md) |
| 看某个插件/前端的职责与机制 | 各模块 `README.md`（见下方模块文档地图） |

## 文档结构

```
docs/                            # 系统级文档（跨模块）
├── README.md                    # 本文档 (入口)
├── SYSTEM_MAP.md                # 系统地图 (一图胜千言)
├── DECISIONS.md                 # 架构决策记录 (ADRs)
├── architecture/                # 架构文档
│   ├── OVERVIEW.md              # 架构总览
│   ├── DATA_FLOW.md             # 数据流与调用链（排障地图）
│   └── PROTOCOLS.md             # 协议规范
├── reference/                   # 参考文档
│   ├── ROUTES.md                # 路由参考
│   ├── ERROR_CODES.md           # 错误码参考
│   └── CONFIGURATION.md         # 配置参考
├── guides/                      # 操作指南
│   ├── QUICK_START.md           # 快速上手
│   ├── PLUGIN_DEVELOPMENT.md    # 插件开发
│   └── TROUBLESHOOTING.md       # 故障排查
├── design/                      # 现行设计规范（只写跨层取舍与不变量）
│   ├── vdfs.md                          # VDFS 机制规范（权威；资源存储见 §11 / §13.4）
│   ├── context-compression-design.md    # 上下文压缩 L0-L6 分层总览
│   ├── http-api-transport.md            # Gateway HTTP/WS 传输层设计
│   └── open-agent-bundle-spec.md        # OAB 包规范
├── CHANGELOG.md                 # 更新日志
└── archive/                     # 历史归档 (仅供参考，含 implementation-logs/；
                                 #  已废止的实体机制档案在此：entity-provider-mechanism.md /
                                 #  entity-management-mechanism.md)

symbio/src/plugins/<plugin>/     # 模块级文档（就近原则）
└── README.md                    # 插件职责、路由、内部机制（14 个插件全覆盖）

tauri/                           # 前端
├── README.md                    # 前端入口
└── docs/FRONTEND.md             # 前端架构与视图清单
```

## 模块文档地图

| 模块 | 文档 | 一句话职责 |
|------|------|-----------|
| session | [plugins/session/README.md](../symbio/src/plugins/session/README.md) | 会话编排唯一入口：工具循环、提示词组装、上下文压缩 |
| model | [plugins/model/README.md](../symbio/src/plugins/model/README.md) | 无状态单轮 LLM 网关（execute_turn），多协议适配 |
| agent | [plugins/agent/README.md](../symbio/src/plugins/agent/README.md) | 人格/智能体资产：经 traverse 贡献工具与人格 |
| mcp | [plugins/mcp/README.md](../symbio/src/plugins/mcp/README.md) | MCP 外部工具接入与能力注册 |
| skill | [plugins/skill/README.md](../symbio/src/plugins/skill/README.md) | 技能脚本（loader/plugin）发现与装载 |
| local | [plugins/local/README.md](../symbio/src/plugins/local/README.md) | 本地文件系统工具 + system 提示词下发 |
| web | [plugins/web/README.md](../symbio/src/plugins/web/README.md) | 网页抓取/搜索工具 |
| home | [plugins/home/README.md](../symbio/src/plugins/home/README.md) | 根插件：持应用级状态（`<homedir>/PLUGIN.yml`），构造 worker(Composite) 并传入必需插件清单 |
| composite | [plugins/composite/README.md](../symbio/src/plugins/composite/README.md) | 子插件容器：扫描 `plugins/` 目录 + 路径合并分发 |
| gateway | [plugins/gateway/README.md](../symbio/src/plugins/gateway/README.md) | HTTP/WS 入站网关 |
| setting | [plugins/setting/README.md](../symbio/src/plugins/setting/README.md) | 运行时设置 |
| hook | [plugins/hook/README.md](../symbio/src/plugins/hook/README.md) | 生命周期钩子 |
| event_bus | [plugins/event_bus/README.md](../symbio/src/plugins/event_bus/README.md) | 进程内事件总线 |
| telegram | [plugins/telegram/README.md](../symbio/src/plugins/telegram/README.md) | Telegram 通道接入 |
| tauri 前端 | [tauri/README.md](../tauri/README.md) → [docs/FRONTEND.md](../tauri/docs/FRONTEND.md) | Vue3 + Tauri2 桌面前端 |

## 文档原则

1. **真实**：文档必须与代码同步，过时文档比无文档更危险
2. **精简**：每个文档聚焦一个主题，避免重复
3. **准确**：代码示例必须可执行，接口签名必须与代码一致
4. **唯一来源**：每个知识点只有一个权威文档（模块机制以模块内文档为准）

## 维护规则

| 变更类型 | 需要更新的文档 |
|----------|----------------|
| 新增路由 | `ROUTES.md`（系统级总表）+ 对应模块 `README.md` 路由段 |
| 新增错误码 | `ERROR_CODES.md` |
| 新增配置项 | `CONFIGURATION.md` |
| 模块内部机制变更 | 该模块 `README.md`（系统级文档不复制细节） |
| 跨模块架构变更 | `OVERVIEW.md` + `DECISIONS.md` + `SYSTEM_MAP.md` |
| 协议变更 | `PROTOCOLS.md` |
| 历史实施记录 | 只进 `archive/`，现行文档不保留过程日志 |
| 常见问题 | `TROUBLESHOOTING.md` |

## 阅读路径

### 新手入门
1. [QUICK_START.md](./guides/QUICK_START.md) - 跑通第一个场景
2. [SYSTEM_MAP.md](./SYSTEM_MAP.md) - 了解系统全貌
3. [OVERVIEW.md](./architecture/OVERVIEW.md) - 理解设计哲学

### 开发者
1. [PROTOCOLS.md](./architecture/PROTOCOLS.md) - 理解核心协议
2. [ROUTES.md](./reference/ROUTES.md) - 查找可用路由
3. 目标模块 `README.md` - 理解模块内部机制
4. [PLUGIN_DEVELOPMENT.md](./guides/PLUGIN_DEVELOPMENT.md) - 开发新插件

### 维护者
1. [DECISIONS.md](./DECISIONS.md) - 理解历史决策
2. [ERROR_CODES.md](./reference/ERROR_CODES.md) - 处理错误
3. [TROUBLESHOOTING.md](./guides/TROUBLESHOOTING.md) - 排查问题

---

> **维护原则**：本文档是文档体系的入口，任何结构性变更必须先更新此文件。
