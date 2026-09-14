# Setting 插件

系统级配置读写插件，同时是 `.vdfs/setting` 这一 VDFS 挂载点的实现者。

## 路由

| Path | 说明 |
|------|------|
| `setting/config/get` | 读取整体配置（扁平 Value） |
| `setting/config/set` | 写入整体配置 |

> `setting/list` / `setting/get` 已下线：设置分区的清单与取值改由 VDFS 承担
> （`.vdfs/setting` 的 `vdfs/list` / `vdfs/read`）。

## 机制

- 配置持久化在主目录（与 home 的 config.yaml 体系协同，详见 `../home/README.md`）。
- 分区清单在 `SETTING_SECTIONS` 中登记，各分区的表单定义由 `section_definition()`
  下发，随 VDFS 节点的 `schema` 一起到达前端（前端按 `ext = form` 渲染）。
- 各分区的保存由前端经对应插件的 `config/set` 完成（`config` 绑定的
  `load_path` / `save_path`）。

## 关联

- 全局配置：`../home/README.md`
- VDFS 机制：`docs/design/vdfs.md`
- 实体机制（内部实现，无对外地址）：`docs/design/entity-management-mechanism.md`
