//! 五类扩展的**局部性夹具**（[DE-023](../../../docs/003-design/023-内核边界与扩展局部性.md "DE-023-内核边界与扩展局部性") A1）。
//!
//! 每类扩展各一个用例：**事实 / 视图 / 策略 / 插件 / 共享类型**。夹具全部住
//! `plugins/`、只用内核的公开扩展口（`PluginDeclarations` + `Builder` 的声明类型、
//! `Projection`、`CapabilityFamily` / `CapabilityRow`），因此「加一样东西要不要动
//! 内核」有了一条可执行的答案：这些用例编译并通过，而内核文件一个都没改。
//!
//! 它们是**测试夹具**，不是生产声明——`#[cfg(test)]` 保证不进产物；生产声明仍由
//! `plugins::loop::control_decl` 等 owner 文件交出。

#[cfg(test)]
#[path = "lib.test.rs"]
mod tests;
