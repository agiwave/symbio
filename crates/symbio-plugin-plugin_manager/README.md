---
kind: design
id: crates/symbio-plugin-plugin_manager/README
title: Plugin_manager 插件（插件管理）
status: active
created: 2026-09-25
impl: done
---

# Plugin_manager 插件（插件管理）

`<根>/plugin_manager` 挂载点的实现者：本智能体**插件集合**的门面——列出已安装插件、
启用 / 停用 / 卸载，从**插件市场**添加尚未安装的可选插件，以及打开各插件的配置。

## 定位：容器是权威，本插件是视图

「这个智能体由哪些插件组成」是**容器的事实**：插件根下一层目录 = 一个插件
（见 `symbio_sdk::plugin::dir`）。本插件**不自己扫目录**，也不持任何状态——它经
`ctx.parent` 取回所在容器的 VDFS 视图（`Composite::get_vfs_provider`），把容器的
注册表条目列成清单，并把动词转发回容器执行：

| 谁 | 干什么 |
|---|---|
| 容器（`composite`） | 注册表的**权威**：扫目录、三条判据（合格 / 必需 / 启用）、启停与增删 |
| 本插件 | 注册表的**视图**：合成条目节点与动作、转发动词 |

于是「有哪些插件」只有一份实现。本插件也不认识任何具体插件。

## 路由

**无**（权威登记见 [路由参考](../../docs/004-ref/003-ROUTES.md "RE-003-ROUTES") §Plugin_manager 插件）。
本插件是无状态 provider，`route` 恒 `NotFound`。

## 清单 = 各插件的条目 + 插件市场 + 自有分区

顺序是插件条目在前、插件市场居中、自有分区垫后（前两者才是用户在管理页里要动手的东西）：

- 条目来自容器的注册表（`Action(plugins)`，见 `composite/vdfs.rs`），**含没有 VDFS 的
 与已停用的**——后者必须列出来，否则用户看不到自己刚停用的那个，也就永远点不回
 「启用」；
- 一个条目**就是那个插件的配置**：入口挂在 `<根>/plugin_manager/<插件名>`，读 / 写
 转发到容器根下的 `<插件名>/PLUGIN.yml`——同一份配置仍然只有一个**文件**、一份
 **定义**（拥有者给的那份，见 `symbio_sdk::capability::configurable`）；
- 没有配置文档的插件（如 `telegram`）合成一张**只读概览**（`binding = info`），
 否则点开一片空白，连装配按钮都没有落脚处；
- 本插件只补一个场景标签（`kind = plugin_manager`）——列表在哪儿，场景就是哪儿。

条目的状态点三态：`active`（启用且已挂载）/ `disabled`（停用）/ `failed`（用户想装它，
这一轮装配没装成）。第三种由容器的注册表条目带出来（`PluginEntry::mount_error`，失败原因
随节点描述呈现）——本插件只**呈现**它，不判断为什么失败：判据在容器，它才看得见工厂表。

## 装配动作：按装配态显隐

条目上恒有三个动作（启用 / 停用 / 卸载），显隐由 `DetailAction.when` 判定：

- `启用`：停用时可见；
- `停用`：已启用**且**允许停用时可见（界面底座插件拒绝停用，见装配方的
 `ASSEMBLY_UNDISABLABLE_PLUGINS`）；
- `卸载`（`id = delete`）：非必需时可。**这个 id 是刻意的**：它就是前端机制动作的
 同名动作，因此声明它既换了文案（「删除」→「卸载」），也顶掉了兜底那一个——
 必需插件于是连按钮都不出现，而不是点了才报错。

`when` 只能对**表单模型**求值，而装配态不在配置正文里：`read` 因此把装配态叠成几个
**投影键**（`plugin_enabled` / `plugin_required` / `plugin_can_disable` / `plugin_version`，
见 `symbio_sdk::plugin::dir` 的同名常量），条件据此判定。

动作**恒在定义里**（只靠条件显隐），不按当下状态增删：定义是节点的一部分，而状态一变
调用方只会**重读正文**、不会重新取节点——按状态增删会让按钮停在旧状态上。

## 插件市场（`market`）

「装一个还没装的插件」的入口不再是根节点上那张由容器声明的通用新建表单，而是清单中一个
专门的 `market` 节点（`ext = market`，前端渲染 `MarketPanel`）。它夹在插件条目与自有分区
之间——「管理已装的」和「添加未装的」是用户在管理页里要动手的两件事，挨在一起。

它不是插件条目，也没有自己的配置：候选在 `read` 时现场算，安装与条目复用同一个 `write`
动词，前端因此不必知道容器根上的安装契约（`VDFS_PLUGIN_PROVIDER_FIELD` / `create`）。

候选三判据（缺一不可，见 `market_candidates` 与 D-520）：

1. **可选**——`PLUGIN.yml` 声明 `plugin_optional: true`，即不在 `ASSEMBLY_CORE_PLUGINS` 内
   （核心插件默认加载且不可删除，不进市场）；
2. **未安装**——当前容器注册表里没有它（装过的回到列表，由列表负责卸载）；
3. **本构建链接了工厂**——`creator_has(id)` 为真。只按清单渲染会得到一个「点了没反应」
   的按钮：工厂没链进本二进制，容器没有它的 creator，安装只会得到 NotFound。

`read` 返回 JSON 文本（`application/json`）：
`{ "candidates": [{ name, provider, source, label }] }`。`source` 目前恒为 `builtin`——
它是**未来外部插件的呈现位**：D-407 仍暂缓进程外插件协议，本期市场只做「编译期已链接
插件的运行期装卸」，先把入口和契约立住。

`write` 只补一层部署语义的校验：`provider` 必须属于 `ASSEMBLY_OPTIONAL_PLUGINS`，否则核心
插件会得到「已在装配中」这种答非所问的提示。「本构建是否链接」不在这里重复判——容器
`install` 内部的 `provider_gap` 才是那个判据的 owner（候选列表已提前把装不了的挡在展示之前）。
校验通过后转成容器根上的 `Write { create: true }`，由容器执行真正的挂载。

## 装配态一变就广播

`act` / `install` / `uninstall` 成功后各广播一次变更（`vdfs_notify_change`）：条目集合与
当前选中项的**动作集**都变了，订阅方据此重拉并重读当前项（前端 `useVdfs` 的既有收敛
路径），按钮因此不会停在旧状态上。订阅走 `vdfs_watch_changes`（本插件是自管变更源）。

## 自有分区（`appearance` / `about`）

- 清单在 `OWN_SECTIONS` 中登记（`id` / `label`），由 `impl VdfsProvider` 的 `list` 产出。
- 它们的数据由**前端 store 自持**（主题、版本信息等不在后端），因此这两个分区在 VDFS
 上是**只读节点**（`access = r`，`ext` 即分区 id）：`vdfs/read` / `vdfs/write` 对它们
 恒 `Forbidden`，前端按 `ext` 直接渲染专属面板（`registry/vdfsTypes.ts` 的
 `appearance` / `about` 渲染器）。
- 分区节点显式声明 `status = none`：静态分区没有「运行中 / 就绪」可言，不清掉节点
 `status` 的缺省就会在列表里画一个绿点——那是个不存在的信息。
- 原 `plugin_manager/config/get` / `plugin_manager/config/set`（以及更早的
 `plugin_manager/list` / `plugin_manager/get`）已全部下线：分区取值由 VDFS 承担，
 插件配置由各插件自己的配置文件承担。

## 关联

- 核心 / 可选插件清单：`crates/symbio-deploy/src/lib.rs` 的 `ASSEMBLY_CORE_PLUGINS` /
  `ASSEMBLY_OPTIONAL_PLUGINS`（由 `scripts/facts/assembly.mjs` 从 `PLUGIN.yml` 投影生成，D-520）
- 装配判据与运行期增删：`crates/symbio-plugin-composite/src/registry.rs`
- 注册表动词（根上的 `Action(plugins|enable|disable)` / `Write` / `Delete`）：
 `crates/symbio-plugin-composite/src/vdfs.rs`
- 插件目录与配置：`symbio_sdk::plugin::dir`（`PLUGIN.yml` 的保留键）
- 可配置声明通道：`symbio_sdk::capability::configurable`（`ConfigurableVisitor` / `CONFIGURABLE_VISITOR`）
- VDFS 机制：`docs/003-design/009-VDFS规范.md`（`DetailDefinition` 是 `ext = form` 节点的
 `schema` 方言；配置文件地址 = `<插件目录>/PLUGIN.yml`，见 §3.4 / §13.1）
