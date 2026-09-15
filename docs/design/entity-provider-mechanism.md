# 实体提供者机制（EntityProvider）—— 已废止

> **状态：已废止**。本文件保留只为让既有链接不失效；**权威规范是
> [vdfs.md](./vdfs.md) §13.4**，历史形态见
> [archive/entity-provider-mechanism.md](../archive/entity-provider-mechanism.md)。

## 废止了什么

| 已删除 | 位置 | 取代者 |
|---|---|---|
| `EntityProvider` trait（20 个钩子） | `symbio_core/entities.rs` | 各插件的 `impl VdfsProvider` |
| `provider_registry()` / `EntityProviderInfo` | 同上 | 各 provider 的 `label()` / `order()` / `icon()` / `root_new_types()` |
| `EntityVdfsAdapter`（1504 行） | `symbio_core/vdfs/entity_adapter.rs` | 无——插件直连，不需要适配器 |
| `nav_meta_of()` | `symbio_core/entities.rs` | 各 provider 自持字面量 |

## 现在的形态

1. **每个资源插件直接实现 `VdfsProvider`**——用现成的 `list` / `stat` / `read` /
   `write` / `delete` / `action` / `watch` 表达自身语义。`vdfs_provider.rs`
   是核心机制，未因本次收敛做任何修改。
2. **`symbio_core/entities.rs` 降为「存储原语自由函数」**——没有 trait、没有
   注册表，只保留 `EntityStore` 的落盘：`storage_service` / `list_entity_ids` /
   `read_manifest` / `write_entity_manifest` / `delete_entity_dir` /
   `import_zip_to_entity` / `export_entity_zip`，以及 zip / base64 工具。
3. **差异回到各插件**：清单来源、摘要口径、manifest 校验、内存注册表同步、
   容器语义，都是各插件自己的事，不再挤进一张通用接口。
4. **变更广播按类型全局持有**：`vdfs::host::notify_change` / `watch_changes` /
   `unwatch_changes`（写 / 删的唯一实现与目录自管型 provider 都调它）。

## 为什么

trait + 适配器这一层曾以「新增实体类型 VDFS 侧零改动」为价值主张，但实际代价是
把**每类资源的差异**挤进一张 20 钩子的通用接口，再由一个 1500 行的适配器去猜
（「有容器但无详情定义 ⇒ 当目录」这类规则就是猜的产物）。去掉之后，每类资源的
语义回到自己的 `impl` 里：一眼可见，改一处只影响一处。

代价是 plumbing（`id_of` / 节点合成 / 路径解析）在几个插件间各有一份。这是
**有意的取舍**：这些代码短、稳定、且各自很快就长出差异（model 的列表来自内存、
skill 的摘要解析 SKILL.md、agent 有三层容器寻址），抽成共享层只会重新引入
「通用参数该叫什么」的争论——而那正是 `category` / `list_items` 被否决的原因。
