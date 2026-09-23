# VDFS 插件

**文件系统本身**——对前端、对大模型的统一资源与数据访问层（Virtual Dynamic File System）。
本插件不含任何资源语义、不持有拓扑：不认识会话/模型/设置，不认识挂载点，
只认识「根 provider + 全路径」。虚拟根归组合容器所有（见 `../composite/README.md`）。

机制规范（权威、跨模块）在 [`docs/design/vdfs.md`](../../../../docs/design/vdfs.md)；
本文件只写插件自身的落位。

## 一棵树的两半

判别只有一条：**规范化后的地址是否以 `.vdfsv2` 打头**。除此之外没有任何分流规则。

| 层 | 地址形态 | 落点 |
|----|----------|------|
| 虚拟层 | `.vdfsv2` · `.vdfsv2/<子目录>/…` | 容器注册的各 provider 组合视图 |
| 物理层 | `README.md`（工作目录相对） · `D:/tmp/a.txt`（绝对） | 磁盘真实文件（`physical.rs`） |

`.vdfsv2` 下有多少子目录、叫什么，由容器与各 provider 的注册决定——新增一类资源
不需要改动本插件任何一行。

## 两条链路，同一批 provider

前端与 LLM 消费**同一批**注册的挂载 provider，因此不存在「前端支持而 LLM 不支持」的资源；
差异只在呈现——前端拿域响应原样渲染，工具在 `execute` 内封装。工具集是操作的**子集**
（`watch` / `unwatch` / `action` 不经 LLM 工具暴露：实时订阅归宿主前端，动作由详情
`actions` 声明后按需触发）。

**操作与工具清单以 [docs/CURRENT.md](../../../../docs/CURRENT.md) §3 为准**（操作闭集
`protocol.rs::VDFS_OPS` 有测试锁计数；工具集完整性由 `tools/mod.rs` 的
`tools_cover_all_ops` 测试锁死）——本 README 不抄那份会漂移的清单。

## 模块分工

| 文件 | 职责 |
|------|------|
| `fs.rs` | `UnifiedFs`——**唯一的地址翻译点**，`.vdfsv2`/物理分流 |
| `host.rs` | 访问层：取根 / 翻译操作 / 树遍历 / 事件投递 |
| `physical.rs` | 物理文件层（磁盘读写 + 路径守卫） |
| `provider.rs` | `ToolVdfs` 封装 provider（工具链路） |
| `protocol.rs` | 线路信封（`vdfs/*` 请求响应 + `VDFS_OPS`） |
| `plugin.rs` | 插件装配（`route` 分发 + `traverse` 注册工具） |
| `tools/` | 10 个工具，一操作一文件（见上表） |

## 关键不变量

- **能力判据只认访问位**（`r`/`w`/`l`/`t`）；`is_dir()` = `access.list`。
- **变更投递不经本插件**：provider 回调由访问层接到事件总线
  （`host::event_bus_sink`），少一跳、少一个长驻任务。
- **隐藏是机制级**：`VdfsNode.hidden` —— 列表不出现、可达性不变。
- **路径守卫**：物理层黑名单 + 解析符号链接后复验（防链接逃逸）；写路径拒绝符号链接。
  Windows `canonicalize` 的 `\\?\` 前缀在比较前剥掉（`normalize_for_comparison`）。

## 关联

- 机制规范：[`docs/design/vdfs.md`](../../../../docs/design/vdfs.md)
- 前端页面规范：[`docs/design/vdfs-frontend.md`](../../../../docs/design/vdfs-frontend.md)
- 纯接口（trait + 域类型）：`symbio_core::vdfs_provider`
- symbio 桥（上下文注入 / 错误翻译）：`symbio_core::vdfs::host`
- 虚拟层拓扑（容器聚合 + 注册为根）：`../composite/README.md`
