//! 装配验收（通用判据）：装配清单里的每个包都能被构造，每个已链接包交来的端口
//! 都进冻结表并有一行 `PORT_ACTOR` 受理行。
//!
//! 本文件不含任何具体插件名——**加一个能力包 = 建目录 + 写 `PLUGIN.yml` + 生成物**，
//! 不新增验收文件。判据遍历的是装配清单与 `inventory` 里的声明/工厂，而不是某个探针。
//!
//! 「不申报工具的包不改变工具面」由 e2e 的字节棘轮承担
//! （`e2e/cases/t46-prompt-shape-baseline.mjs`）：工具面是模型请求体的一部分，
//! 只在 CLI 外面量得准；宿主 crate 里不重复造一份。

#[cfg(test)]
#[path = "assembly_acceptance.test.rs"]
mod tests;
