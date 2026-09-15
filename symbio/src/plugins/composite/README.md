# Composite 插件

通用容器：按**插件目录**实例化子插件，是"分形插件架构"的关键。它不内置任何插件
清单，也可以嵌套另一个容器。

## 机制

- **目录驱动**：扫描**自己目录下的 `plugins/`**（父插件经 ctx 键 `PLUGIN_DIR` 告知
  它的目录），每个子目录经 `ObjectCreatorRegistry`（`create_object`）实例化并挂载。
  加载判据 = 目录下的 `PLUGIN.yml` 可解析且 `plugin_provider` 指向已注册的工厂。
  「必需插件」清单由**构造者**经 ctx 键 `REQUIRED_PLUGINS` 传入（`home` 传的是
  `SYSTEM_PLUGINS`），容器只负责把缺失的目录 / 配置文件补出来。
- **配置归插件**：构造子插件时把**它的目录**经 ctx 键 `PLUGIN_DIR` 告知它，插件据此
  自己读写 `PLUGIN.yml`；容器不碰任何子插件的配置（见 `docs/design/vdfs.md` §3.4）。
- **路由转发**：`route()` 内按 `PATH` 剥离当前层级前缀，转发给对应子插件；本地指令（如查询自身拓扑）就地处理。
- **traverse 聚合**：`traverse()` 深度优先聚合全子树的能力贡献（工具 / 人格 / VDFS 资源注册项），供 session、home 等消费。
- **VDFS 拓扑唯一持有者**：逐子插件广播收集各自的 `impl VdfsProvider`，组合成「包含子目录列表的 provider」（`CompositeVdfs`）并登记进 `register_vdfs_root` 槽位，成为 `.vdfs` 的服务者——它没有任何「根级别」概念（见 `docs/design/vdfs.md` §2.5 / §13.2）。
- **对称性**：容器与叶子插件实现同一 `Plugin` Trait，接口完全一致。

## 关联

- 架构哲学：`docs/architecture/OVERVIEW.md`
- 注册机制：`symbio_core/creator.rs`（`submit_object_creator!`）
