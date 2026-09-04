# Symbio 开发编码规范

> **文档类型：How-to guide（操作指南）** — 编码与协作规范，解决问题导向。

> 本文档基于**当前代码**（纯 Rust 核心库 + E2E CLI）。
> 早期"Tauri + Vue 前端"叙述中关于 Vue/TS 的引用已不再适用。

## 1. 总体原则

1. **机制化优先**：避免对关系、类型、行为做硬编码 `match`。
   哪些字段是关系、哪些 ID 是 `kind`、如何展示类型，**全部由 `prop` CU 派生**。
   需要在核心代码里写 `match` 的位置基本只剩"路由分发"与"调度循环"两处。
2. **统一事实来源**：每个业务概念的真理只在**一处**维护——
   CU 的元数据在 `seed_cus.jsonl`，插件接口在 trait，
   工具名由路径派生。任何第二份副本都意味着重构风险。
3. **自相似性**：所有插件实现同一 `Plugin` Trait，
   容器插件通过 `route()` 递归剥前缀；禁止出现"特殊插件"路径。
4. **不可变优先**：CU 实例在写入后基本只读；
   写路径集中到 `Store::update` / `Store::add`，避免散落的 in-place mutation。
5. **配置即插件**：新功能优先通过 `~/.symbio/config.yaml` 的 `plugins.<name>` 挂载，
   而不是写死到代码里。

## 2. Rust 后端代码规范

### 2.1 错误处理

* 业务层使用 `Result<T, PluginError>`；`?` 直接传播。
* 跨边界（`PluginPayload` / 帧）的错误必须携带稳定 `code`（见 `API_DESIGN.md` §6）。
* 不要 `unwrap()` / `expect()` 在生产路径中。允许在 invariant 校验处使用 `expect("msg")`
  并配以单元测试覆盖。
* 错误日志统一使用 `crate::plugin_warn!` / `plugin_error!` 宏，便于按插件过滤。

### 2.2 并发与状态

* 业务插件**无状态优先**；需要状态时优先 `Arc<AtomicXxx>`。
* 复杂状态用 `Arc<RwLock<T>>`，**先 clone 再 lock**：
  ```rust
  let st = self.inner.clone();
  let mut g = st.write().await;
  *g = new_value;
  ```
  避免在锁内执行 `await` 跨越其他锁。
* 禁止 `tokio::spawn` 静默吞错；所有后台任务必须把错误用 `plugin_error!` 记录。

### 2.3 序列化与字段命名

* 跨端结构集中放在 `symbio/src/symbio_core/schemas/`。
* 字段命名 Rust 端用 `snake_case`；host 侧按需 `#[serde(rename_all = "camelCase")]`。
* 对外暴露的 JSON 字段**必须**是**可枚举**的——禁止 `serde_json::Value` 当作公共字段。

### 2.4 Trait 抽象

* `Plugin` 是 V3.0 上下文注入版（`self: Arc<Self>`、`ctx: Arc<dyn InvokeRequest>`）。
  `&self` 的 `Plugin` 已**不推荐**新增。
  插件通过 `submit_object_creator!` 宏注册构造函数，由 `create_object::<dyn Plugin>(id, ctx)` 实例化，无 `PluginFactory` 概念。
* 容器插件不要做"类型分发"——直接按 `PATH` 字符串剥前缀。
* 叶子插件在 `route("xxx", ctx)` 内做参数校验、调用 Store / 业务逻辑、构造响应。

### 2.5 测试约定

* 单元测试写在被测文件同模块底部 `#[cfg(test)] mod tests`。
* 集成 / 行为测试写在 `symbio/tests/` 中（当前以模块内联测试为主，尚无独立 `tests/` 目录）。
* 大规模测试脚本（如能力验证）放在仓库根 `scripts/` 下（如 `test-capabilities.mjs` / `test-capability-chain.mjs`）。
* **脚本一律平台无关**：用 Node.js（`.mjs`）编写，不使用 `.sh` / `.ps1` / `.bat`。
  调用子进程时传参数数组并设 `shell: false`，避免跨平台的引号转义差异。

## 3. Agent 子系统规范

详见 [PRINCIPLES.md](../../symbio/src/plugins/agent/docs/PRINCIPLES.md)
与 [COGNITION.md](../../symbio/src/plugins/agent/docs/COGNITION.md)，
本节只做要点回顾。

### 3.1 关系机制化

* 关系名（如 `is_a` / `has` / `part_of`）**不是硬编码常量**。
  `RelationPropRegistry::from_prop_cus(&prop_cus)` 在启动时从 `seed_cus.jsonl` 派生。
* 判断关系用 `cu.has_relation("is_a", "fact")` 或 `cu.is_type("fact")`
  （后者是前者的语法糖，**不是**独立硬编码）。
* 业务代码**禁止**出现 `if rel == "is_a"` 这种字符串比较；
  必须借助 `RelationPropRegistry` 或语义化 API。

### 3.2 展示机制化

* `kind` 类型清单、显示名（用 `id`）、`priority` 排序均由 `seed_cus.jsonl` 派生。
* 已经在 `conversation.rs` 中实现 `dynamic_types_with_priority()` 与
  `type_display_name(&id)`；禁止再写新的 `match kind` 分发。

### 3.3 `identity` 是 CU 实例而非类型

* `id == "identity"` 的 CU 是**每 agent 必备的一条 fact**，不是 kind 类型。
* 识别必须用 `cu.id() == Some("identity")`；禁止用 `is_a` 匹配。

## 4. CLI 与 host 集成

* CLI 与核心库共用 crate（`symbio` lib + `bin/seed_agents.rs`）。
* CLI 通过 `symbio::initialize()` + `create_root_plugin()` 拿到 root，
  再以**普通 host** 身份发起 `route()`。
* 任何 host 都能独立调用 `route("local/shell", …)` 做集成测试。

## 5. 配置管理

* 全局配置位于 `~/.symbio/config.yaml`。
* 插件独立配置通过 `route("<plugin>/config", { action: "get" | "set", ... })` 读写。
* 写配置时同步触发 `setting/set` 钩子，方便持久化与多端同步。
* 路径分隔统一使用 `/`；Windows 平台在比较前用 `normalize_path_for_comparison`
  去除 `\\?\` 前缀（见 `local/policy.rs`）。

## 6. 文档与代码同源

* 业务概念**优先用代码表达**（trait / 数据 / 测试），文档作为外部描述。
* 任何对核心机制的修改（关系、类型、插件接口）必须同步：
  1. 单元测试 / 集成测试
  2. `docs/explanation/` 与 `docs/reference/`（或插件自包含 docs）
  3. `CHANGELOG.md`
* 历史性叙述（Tauri / Vue 时代）集中迁到 `docs/archive/design_docs/` 并标注"历史参考"。
