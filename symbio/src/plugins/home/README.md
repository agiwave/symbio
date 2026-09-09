# Home 插件

根容器：插件树顶端，持有全局配置并仅挂载一个子插件 `worker`（Composite）。

## 职责

- **全局配置**：读取/维护 `<homedir>/config.yaml`（插件树拓扑、各插件参数的唯一事实源）。
- **拓扑挂载**：按配置实例化 worker (Composite)，由 Composite 按需挂载其余 13 个插件。
- **自身终结的路由**：`home/*`、`work/*`、`entities/providers`、`save_config`。

## 路由

| Path | 说明 |
|------|------|
| `home/status` | 根节点状态 |
| `work/*` | 工作区级指令转发 |
| `entities/providers` | Provider 实体列表（聚合自 model） |
| `save_config` | 持久化全局配置 |

## 关联

- 动态容器机制：`../composite/README.md`
- 架构层级图：`docs/architecture/OVERVIEW.md`
