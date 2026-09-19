//! 会话页选项机制 —— 宿主端点与会话自有选项
//!
//! ## 角色
//!
//! session 插件是**选项宿主**：会话页输入区下方的根选项列表由本插件在
//! `options/list` 端点下发。下发的节点来自两处，合流后一次响应：
//!
//! 1. **全项目收集**（`collect_options` + `Plugin::traverse`）——agent 插件
//!    贡献「智能体」、model 插件贡献「Model」等；
//! 2. **宿主自有**——工作目录 / 运行模式 / 风险等级 / 心跳任务：这四类
//!    选项的状态就在会话自身（`session.metadata`），由本插件在 `traverse`
//!    参与 `available_options` 时直接构造（与其它插件同构，走同一注册通道）。
//!
//! ## 状态落库
//!
//! 所有状态型选项的选择动作统一指向 [`SESSION_STATE_ENDPOINT`]
//! （`worker/session/update`），把值合并写入 `session.metadata`。后端各解析链
//! （`orchestrator::resolve_session_params` / `tool_executor`）已按 metadata
//! 回退取值，因此**前端不需要知道任何业务字段名**——这正是「前端零业务代码」
//! 的关键：选项的展示、选择、落库全部由后端声明，前端只做通用渲染与转发。
//!
//! ## order 约定（跨插件协调，禁止插件间直接依赖）
//!
//! 插件之间不可见，故 order 采用**号段约定**（各自定义本地常量）：
//! `10` 工作目录 / `20` 智能体 / `30` Model / `40` 风险等级 / `50` 运行模式 /
//! `60` 心跳任务；新增贡献方取空闲号段。

use super::plugin::SessionPlugin;
use crate::symbio_core::schemas::detail::{
    DetailAction, DetailDefinition, DetailField, DetailSection,
};
use crate::symbio_core::schemas::options::{
    OptionAction, OptionDisplay, OptionNode, OptionType, OptionsRequest, OptionsResponse,
    OPTION_PICK_DIRECTORY,
};
use crate::symbio_core::vdfs_provider::{VDFS_STATUS_ACTIVE, VDFS_STATUS_DISABLED};
use crate::symbio_core::{
    InvokeRequest, InvokeRequestExt, InvokeResponse, PluginPayload, SESSION_ID, WORKDIR,
};
use serde_json::{json, Value};
use std::sync::Arc;

/// 会话自有选项的展示顺序（号段见模块文档）
const ORDER_WORKDIR: i32 = 10;
const ORDER_RISK: i32 = 40;
const ORDER_MODE: i32 = 50;
const ORDER_HEARTBEAT: i32 = 60;

/// 心跳任务默认空闲间隔（秒），与前端历史默认值一致
const DEFAULT_HEARTBEAT_INTERVAL: i64 = 300;

/// 取非空去空白字符串
fn non_empty(s: &Option<String>) -> Option<&str> {
    s.as_deref().map(str::trim).filter(|s| !s.is_empty())
}

/// 路径末段（工作目录展示名；与前端 basename 语义一致）
fn basename(p: &str) -> String {
    p.trim_end_matches(['/', '\\'])
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(p)
        .to_string()
}

/// 从 metadata 取字符串字段（去空白）
fn meta_str(metadata: &Value, key: &str) -> Option<String> {
    metadata
        .get(key)
        .and_then(|v| v.as_str())
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_string)
}

/// `options/list` —— 选项列表（根层 / 子层由 `parent` 区分）。
///
/// 根层：把会话当前状态注入收集上下文，广播全项目收集选项，返回节点列表；
/// 子层（`parent` 非空）：在**同一份收集结果**中定位该节点并返回其子项
/// （懒加载与 VDFS `vdfs/list` 的 `parent` 同构，单一通道）。
pub(crate) async fn handle_list_options(
    plugin: &SessionPlugin,
    ctx: Arc<dyn InvokeRequest>,
) -> InvokeResponse<PluginPayload> {
    let req = ctx.payload::<OptionsRequest>().ok().unwrap_or_default();

    let session_id = non_empty(&req.session_id)
        .map(str::to_string)
        .or_else(|| ctx.get(SESSION_ID).filter(|s| !s.trim().is_empty()));

    // 收集上下文：会话标识是各贡献插件回填「当前选中值」的依据
    // （agent 插件读 AGENT_ID / model 插件读 PROVIDER_ID，均由本插件从会话
    //  metadata 注入——贡献方无需自行加载会话，保持插件间零耦合）
    let collect_ctx = ctx.fork();
    if let Some(sid) = &session_id {
        collect_ctx.set(SESSION_ID, sid.clone());
        if let Ok(session) = plugin.get_or_create_session(sid).await {
            let meta = &session.metadata;
            if let Some(aid) = meta_str(meta, "agent_id") {
                collect_ctx.set(crate::symbio_core::AGENT_ID, aid);
            }
            if let Some(pid) = meta_str(meta, "provider_id") {
                collect_ctx.set(crate::symbio_core::PROVIDER_ID, pid);
            }
            // workdir 是 agent 插件发现「工作区级 bundle」的依据
            // （BundleStore 两级发现：工作区级 + 全局级），必须一并注入
            if let Some(wd) = meta_str(meta, "workdir") {
                collect_ctx.set(WORKDIR, wd);
            }
        }
    }

    let parent_plugin = plugin.get_parent();
    let visitor = crate::symbio_core::collect_options(parent_plugin.as_ref(), &collect_ctx).await;
    let mut nodes = visitor.list_options().await;

    if let Some(parent_id) = non_empty(&req.parent).map(str::to_string) {
        nodes = find_node(&nodes, &parent_id)
            .map(|n| n.children.clone())
            .unwrap_or_default();
    } else {
        // 根层 = 会话输入区下方的选项栏：统一应用紧凑显示策略（机制级、由后端声明）。
        // 仅「图标 + 当前值」，类别标签移入悬停提示，压缩横向空间。贡献方可在节点上
        // 显式 `with_display(true)` 覆盖，恢复「图标 + 类别标签 + 当前值」双段渲染。
        // 子层（级联菜单内）不应用——选择时类别标签是必要信息。
        apply_chat_bar_display_defaults(&mut nodes);
    }

    // 选项宿主的统一注入：任一选项（含各插件贡献的）都在会话作用域内执行，
    // 宿主为每个 action.payload 补上 `session_id`，贡献方无需感知会话标识
    // （与 callPlugin 的路由上下文同源，属机制级而非业务级）。
    if let Some(sid) = &session_id {
        inject_session_scope(&mut nodes, sid);
    }

    Ok(PluginPayload::new(&OptionsResponse { nodes }))
}

/// 选项栏（会话输入区下方）统一显示策略：根选项默认仅显示「图标 + 当前值」
/// （紧凑模式），类别标签移入悬停提示。
///
/// 仅当贡献方未在节点上显式声明 `display` 时回落此默认值——贡献方可用
/// `OptionNode::with_display(true)` 覆盖，恢复「图标 + 类别标签 + 当前值」。
/// 调用方须仅在根层（选项栏）调用，子层（级联菜单内）保持类别标签。
fn apply_chat_bar_display_defaults(nodes: &mut [OptionNode]) {
    for node in nodes.iter_mut() {
        if node.display.is_none() {
            node.display = Some(OptionDisplay { show_label: false });
        }
    }
}

/// 为节点树中每个选项动作注入会话作用域（`session_id`）。
///
/// 仅注入对象/缺省载荷（标量或数组载荷保持原样，避免破坏自定义契约）；
/// 载荷中已显式声明 `session_id` 的以贡献方为准。
fn inject_session_scope(nodes: &mut [OptionNode], session_id: &str) {
    for node in nodes.iter_mut() {
        if let Some(action) = node.action.as_mut() {
            if action.payload.is_null() || action.payload.is_object() {
                if !action.payload.is_object() {
                    action.payload = json!({});
                }
                if action
                    .payload
                    .get("session_id")
                    .and_then(|v| v.as_str())
                    .map(|s| s.trim().is_empty())
                    .unwrap_or(true)
                {
                    action.payload["session_id"] = json!(session_id);
                }
            }
        }
        inject_session_scope(&mut node.children, session_id);
    }
}

/// 在节点树中按 id 定位（深度优先）
fn find_node<'a>(nodes: &'a [OptionNode], id: &str) -> Option<&'a OptionNode> {
    for n in nodes {
        if n.id == id {
            return Some(n);
        }
        if let Some(found) = find_node(&n.children, id) {
            return Some(found);
        }
    }
    None
}

impl SessionPlugin {
    /// 参与 `available_options` 收集：构造会话自有选项节点。
    ///
    /// 有会话上下文时从 `session.metadata` 回填「当前选中值」；无会话
    /// （新建草稿态）时全部按**默认值**下发——使选项行在会话真正创建前即可
    /// 完整渲染，草稿选择由前端通用缓冲持有，创建时统一写入 metadata。
    pub(crate) async fn build_option_nodes(&self, ctx: &Arc<dyn InvokeRequest>) -> Vec<OptionNode> {
        let (meta, has_messages) = match ctx.get(SESSION_ID).filter(|s| !s.trim().is_empty()) {
            Some(session_id) => match self.get_or_create_session(&session_id).await {
                Ok(session) => {
                    let has_messages = !session.messages.is_empty();
                    (session.metadata, has_messages)
                }
                Err(_) => (Value::Null, false),
            },
            None => (Value::Null, false),
        };

        vec![
            self.workdir_option(meta_str(&meta, "workdir"), has_messages),
            self.risk_option(meta_str(&meta, "risk_level")),
            self.mode_option(meta_str(&meta, "mode")),
            self.heartbeat_option(meta.get("heartbeat").cloned()),
        ]
    }

    /// 工作目录：机制原生取值原语（`pick = directory`）→ 写回 metadata.workdir
    ///
    /// 已绑定目录且**已有对话历史**时置为只读（`enabled = false`）：不同目录的
    /// 上下文混在一起会干扰模型，故须新建会话才能换目录（原前端锁语义上提为后端声明）。
    fn workdir_option(&self, workdir: Option<String>, has_messages: bool) -> OptionNode {
        let node = OptionNode::invoke(
            "workdir",
            "工作目录",
            OptionAction {
                pick: Some(OPTION_PICK_DIRECTORY.to_string()),
                ..OptionAction::session_state_bind("metadata.workdir")
            },
        )
        .with_icon("folder")
        .with_order(ORDER_WORKDIR);

        let node = match workdir {
            Some(wd) => node.with_value_label(&wd, basename(&wd)),
            // 尚未绑定：仍可点击（选择目录），展示为未设置
            None => node.with_value_label("", "未选择目录"),
        };

        if has_messages
            && node
                .value
                .as_deref()
                .map(|v| !v.is_empty())
                .unwrap_or(false)
        {
            node.with_status(VDFS_STATUS_DISABLED)
                .with_enabled(false)
                .with_description("当前会话已有对话历史，不能更换工作目录（如需换目录请新建会话）")
        } else {
            node.with_status(VDFS_STATUS_ACTIVE)
                .with_description("会话的工作目录（决定文件工具的作用范围）")
        }
    }

    /// 执行风险等级：low / medium / high 三档子选项
    fn risk_option(&self, current: Option<String>) -> OptionNode {
        let cur = current.unwrap_or_else(|| "medium".to_string());
        let children = vec![
            OptionNode::session_state("risk_level:low", "低风险", "risk_level", json!("low"))
                .with_description("仅自动执行低风险工具；中/高风险需审批"),
            OptionNode::session_state("risk_level:medium", "中风险", "risk_level", json!("medium"))
                .with_description("中风险及以下自动执行；高风险需审批"),
            OptionNode::session_state("risk_level:high", "高风险", "risk_level", json!("high"))
                .with_description("所有工具自动执行（含高风险）"),
        ];
        let label = match cur.as_str() {
            "low" => "低风险",
            "high" => "高风险",
            _ => "中风险",
        };
        OptionNode::sub("risk_level", "执行风险", children)
            .with_icon("risk")
            .with_order(ORDER_RISK)
            .with_value_label(&cur, label)
            .with_description("低于该等级的工具需用户审批")
    }

    /// 运行模式：interactive / auto 两档子选项
    fn mode_option(&self, current: Option<String>) -> OptionNode {
        let cur = current.unwrap_or_else(|| "interactive".to_string());
        let children = vec![
            OptionNode::session_state("mode:interactive", "交互", "mode", json!("interactive"))
                .with_description("需审批/需交互的工具在会话流中显示卡片，等待用户响应"),
            OptionNode::session_state("mode:auto", "自动", "mode", json!("auto"))
                .with_description("无人值守：工具失败返回友好错误让模型自行继续"),
        ];
        let label = if cur == "auto" { "自动" } else { "交互" };
        OptionNode::sub("mode", "运行模式", children)
            .with_icon("run-mode")
            .with_order(ORDER_MODE)
            .with_value_label(&cur, label)
            .with_description("工具失败时是否阻塞模型继续")
    }

    /// 心跳任务：自动化表单（复用详情表单方言 [`DetailDefinition`]）。
    ///
    /// `enabled` 开关与三项**基础设置**（空闲间隔 / 任务提示词 / 携带历史）
    /// **恒可见**：基础设置不使用 `visible_when` 门控——未启用时用户同样能
    /// 看到并可预先填写，开关与参数一次保存即生效（`option` 绑定保存全部字段，
    /// 关闭开关也不会丢失已填参数）。
    ///
    /// 「立即执行一次」**已取消**（2026-09-18）：它原是一个 `invoke` 类型的独立
    /// 命令选项（图标 `play`，点了直接调 `session/heartbeat/trigger`），但它的作用
    /// 与「在输入框里直接发一条消息」完全重复——心跳的实质就是往会话发一轮提示词，
    /// 用户想立刻做一次，直接在输入框发即可。留着它等于给同一件事两个入口，
    /// 且按钮那个还绕开了对话本身。
    fn heartbeat_option(&self, raw: Option<Value>) -> OptionNode {
        let enabled = raw
            .as_ref()
            .and_then(|v| v.get("enabled"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let interval = raw
            .as_ref()
            .and_then(|v| v.get("interval_seconds"))
            .and_then(|v| v.as_i64())
            .filter(|v| *v > 0)
            .unwrap_or(DEFAULT_HEARTBEAT_INTERVAL);
        let prompt = raw
            .as_ref()
            .and_then(|v| v.get("prompt"))
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let include_history = raw
            .as_ref()
            .and_then(|v| v.get("include_history"))
            .and_then(|v| v.as_bool())
            .unwrap_or(true);

        let definition = DetailDefinition {
            // option 绑定：预填自节点 data、保存回节点 action（与 VDFS 详情表单同一方言）
            binding: "option".to_string(),
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
        };

        OptionNode {
            form: Some(definition),
            // 表单字段值写入 metadata.heartbeat（bind 点路径）
            action: Some(OptionAction::session_state_bind("metadata.heartbeat")),
            data: Some(json!({
                "enabled": enabled,
                "interval_seconds": interval,
                "prompt": prompt,
                "include_history": include_history,
            })),
            status: if enabled {
                VDFS_STATUS_ACTIVE.to_string()
            } else {
                VDFS_STATUS_DISABLED.to_string()
            },
            ..OptionNode::new("heartbeat", "心跳任务", OptionType::Form)
        }
        .with_icon("heartbeat")
        .with_order(ORDER_HEARTBEAT)
        .with_value_label(
            if enabled { "on" } else { "off" },
            if enabled { "已开启" } else { "未开启" },
        )
        .with_description("会话空闲时自动触发一次对话（定时任务）")
    }
}

#[cfg(test)]
#[path = "options.test.rs"]
mod tests;
