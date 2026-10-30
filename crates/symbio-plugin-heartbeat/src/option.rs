//! 会话选项栏里的那一行：`heartbeat` 字段（`widget = "form"` 的结构化子对象）
//!
//! ## 它为什么不在 session 的选项定义里
//!
//! 会话的选项定义是**收集期汇流**的结果（`session/options/mod.rs`：宿主声明自己的字段，
//! 各贡献方在自己的 `traverse(available_options)` 分支里注册）。`heartbeat` 这一格的意思
//! 只有本插件知道（[`super::setting`] 是它的 owner），所以声明也该由本插件产出——
//! 与 `agent` 贡献「智能体」、`model` 贡献「Model」同一条通道，零新机制。
//!
//! 副作用即 [D-077](README.md#d-077-心跳片的落点定为一层的插件目录卸载单位与第二个消费单位都由它给出 "crates/symbio-plugin-heartbeat/src/README") 的守卫后半：停用 `heartbeat`
//! ⇒ 下一轮收集里这一行不在，会话选项栏少一格，其余照旧。
//!
//! ## order 号段是跨插件协调，禁止插件间直接依赖
//!
//! `60` 这个号约定登记在 `crates/symbio-plugin-session/src/options/mod.rs` 的模块文档里（那张号段表就是
//! 协调方式本身）。取号段前先看它，别在两个贡献方之间撞出同一个 order。
//!
//! ## 落库走的是前端已在用的同一条写通道
//!
//! `binding = "option"`：值预填自节点 `attributes.metadata`，保存回
//! `vdfs/write(<会话>)` 的 `{"metadata": {<字段 key>: <值>}}`。本文件因此**不需要**
//! 任何写代码——它只声明字段。

use serde_json::{json, Value};
use symbio_plugin_api::{DetailAction, DetailDefinition, DetailField, DetailOption, DetailSection};

/// 展示顺序号段：60 = 心跳任务（号段表见文件头）
pub(crate) const ORDER_HEARTBEAT: i32 = 60;

/// 心跳任务默认空闲间隔（秒），与前端历史默认值一致
const DEFAULT_HEARTBEAT_INTERVAL: i64 = 300;

fn option(value: &str, label: &str, description: &str) -> DetailOption {
    DetailOption {
        value: value.to_string(),
        label: label.to_string(),
        description: (!description.is_empty()).then(|| description.to_string()),
    }
}

/// 心跳任务的**缺省配置**（未启用 / 300 秒 / 空提示词 / 携带历史）。
///
/// 它是 [`heartbeat_field`] 的 `default`（`form` widget 的子对象缺省值）：
/// 未配置过心跳的会话因此显示「未开启」而不是把字段名印在按钮上，子表单也据此预填。
/// 与 `HeartbeatConfig::default` 同义，
/// 两处的一致性由 `option.test.rs` 钉着。
fn heartbeat_defaults() -> Value {
    json!({
        "enabled": false,
        "interval_seconds": DEFAULT_HEARTBEAT_INTERVAL,
        "prompt": "",
        "include_history": true,
    })
}

/// 心跳任务：结构化子对象字段（`widget = "form"`），子定义见 [`heartbeat_definition`]。
///
/// `options` 在这里当**值→标签表**用：紧凑渲染形态取子定义的 `title_from`
/// （`enabled`）作代表值，再查这张表得到「已开启 / 未开启」（见
/// [`DetailField::form`] 的说明）。`default` 是子对象的缺省配置。
pub(crate) fn heartbeat_field() -> DetailField {
    DetailField {
        key: "heartbeat".to_string(),
        label: "心跳任务".to_string(),
        description: Some("会话空闲时自动触发一次对话（定时任务）".to_string()),
        widget: "form".to_string(),
        icon: Some("heartbeat".to_string()),
        options: vec![option("true", "已开启", ""), option("false", "未开启", "")],
        default: Some(heartbeat_defaults()),
        form: Some(Box::new(heartbeat_definition())),
        ..Default::default()
    }
}

/// 心跳任务的**子定义**（结构化子对象的字段表）。
///
/// `enabled` 开关与三项**基础设置**（空闲间隔 / 任务提示词 / 携带历史）
/// **恒可见**：基础设置不使用 `visible_when` 门控——未启用时用户同样能
/// 看到并可预先填写，开关与参数一次保存即生效（`option` 绑定保存全部字段，
/// 关闭开关也不会丢失已填参数）。
///
/// `title_from` 指向 `enabled`：紧凑渲染形态据此取「一句话摘要」的代表值。
/// 纵向表单渲染器不受影响——它的 `title_from` 只认字符串/数字，布尔会被跳过、
/// 回落到 `title_fallback`（「心跳任务」）。
///
/// 「立即执行一次」**已取消**（2026-09-18）：它原是一个独立命令选项（图标
/// `play`，点了直接调 `session/heartbeat/trigger`），但它的作用与「在输入框里
/// 直接发一条消息」完全重复——心跳的实质就是往会话发一轮提示词，用户想立刻
/// 做一次，直接在输入框发即可。留着它等于给同一件事两个入口，而按钮那个还
/// 绕开了对话本身。
fn heartbeat_definition() -> DetailDefinition {
    DetailDefinition {
        // option 绑定：预填自节点 metadata、保存回 VDFS 写通道（与 VDFS 详情表单同一方言）
        binding: "option".to_string(),
        title_from: vec!["enabled".to_string()],
        title_fallback: Some("心跳任务".to_string()),
        sections: vec![DetailSection {
            title: None,
            collapsed: false,
            fields: vec![
                DetailField {
                    key: "enabled".to_string(),
                    label: "启用心跳任务".to_string(),
                    description: Some(
                        "开启后，会话空闲达到设定间隔会自动以「任务提示词」触发一次对话；\
                         正在工作中的会话不会触发"
                            .to_string(),
                    ),
                    widget: "toggle".to_string(),
                    ..Default::default()
                },
                // 以下三项为「基础设置」：**恒可见**（不随启用开关显隐）——
                // 关闭心跳时也允许预先填写，开启后一次保存即生效。
                DetailField {
                    key: "interval_seconds".to_string(),
                    label: "空闲间隔（秒）".to_string(),
                    description: Some("会话无活动多久后自动触发".to_string()),
                    widget: "number".to_string(),
                    default: Some(json!(DEFAULT_HEARTBEAT_INTERVAL)),
                    min: Some(10.0),
                    step: Some(10.0),
                    ..Default::default()
                },
                DetailField {
                    key: "prompt".to_string(),
                    label: "任务提示词".to_string(),
                    description: Some("每次心跳自动发送给 AI 的内容".to_string()),
                    placeholder: Some(
                        "例如：检查当前工作目录的待办，主动推进一项不依赖用户输入的小任务。"
                            .to_string(),
                    ),
                    widget: "textarea".to_string(),
                    rows: Some(4),
                    full_width: true,
                    ..Default::default()
                },
                DetailField {
                    key: "include_history".to_string(),
                    label: "携带历史会话信息".to_string(),
                    description: Some(
                        "关闭后，心跳触发时不带历史上下文（以全新上下文执行）".to_string(),
                    ),
                    widget: "toggle".to_string(),
                    default: Some(json!(true)),
                    ..Default::default()
                },
            ],
        }],
        actions: vec![DetailAction {
            id: "save".to_string(),
            label: "保存".to_string(),
            style: "primary".to_string(),
            ..Default::default()
        }],
        ..Default::default()
    }
}

#[cfg(test)]
#[path = "option.test.rs"]
mod tests;
