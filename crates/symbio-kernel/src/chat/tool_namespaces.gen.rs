//! 工具主体的命名空间集合 —— **由 `scripts/facts/tool-namespaces.mjs` 生成，请勿手改**。
//!
//! 事实源：各插件自己目录里的 `PLUGIN.yml` 的 `tool_namespace:` 行。
//! 加一个带工具的插件 = 在**自己目录**里写一行；`kernel` 的匹配臂不动。
//!
//! 不在这里的三个命名空间：`mcp` / `agent` / `legacy`——它们是工具名拼接规则的
//! 产物（`mcp__server__tool`、`agent_xxx`、兜底段），不是部署事实，住 `principal.rs`。

/// 工具主体的命名空间：`<ns>_<tool>` 合成为 `<ns>/<tool>`（授权表按这个地址判）。
#[rustfmt::skip]
pub const TOOL_NAMESPACES: &[&str] = &["local", "vdfs", "web"];
