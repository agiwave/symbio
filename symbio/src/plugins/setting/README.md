# Setting 插件

`.vdfs/setting` 挂载点的实现者：承载**前端状态自持**的设置分区（`appearance` /
`about`），外加**各插件配置文档的清单**。

## 路由

**无**。本插件是无状态 provider，`route` 恒 `NotFound`。

## 清单 = 各插件交出来的配置条目 + 自有分区

配置的读写**不归本插件**：各插件的配置归各插件自己（`.vdfs/<插件>/PLUGIN.yml`）。
本插件只把它们的条目**列出来**，顺序是插件配置在前、自有分区垫后（前者才是用户在
设置页里要动手的东西）：

- 条目由各插件在 `traverse` 里经 `announce_configurable` 声明，容器用**共享**收集器
  收下并写回请求 ctx（见 `symbio_core::configurable`），本插件的 `list` 读出来即可
  ——**不反查插件目录、不硬编码插件表**；
- 条目自带**真实地址**（`<插件目录>/PLUGIN.yml`）与呈现定义，所以点开就是那个插件的
  配置表单，读写照旧落在它自己的文件上：**同一份配置只有一个地址**；
- 本插件只补一个场景标签（`kind = setting`）——列表在哪儿，场景就是哪儿。

## 自有分区（`appearance` / `about`）

- 清单在 `SETTING_SECTIONS` 中登记（`id` / `label`），由 `impl VdfsProvider` 的
  `list` 产出。
- 它们的数据由**前端 store 自持**（主题、版本信息等不在后端），因此这两个分区在
  VDFS 上是**只读节点**（`access = r`，`ext` 即分区 id）：`vdfs/read` /
  `vdfs/write` 对它们恒 `Forbidden`，前端按 `ext` 直接渲染专属面板
  （`registry/vdfsTypes.ts` 的 `appearance` / `about` 渲染器）。
- 原 `setting/config/get` / `setting/config/set`（以及更早的 `setting/list` /
  `setting/get`）已全部下线：分区取值由 VDFS 承担，插件配置由各插件自己的
  配置文件承担。

## 关联

- 插件配置落盘：各插件自己的 `PLUGIN.yml`（见 `symbio_core::plugin_dir`）
- 可配置声明通道：`symbio_core::configurable`（`ConfigurableVisitor` / `CONFIG_VISITOR`）
- VDFS 机制：`docs/design/vdfs.md`（`DetailDefinition` 是 `ext = form` 节点的
  `schema` 方言；配置文件地址 = `<插件目录>/PLUGIN.yml`，见 §3.4 / §13.1）
