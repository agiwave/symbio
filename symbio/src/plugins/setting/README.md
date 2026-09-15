# Setting 插件

`.vdfs/setting` 挂载点的实现者：只承载**前端状态自持**的设置分区
（`appearance` / `about`）。插件配置不在这里——各插件的配置归各插件自己
（如 `.vdfs/session/配置`、`.vdfs/local/配置`）。

## 路由

**无**。本插件是无状态 provider，`route` 恒 `NotFound`。

## 机制

- 分区清单在 `SETTING_SECTIONS` 中登记（`id` / `label` / `description`），
  由 `impl VdfsProvider` 的 `list` 直接产出。
- `appearance` / `about` 的数据由**前端 store 自持**（主题、版本信息等不在
  后端），因此这两个分区在 VDFS 上是**只读节点**（`access = r`，`ext` 即分区 id）：
  `vdfs/read` / `vdfs/write` 对它们恒 `Forbidden`，前端按 `ext` 直接渲染
  专属面板（`registry/vdfsTypes.ts` 的 `appearance` / `about` 渲染器）。
- 原 `setting/config/get` / `setting/config/set`（以及更早的 `setting/list` /
  `setting/get`）已全部下线：分区取值由 VDFS 承担，插件配置由各插件自己的
  配置文档承担。

## 关联

- 全局配置落盘：`../home/README.md`（`save_config` 切片合并）
- VDFS 机制：`docs/design/vdfs.md`（`DetailDefinition` 是 `ext = form` 节点的
  `schema` 方言；配置文档地址 = `<挂载根>/配置`）
