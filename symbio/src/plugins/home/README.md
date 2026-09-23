# Home 插件

系统根插件：插件树顶端，持有自己的应用级状态，并仅挂载一个子插件 `worker`
（Composite —— 它是 home 的**动态内置替身**，与 home 共用系统根目录）。

## 职责

- **自身配置**：读写 `<homedir>/PLUGIN.yml`（工作区 / 最近记录）。**不再管任何子插件的
  配置**——各插件配置在自己的目录里（`<插件>/PLUGIN.yml`），谁写谁读。
- **拓扑挂载**：构造 worker (Composite)，并把系统根目录（`PLUGIN_DIR`）与系统必备
  插件清单（`SYSTEM_PLUGINS` → `REQUIRED_PLUGINS`）告知它；由 Composite 扫描
  `<homedir>/*` 挂载子插件。
- **自身终结的路由**：`home/*`、`work/*`。

## 路由

`home/*` 与 `work/*` 由本插件自身终结；其余路径剥离首段后转发给 `worker`。
**完整清单以 [ROUTES.md](../../../../docs/reference/ROUTES.md) §Home 插件为准**——本 README 不抄路由表。

> 资源类别清单不设终结路由：`<根>` 自身的 `vdfs/list` 就是子目录清单
> （由组合容器逐子插件收集，见 `../composite/README.md` 与 `docs/design/vdfs.md` §2.5）。

## 关联

- 动态容器机制：`../composite/README.md`
- 架构层级图：`docs/architecture/OVERVIEW.md`
