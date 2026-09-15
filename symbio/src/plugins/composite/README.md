# Composite 插件

动态容器：按配置实例化任意子插件，是"分形插件架构"的关键。

## 机制

- **配置驱动**：从全局配置（home 的 config.yaml）读取子插件清单，逐个经 `ObjectCreatorRegistry`（`create_object`）实例化并挂载。
- **路由转发**：`route()` 内按 `PATH` 剥离当前层级前缀，转发给对应子插件；本地指令（如查询自身拓扑）就地处理。
- **traverse 聚合**：`traverse()` 深度优先聚合全子树的能力贡献（工具 / 人格 / VDFS 资源注册项），供 session、home 等消费。
- **VDFS 拓扑唯一持有者**：逐子插件广播收集各自的 `impl VdfsProvider`，组合成「包含子目录列表的 provider」（`CompositeVdfs`）并登记进 `register_vdfs_root` 槽位，成为 `.vdfs` 的服务者——它没有任何「根级别」概念（见 `docs/design/vdfs.md` §2.5 / §13.2）。
- **对称性**：容器与叶子插件实现同一 `Plugin` Trait，接口完全一致。

## 关联

- 架构哲学：`docs/architecture/OVERVIEW.md`
- 注册机制：`symbio_core/creator.rs`（`submit_object_creator!`）
