# Home 插件

根容器：插件树顶端，持有全局配置并仅挂载一个子插件 `worker`（Composite）。

## 职责

- **全局配置**：读取/维护 `<homedir>/config.yaml`（插件树拓扑、各插件参数的唯一事实源）。
- **拓扑挂载**：按配置实例化 worker (Composite)，由 Composite 按需挂载其余 13 个插件。
- **自身终结的路由**：`home/*`、`work/*`、`save_config`。

## 路由

| Path | 说明 |
|------|------|
| `home/get_homedir` | 当前系统目录（homedir）与 bootstrap 路径 |
| `home/reload` | 热重载：切换 homedir（可选）+ 重建全部子插件 |
| `work/*` | 工作区级指令（`set_workspace` / `get_workspace`） |
| `save_config` | 收下写配置插件推来的**配置切片**（`{plugin, config}`）并合并落盘 |

> 资源类别清单不设终结路由：`.vdfs` 自身的 `vdfs/list` 就是子目录清单
> （由组合容器逐子插件收集，见 `../composite/README.md` 与 `docs/design/vdfs.md` §2.5）。

## 关联

- 动态容器机制：`../composite/README.md`
- 架构层级图：`docs/architecture/OVERVIEW.md`
