# Symbio 文档中心

> **文档驱动开发**：本文档体系是 Symbio 项目的"单一事实来源"。

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
| 会话上下文压缩/输出长度治理 | [design/context-compression-design.md](./design/context-compression-design.md) |

## 文档结构

```
docs/
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
├── CHANGELOG.md                 # 更新日志
├── design/                      # 现行设计规范（上下文压缩 / 实体管理机制 / OAB / 前端 PRD）
├── branding/                    # 品牌资产
└── archive/                     # 历史归档 (仅供参考)
```

## 文档原则

1. **真实**：文档必须与代码同步，过时文档比无文档更危险
2. **精简**：每个文档聚焦一个主题，避免重复
3. **准确**：代码示例必须可执行，接口签名必须与代码一致
4. **唯一来源**：每个知识点只有一个权威文档

## 维护规则

| 变更类型 | 需要更新的文档 |
|----------|----------------|
| 新增路由 | `ROUTES.md` |
| 新增错误码 | `ERROR_CODES.md` |
| 新增配置项 | `CONFIGURATION.md` |
| 架构变更 | `OVERVIEW.md` + `DECISIONS.md` + `SYSTEM_MAP.md` |
| 协议变更 | `PROTOCOLS.md` |
| 常见问题 | `TROUBLESHOOTING.md` |

## 阅读路径

### 新手入门
1. [QUICK_START.md](./guides/QUICK_START.md) - 跑通第一个场景
2. [SYSTEM_MAP.md](./SYSTEM_MAP.md) - 了解系统全貌
3. [OVERVIEW.md](./architecture/OVERVIEW.md) - 理解设计哲学

### 开发者
1. [PROTOCOLS.md](./architecture/PROTOCOLS.md) - 理解核心协议
2. [ROUTES.md](./reference/ROUTES.md) - 查找可用路由
3. [PLUGIN_DEVELOPMENT.md](./guides/PLUGIN_DEVELOPMENT.md) - 开发新插件

### 维护者
1. [DECISIONS.md](./DECISIONS.md) - 理解历史决策
2. [ERROR_CODES.md](./reference/ERROR_CODES.md) - 处理错误
3. [TROUBLESHOOTING.md](./guides/TROUBLESHOOTING.md) - 排查问题

---

> **维护原则**：本文档是文档体系的入口，任何结构性变更必须先更新此文件。

