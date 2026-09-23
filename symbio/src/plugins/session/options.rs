//! 会话选项 —— 宿主端点与会话自有选项的**字段声明**
//!
//! ## 角色
//!
//! session 插件是**选项宿主**：会话页输入区下方的选项栏与会话详情页的可修改项
//! 是**同一批字段**（工作目录 / 执行风险 / 运行模式 / 心跳任务 + 各插件贡献的
//! 智能体 / Model）。本模块声明其中**会话自有**的四项；其余由贡献方在自己的
//! `traverse(available_options)` 分支注册（agent → 智能体、model → Model）。
//!
//! ## 三条通路（`docs/archive/session-options-unification.md` §3.2）
//!
//! ```text
//! 定义   <根>/session/<id> → node.schema               = DetailDefinition { binding: "option", … }
//!        <根>/session      → new_types[session].schema = 同一份
//! 当前值 <根>/session/<id> → node.attributes.metadata  （键 = 定义里的字段 key）
//! 落库   vdfs/write(<根>/session/<id>, {"metadata": {<字段 key>: <值>}})
//! ```
//!
//! 定义只声明「有哪些字段、候选有哪些、什么条件禁用」——**不带当前值**，
//! 因此与「是哪个会话」无关：可在会话清单里算一次、给每一项复用。
//!
//! ⚠️ 这里曾经并行一条 `options/list` 通道（`OptionNode` 节点协议，带
//! `action.endpoint` / `action.bind` 点路径 / `display` 显示策略），已于
//! 2026-09-23 整体下线：同一件事有两条下发通道，而守卫不会因为「两边说的
//! 不一样」变红。
//!
//! ## order 约定（跨插件协调，禁止插件间直接依赖）
//!
//! 插件之间不可见，故 order 采用**号段约定**（各自定义本地常量）：
//! `10` 工作目录 / `20` 智能体 / `30` Model / `40` 风险等级 / `50` 运行模式 /
//! `60` 心跳任务；新增贡献方取空闲号段。
//!
//! 号段只用于**收集层排序**，不下发：前端收到的是已排好序的字段数组
//! （见 `symbio_core::option::OptionVisitor::register_option_field`）。

use super::plugin::SessionPlugin;
use crate::symbio_core::schemas::detail::{
    DetailAction, DetailCondition, DetailDefinition, DetailField, DetailOption, DetailSection,
    DETAIL_PICK_DIRECTORY,
};
use crate::symbio_core::InvokeRequest;
use serde_json::{json, Value};
use std::sync::Arc;

/// 会话自有选项的展示顺序（号段见模块文档）
const ORDER_WORKDIR: i32 = 10;
const ORDER_RISK: i32 = 40;
const ORDER_MODE: i32 = 50;
const ORDER_HEARTBEAT: i32 = 60;

/// 心跳任务默认空闲间隔（秒），与前端历史默认值一致
const DEFAULT_HEARTBEAT_INTERVAL: i64 = 300;

impl SessionPlugin {
    /// 会话自有选项的**字段声明**（供 `node.schema` 下发）。
    ///
    /// 只声明**有哪些字段**，**不带当前值**——值随节点 `attributes.metadata`
    /// 下发（设计文档 §3.2）。因此本函数与「是哪个会话」无关：可在会话清单里
    /// 算一次、给每一项复用（清单一次取全是既有取向，见
    /// `session/docs/vdfs-session-messages.md` S8）。
    ///
    /// 返回 `(order, field)`：`order` 只给收集层排序用，不下发。
    pub(crate) fn session_option_fields(&self) -> Vec<(i32, DetailField)> {
        vec![
            (ORDER_WORKDIR, workdir_field()),
            (ORDER_RISK, risk_field()),
            (ORDER_MODE, mode_field()),
            (ORDER_HEARTBEAT, heartbeat_field()),
        ]
    }

    /// 会话的**选项定义**（`binding = "option"`）——选项栏与会话详情页共用同一份。
    ///
    /// 载体有两处，内容同一份（一处真相、两处投递）：已落盘会话挂
    /// [`VdfsNode::schema`](crate::symbio_core::vdfs_provider::VdfsNode::schema)，
    /// 新建草稿挂 `VdfsNewType::schema`。
    ///
    /// ## 为什么不需要请求上下文
    ///
    /// 定义只声明「有哪些字段与候选」，值与「是哪个会话」都不在这里
    /// （设计文档 §6）——所以这里用一个**空的请求上下文**收集，且这正是它相对
    /// 旧形态的关键简化：同一份定义在「新建草稿」与「已落盘会话」两个载体上
    /// 逐字节相同，`default` 也始终表示「后端的缺省回落」而非当前值。
    ///
    /// 收集走 `available_options` 广播（各贡献方在同一契约下注册字段）。收集失败
    /// 时按机制约定降级为**空定义**（选项栏退化为不显示），不阻断会话页。
    ///
    /// ⚠️ 每次调用都会广播一次全项目收集——候选项（agent 目录 / provider 表）是
    /// 运行期数据，故不做缓存；若将来成为热点，正确的做法是在**机制层**给
    /// `VdfsProvider` 加「自述可缓存」的通用开关，而不是给会话开特例。
    pub(crate) async fn build_option_definition(&self) -> DetailDefinition {
        let ctx: Arc<dyn InvokeRequest> =
            Arc::new(crate::symbio_core::SimpleRequest::new(None, None));
        let parent = self.get_parent();
        let visitor = crate::symbio_core::collect_options(parent.as_ref(), &ctx).await;
        DetailDefinition {
            // 「值来自外部（节点 metadata），提交只回纯字段值」——与心跳表单同一绑定
            binding: "option".to_string(),
            title_fallback: Some("会话选项".to_string()),
            sections: vec![DetailSection {
                title: None,
                collapsed: false,
                fields: visitor.list_option_fields().await,
            }],
            ..Default::default()
        }
    }
}

// ==================== 字段声明（`node.schema`） ====================
//
// 全是**纯函数、与会话无关**（可在会话清单里算一次、给每一项复用）：
// 这里只说「字段叫什么、候选有哪些、什么条件禁用」，值不在这里。

/// 造一个候选项（`value → label`，可带一行说明）
fn option(value: &str, label: &str, description: &str) -> DetailOption {
    DetailOption {
        value: value.to_string(),
        label: label.to_string(),
        description: (!description.is_empty()).then(|| description.to_string()),
    }
}

/// 工作目录：机制原生取值原语（`pick = directory`）→ 写回 `metadata.workdir`。
///
/// 锁定条件由**声明**给出（`disabled_when`），不由 Rust 算成一个布尔：
/// 「已绑定目录**且**已有对话历史」⇒ 不能更换（不同目录的上下文混在一起会干扰
/// 模型）。两个条件都要——只看「有历史」会把「有历史但从未绑定目录」的会话也锁上，
/// 那本来是可以选的。
///
/// 条件求值的作用域是 `{ ...node.attributes, ...字段值 }`（设计文档 §3.5）：
/// `message_count` 来自节点，`workdir` 来自字段自身。
///
/// ⚠️ 「有历史」写成 `truthy: true` 而**不是** `not_equals: 0`：草稿节点没有任何
/// 属性，`message_count` 缺席。`truthy` 对缺席键求值为 `false`（= 没有历史，可
/// 自由选择），而 `not_equals: 0` 对缺席键求值为 `true`（= 有历史）——那会让
/// **新建会话时工作目录一上来就锁死**。`truthy` 同时覆盖 `0`（无历史）与非 `0`。
///
/// `options` 在这里当**值→标签表**用（`path` 没有候选菜单）：未设置时按钮显示
/// 「未选择目录」，而不是把字段名印上去——紧凑渲染形态的取值规则见
/// [`DetailField::options`] 与前端 `schemas/vdfs-form.compactFieldText`。
fn workdir_field() -> DetailField {
    DetailField {
        key: "workdir".to_string(),
        label: "工作目录".to_string(),
        // 一句话覆盖两种状态：未锁定时前一句是全部；锁定时按钮变灰，这里就是原因
        description: Some(
            "会话的工作目录（决定文件工具的作用范围）；已有对话历史后不可更换\
             （如需换目录请新建会话）"
                .to_string(),
        ),
        widget: "path".to_string(),
        icon: Some("folder".to_string()),
        pick: Some(DETAIL_PICK_DIRECTORY.to_string()),
        options: vec![option("", "未选择目录", "")],
        disabled_when: Some(DetailCondition {
            all: vec![
                DetailCondition {
                    key: "message_count".to_string(),
                    truthy: Some(true),
                    ..Default::default()
                },
                DetailCondition {
                    key: "workdir".to_string(),
                    truthy: Some(true),
                    ..Default::default()
                },
            ],
            ..Default::default()
        }),
        ..Default::default()
    }
}

/// 执行风险等级：low / medium / high 三档候选
fn risk_field() -> DetailField {
    DetailField {
        key: "risk_level".to_string(),
        label: "执行风险".to_string(),
        description: Some("低于该等级的工具需用户审批".to_string()),
        widget: "select".to_string(),
        icon: Some("risk".to_string()),
        default: Some(json!("medium")),
        options: vec![
            option("low", "低风险", "仅自动执行低风险工具；中/高风险需审批"),
            option("medium", "中风险", "中风险及以下自动执行；高风险需审批"),
            option("high", "高风险", "所有工具自动执行（含高风险）"),
        ],
        ..Default::default()
    }
}

/// 运行模式：interactive / auto 两档候选
fn mode_field() -> DetailField {
    DetailField {
        key: "mode".to_string(),
        label: "运行模式".to_string(),
        description: Some("工具失败时是否阻塞模型继续".to_string()),
        widget: "select".to_string(),
        icon: Some("run-mode".to_string()),
        default: Some(json!("interactive")),
        options: vec![
            option(
                "interactive",
                "交互",
                "需审批/需交互的工具在会话流中显示卡片，等待用户响应",
            ),
            option(
                "auto",
                "自动",
                "无人值守：工具失败返回友好错误让模型自行继续",
            ),
        ],
        ..Default::default()
    }
}

/// 心跳任务的**缺省配置**（未启用 / 300 秒 / 空提示词 / 携带历史）。
///
/// 它是 [`heartbeat_field`] 的 `default`（`form` widget 的子对象缺省值）：
/// 未配置过心跳的会话因此显示「未开启」而不是把字段名印在按钮上，子表单也据此预填。
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
fn heartbeat_field() -> DetailField {
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
/// 做一次，直接在输入框发即可。留着它等于给同一件事两个入口，且按钮那个还
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
#[path = "options.test.rs"]
mod tests;
