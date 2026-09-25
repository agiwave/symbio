//! agent_run —— 委托型工具：把任务交给另一个已安装的 agent 目录（子智能体）执行。
//!
//! ## 架构定位：普通工具，零特殊化
//!
//! agent_run 与 file_read / shell 等工具完全同构：
//! - 经 `traverse(available_tools)` 注册（Capability 机制，见 `super::plugin`）；
//! - 子会话过程经**执行期出口** `ExecEventSink`（由 ctx 注入，见 `symbio_core::exec`）
//!   转播到父视图，前端按普通流式工具渲染——工具**不返回通道**；
//! - 子会话内工具需要用户审批时，产出 `user_prompt(WaitingUserAction)`——与
//!   confirm 类工具走**同一套**审批机制：本工具把子会话的审批**转成载荷**
//!   （`failure_kind` + `prompt`）回传，节点由 `session/tool_executor` 以统一 id
//!   构造（工具不得自造节点身份，见 `tool_executor::PendingPrompt`）；
//! - 无注册表、无唤醒队列、无前端专属概念。"子会话"只是本工具的内部实现：
//!   一个由 session 插件托管、按 `metadata.parent_session_id` 归档到父会话
//!   `sessions/` 子目录的普通会话（归属机制见 `session/store/file`）。
//!
//! ## 执行流程（三种调用形态）
//!
//! 首次调用（args: `agent_id` / `prompt` / `working_dir?` / `session_id?`）：
//! 1. 校验目标 agent 目录存在、`working_dir` 合法；
//! 2. `session/update` 落子会话元数据（`parent_session_id` → 存储归档父会话
//!    子目录、不进用户会话列表）；
//! 3. 路由 `session/chat/send`（统一编排入口：collect → traverse 重新装配工具，
//!    子会话与顶层会话走同一条链路；该路由**立刻返回**，本轮在后台推进）；
//! 4. 订阅子会话的实时面（**一条**：`event_bus` + `vdfs/watch <会话地址>`——
//!    消息流式与会话运行态都在上面，见 ADR-025），把子会话过程经出口转播到
//!    父视图（节点锚定到本工具调用之下）；
//! 5. 子会话完成（idle 无待审批）→ 工具结果 = 最终文本（末尾附
//!    `[subagent_session_id]`，供 LLM 后续续会话）；等待审批 → 工具以待审批
//!    **载荷**结束（`prompt.args` 即本工具的续跑参数）。
//!
//! 续会话（args 另带 `session_id`，LLM 携带上次结果中的子会话 id）：
//! 存在性校验后按 3-5 把 prompt 作为该子会话的下一轮消息——委托对话上下文
//! 自然延续，LLM 无需转述历史。
//!
//! 恢复调用（用户在父会话审批后，`session/resume` 按普通工具恢复机制重执行
//! 本工具，args 追加 `session_id` / `target_id` / `approved`）：
//! 路由 `session/chat/send` 的 resume 分支到子会话（在子会话内恢复被审批的
//! 工具），随后同 4-5。父子关系始终由子会话存储元数据承载，无进程内状态。

use super::store::AgentDirStore;
use crate::symbio_core::schemas::session::chat_message::{message_of_node, SEG_MESSAGES};
use crate::symbio_core::schemas::session::chat_message::{
    ChatMessage, MessageContent, MessageRole, MessageStatus, MessageType, ResumeAction,
    ResumeRequest,
};
use crate::symbio_core::schemas::session::session_chat;
use crate::symbio_core::{register_subscriber, unregister_subscriber};
use crate::symbio_core::{
    vdfs_change_of, VdfsContent, VdfsNode, VdfsProvider, VdfsRequest, VDFS_STATUS_WORKING,
};
use crate::symbio_core::{vdfs_context, VdfsError};
use crate::symbio_core::{
    ExecAbortSignal, ExecEnv, ExecEventSink, Plugin, PluginError, PluginFrame, PluginInvokeRequest,
    PluginInvokeRequestExt, MODE, PATH, PLUGIN_SESSION, PROVIDER_ID, RISK_LEVEL, SESSION_CHAT_SEND,
    SESSION_ID, TOOL_CALL_ID, VDFS_ROOT, VDFS_UNWATCH, VDFS_WATCH,
};
use serde_json::{json, Value};
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

// ── 本工具的身份约定（续跑参数经**载荷**回传，由 tool_executor 落到 meta.prompt）──
/// 工具名。**两处必须同源**：`CapabilityMeta.name`（LLM 看到的短名）与待审批载荷
/// 里的 `prompt.tool_name`（`session/resume` 据此决定重执行哪个工具）——只改一处
/// 会静默断掉审批续跑，故收敛为一个常量。
const NAME: &str = "agent_run";
/// 子会话元数据键：父会话 id（session 存储归档依据）
const KEY_PARENT_SESSION_ID: &str = "parent_session_id";

/// agent_run 能力：把任务委托给指定 agent 目录（子智能体）。
pub struct AgentRunCapability {
    /// 子会话默认 workdir（构造时解析：父 ctx > 无）
    workdir: Option<String>,
    /// agent 目录存储根 = **本插件自己的目录**（构造时由插件实例传入）
    ///
    /// 能力侧同样不认识全局布局：目录由插件给，这里只透传给 [`AgentDirStore`]。
    agent_dir_root: std::path::PathBuf,
    /// 工具描述中的可用 agent 清单（供 LLM 选择 agent_id）
    agent_dirs_brief: String,
    /// 插件间路由入口（composite 容器弱引用，构造时由插件实例捕获）
    router: Option<std::sync::Weak<dyn Plugin>>,
}

impl AgentRunCapability {
    pub fn new(
        workdir: Option<String>,
        agent_dir_root: std::path::PathBuf,
        agent_dirs_brief: String,
        router: Option<std::sync::Weak<dyn Plugin>>,
    ) -> Arc<Self> {
        Arc::new(Self {
            workdir,
            agent_dir_root,
            agent_dirs_brief,
            router,
        })
    }
}

#[async_trait::async_trait]
impl crate::symbio_core::Capability for AgentRunCapability {
    fn meta(&self) -> crate::symbio_core::CapabilityMeta {
        crate::symbio_core::CapabilityMeta {
            name: NAME.to_string(),
            context_retention: None,
            description: format!(
                "将任务委托给一个独立的子智能体会话：在其独立会话中执行任务，\
                 完成后返回其最终答复；子智能体拥有自己的提示词与工具集。\n\n\
                 ## 智能体选择\n\n\
                 `agent_id` 可选：省略时默认使用当前会话正在使用的智能体\
                 （当它落在本插件目录下的可用列表里时）；\
                 否则子会话以无智能体的纯对话模式运行。\n\n\
                 ## 可用智能体列表\n\n{}\n\n\
                 ## 工作目录\n\n\
                 可选参数 `working_dir` 指定子智能体的工作目录：绝对路径、支持 `~` 展开、\
                 禁止包含 `..`；省略时沿用当前会话的工作目录。",
                self.agent_dirs_brief
            ),
            input_schema: json!({
                "type": "object",
                "properties": {
                    "agent_id": {
                        "type": "string",
                        "description": "可选。要执行的智能体 ID（agent id，见可用智能体列表）；\
                                省略则默认使用当前会话正在使用的智能体"
                    },
                    "prompt": {
                        "type": "string",
                        "description": "给智能体的清晰详细的提示或任务描述"
                    },
                    "working_dir": {
                        "type": "string",
                        "description": "可选。子智能体的工作目录（绝对路径），省略则继承当前会话的工作目录。\
                                支持 `~` 展开（如 `~/work/proj`），禁止包含 `..`。\
                                示例：`/tmp/proj`、`C:\\Users\\alice\\projects\\myproj`"
                    },
                    "session_id": {
                        "type": "string",
                        "description": "可选。继续之前的委托对话：传入上次工具结果末尾的 \
                                [subagent_session_id]，本次 prompt 将作为该对话的下一轮，\
                                保留全部上下文；省略则新开一个子会话。"
                    }
                },
                "required": ["prompt"]
            }),
            keywords: Vec::new(),
            category: Some(crate::symbio_core::CapabilityCategory::Chat),
            examples: Some(vec![
                "agent_id='<agent-id>', prompt='分析这个项目的架构'".to_string()
            ]),
        }
    }

    async fn execute(
        &self,
        args: Value,
        env: &ExecEnv,
        ctx: Arc<dyn PluginInvokeRequest>,
    ) -> Result<Value, PluginError> {
        #[derive(serde::Deserialize, Clone)]
        struct RunRequest {
            /// 可选：目标智能体 id（agent id）。省略 → 默认用当前会话的智能体
            ///（ctx[AGENT_ID]）；两者皆空 → 子会话以无智能体的纯对话模式运行。
            #[serde(default)]
            agent_id: Option<String>,
            /// 首次调用必填；恢复调用（审批后续跑）可缺省
            #[serde(default)]
            prompt: String,
            /// 子智能体的工作目录（绝对路径）。None = 继承父会话工作目录。
            #[serde(default)]
            working_dir: Option<String>,
            // ── 以下为续会话 / 恢复调用字段 ──
            /// 续会话（LLM 传：继续之前的委托对话）或审批恢复（resume 机制
            /// 透传）时的子会话 id；省略则新开子会话
            #[serde(default)]
            session_id: Option<String>,
            /// 子会话内待恢复节点 id（仅审批恢复时由 resume 机制注入）
            #[serde(default)]
            target_id: Option<String>,
            /// 用户审批结果（session/resume 的 Approve 分支注入）
            #[serde(default)]
            approved: Option<bool>,
        }
        let req: RunRequest = serde_json::from_value(args).map_err(|e| {
            PluginError::ValidationError(format!("参数反序列化失败，请核对契约类型或 Schema: {e}"))
        })?;

        if ctx.get(TOOL_CALL_ID).is_none() {
            return Err(PluginError::ValidationError(
                "agent_run 需要 tool_call_id 上下文".to_string(),
            ));
        }
        let parent_session_id = ctx.get(SESSION_ID).unwrap_or_default();
        if parent_session_id.is_empty() {
            return Err(PluginError::ValidationError(
                "agent_run 需要会话上下文（SESSION_ID）".to_string(),
            ));
        }
        let parent = self
            .router
            .as_ref()
            .and_then(|w| w.upgrade())
            .ok_or_else(|| {
                PluginError::InternalError(
                    "agent_run 无法获取路由入口（插件构造时未捕获 PARENT 引用）".to_string(),
                )
            })?;

        // ── 解析目标智能体：显式参数 > 当前会话绑定的智能体 > None（纯对话子会话）──
        //
        // ⚠️ **默认值必须在本作用域的 store 里可解释**（与 `forward_to_sub_agent`
        // 清空 `AGENT_ID` 是同一条规则：跨作用域不继承「选中」）。
        // 本能力的 store 是 `<本插件目录>/agent`：
        //  - 系统实例的 store = `{homedir}/agent`，与会话选中的 agent **同域** ⇒ 默认照常生效；
        //  - 子树实例（分形）的 store = `<agentdir>/agent`，而 `ctx[AGENT_ID]` 是**外层**
        //    作用域的选中 ⇒ 拿它当默认必然查不到，令 LLM 收到一条它从未传过的 id 的报错。
        // 故默认值先过一遍**本作用域**的 store：不在我的目录里 ⇒ 我这一层没有默认。
        // **显式** `agent_id` 不受此限 —— 那是调用方的指令，仍在下面照旧对照 store 校验。
        let store = AgentDirStore::new(&self.agent_dir_root);
        let current_agent = ctx
            .get(crate::symbio_core::AGENT_ID)
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .filter(|id| store.get(id).is_some());
        let agent_id: Option<String> = req
            .agent_id
            .clone()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .or(current_agent);

        // ── 解析有效工作目录（显式参数 > 父会话继承），并校验目标 agent 目录存在 ──
        // 绝不静默降级：agent 目录缺失 / working_dir 非法直接报错回传 LLM。
        let effective_workdir: Option<String> = match req.working_dir.as_deref() {
            Some(provided) => Some(validate_working_dir(provided)?),
            None => self.workdir.clone(),
        };
        if let Some(aid) = &agent_id {
            if store.get(aid).is_none() {
                return Err(PluginError::NotFound(format!(
                    "目标智能体 '{aid}' 不存在，无法委托任务。请检查 agent_id 是否正确。"
                )));
            }
        }

        // ── 子会话 id + 续跑请求 + 首条消息（三者按调用形态确定）──
        // 三种调用形态：审批恢复（session_id + target_id）/ 续会话（仅
        // session_id，LLM 携带上次结果的子会话 id 继续对话）/ 新建（都省略）。
        let user_message = || ChatMessage {
            id: uuid::Uuid::new_v4().as_simple().to_string(),
            role: Some(MessageRole::User),
            msg_type: Some(MessageType::Text),
            content: Some(MessageContent::Text(req.prompt.clone())),
            ..Default::default()
        };
        let (child_session_id, resume_req, first_message): (
            String,
            Option<ResumeRequest>,
            Option<ChatMessage>,
        ) = match (req.session_id.as_deref(), req.target_id.as_deref()) {
            (Some(sid), Some(tid)) if !sid.is_empty() && !tid.is_empty() => {
                // 恢复调用：在子会话内恢复被审批的工具（普通 resume 语义透传）。
                let action = if req.approved == Some(true) {
                    ResumeAction::Approve
                } else {
                    ResumeAction::Retry
                };
                (
                    sid.to_string(),
                    Some(ResumeRequest {
                        target_id: tid.to_string(),
                        action,
                        args: None,
                        reason: None,
                        answer: None,
                    }),
                    None,
                )
            }
            (Some(sid), None) if !sid.is_empty() => {
                // 续会话：prompt 作为该子会话的下一轮用户消息，上下文自然延续。
                validate_subsession_exists(parent.clone(), &ctx, sid).await?;
                (sid.to_string(), None, Some(user_message()))
            }
            _ => {
                // 首次调用：创建子会话并落元数据。
                // `parent_session_id` 由 session 存储解释为归档位置
                // （父会话目录的 sessions/ 子目录）与列表过滤依据；
                // `agent_id` / `workdir` 供 chat 编排回退链与详情展示。
                let sid = uuid::Uuid::new_v4().to_string();
                if let Err(e) = register_subsession(
                    parent.clone(),
                    &ctx,
                    &sid,
                    &json!({
                        KEY_PARENT_SESSION_ID: parent_session_id,
                        "agent_id": agent_id,
                        "workdir": effective_workdir,
                        "title": format!("↳ {}", agent_id.as_deref().unwrap_or("子智能体")),
                    }),
                )
                .await
                {
                    crate::plugin_warn!("agent", "登记子会话元数据失败（不影响委托执行）: {}", e);
                }
                (sid, None, Some(user_message()))
            }
        };

        // ── 订阅子会话的实时面（先于 chat/send，避免丢首帧）──
        //
        // 与 CLI / 前端**同构**：`event_bus/subscribe` 收件 + `vdfs/watch <会话地址>`
        // 开闸，**两步缺一不可**（只订阅 = 永不响的频道；只 watch = 没有收件人）。
        // 会话域的实时面**只有这一条**（ADR-025）：
        //
        // - 消息流式 = `<sid>/message/<mid>` 的 `updated` + `delta`；
        // - 会话运行态 = `<sid>` 的 `updated`——**本轮结束的唯一判据**（一轮里根
        //   Turn 会多次定格，只有会话节点离开 `working` 才是整轮结束）。
        //
        // 两者**不靠到达顺序**协作（那是把数据的属性当成传输的属性）：本桥的收尾
        // 判据是会话节点的 `status`，而它是**回读**来的——回读永远最新且幂等。
        let (bus_tx, bus_rx) = tokio::sync::mpsc::channel::<PluginFrame>(4096);
        let conn_id = format!("agent-run-{child_session_id}");
        register_subscriber(conn_id.clone(), bus_tx);
        let session_addr = match session_vdfs_addr(&parent, &ctx, &child_session_id).await {
            Ok(addr) => addr,
            Err(e) => {
                unregister_subscriber(&conn_id);
                return Err(e);
            }
        };
        vdfs_watch(&parent, &ctx, &session_addr, true).await;

        // ── 路由 session/chat/send（统一编排入口）──
        // mode / risk_level / provider_id 随请求显式继承（resolve_session_params
        // 的回退链是 req > metadata > 默认，ctx 键会被覆盖，必须走 req 字段）。
        let send_ctx = ctx.fork();
        send_ctx.set(PATH, SESSION_CHAT_SEND.to_string());
        send_ctx.set(SESSION_ID, child_session_id.clone());
        let _ = send_ctx.set_payload(session_chat::Request {
            session_id: Some(child_session_id.clone()),
            agent_id: agent_id.clone(),
            message: first_message,
            provider_id: ctx.get(PROVIDER_ID),
            include_history: None,
            mode: ctx.get(MODE),
            risk_level: ctx.get(RISK_LEVEL),
            resume: resume_req,
        });

        if let Err(e) = parent.clone().route(send_ctx).await {
            // 委托失败：清理订阅，错误直接作为工具结果回传
            vdfs_watch(&parent, &ctx, &session_addr, false).await;
            unregister_subscriber(&conn_id);
            return Err(e);
        }

        // ── 转播子会话过程 → 父视图（执行期出口）──
        //
        // 出口由**执行期环境**给出（`route()` 直连调用时退化为 `Null`：同一份代码
        // 不发增量）；中止同样来自环境——出口没有可关闭的通道，因此「对端消失」
        // 不再是可依赖的信号，中止必须写成一条 select 臂。
        //
        // 转播任务**spawn**（不是内联 await）：① 与后台推进的子会话并发排水，
        // 避免订阅通道积压；② 父会话中止时本 future 会被 drop，而 spawn 出去的
        // 任务仍会跑到清理（摘除订阅），不留订阅泄漏。
        let sink = env.sink().clone();
        let abort = env.abort().clone();
        let relay = tokio::spawn(relay_bridge(
            RelayBridge {
                bus_rx,
                // `get_vfs_provider` 消费 `Arc<Self>`（签名如此），故先 clone
                provider: parent.clone().get_vfs_provider(),
                router: parent.clone(),
                invoke_ctx: ctx.clone(),
                session_addr,
                rel_base: format!("{PLUGIN_SESSION}/{child_session_id}"),
                conn_id,
                child_session_id,
                agent_id,
                tool_call_id: ctx.get(TOOL_CALL_ID).unwrap_or_default(),
            },
            sink,
            abort,
        ));
        let outcome = relay
            .await
            .unwrap_or_else(|e| RelayOutcome::Failed(format!("转播任务异常终止: {e}")));

        match outcome {
            RelayOutcome::Done(text) => Ok(json!({ "content": text })),
            // 待用户动作：只回传**意图载荷**（`failure_kind` + `prompt`），
            // 节点由 `session/tool_executor` 以统一 id 构造。
            RelayOutcome::Pending {
                text,
                prompt,
                failure_kind,
            } => Ok(json!({
                "content": text,
                "success": false,
                "failure_kind": failure_kind,
                "prompt": prompt,
            })),
            RelayOutcome::Failed(e) => Err(PluginError::InternalError(e)),
        }
    }
}

/// 子会话转播的**结局**——转播桥的返回值，也是「子会话跑完了什么」的唯一表达。
///
/// 收口前这些结局以「通道帧」回传（最终文本塞进 `{"content": ...}` 哨兵帧、
/// 错误走 `PluginFrame::Error`、待审批靠节点冒泡），消费端再从帧面反推语义。
/// 现在它们是三个具名变体，消费端（`tool_executor`）直接按变体分派。
enum RelayOutcome {
    /// 子会话本轮结束，产出最终答复文本。
    Done(String),
    /// 子会话内出现待用户动作（审批 / 提问）：本工具据此以「待用户响应」结束。
    Pending {
        /// 卡片正文
        text: String,
        /// 本工具的续跑参数（`session/resume` 重执行 agent_run 时读它）
        prompt: Value,
        /// 子会话给出的 `failure_kind`（`needs_approval` / `needs_interaction`）
        failure_kind: String,
    },
    /// 子会话以错误结束（会话节点 `attributes.error`）。
    Failed(String),
}

/// 校验 working_dir：绝对路径、拒绝 `..` 段、支持 `~` 展开。
fn validate_working_dir(provided: &str) -> Result<String, PluginError> {
    let trimmed = provided.trim();
    if trimmed.is_empty() {
        return Err(PluginError::ValidationError(
            "working_dir 不能为空".to_string(),
        ));
    }
    if trimmed.split(['/', '\\']).any(|seg| seg == "..") {
        return Err(PluginError::ValidationError(format!(
            "Invalid working_dir '{provided}': `..` components are forbidden."
        )));
    }
    let expanded = crate::symbio_core::expand_tilde_path(std::path::Path::new(trimmed));
    if !expanded.is_absolute() {
        return Err(PluginError::ValidationError(format!(
            "Invalid working_dir '{provided}': must be an absolute path. \
             Examples: `/tmp/proj`, `C:\\repo\\myproj`, `~/work`."
        )));
    }
    Ok(expanded.to_string_lossy().to_string())
}

/// 续会话存在性轻校验：该会话必须在**本作用域**的会话存储里、且有消息记录。
///
/// `PersistentChatSession` 是惰性句柄（`session/open` 永远成功），无法直接探测
/// 文件存在性；该校验防止 LLM 拼错 session_id 时 `get_or_create_session`
/// 静默创建出一个顶层孤儿会话。
///
/// ## 探针为什么走 VDFS 纯接口（而不是一条专用协议）
///
/// 会话的**唯一**读入口是 VDFS（`read(<根>/session/<sid>)`），存在性因此也该由
/// 同一层回答。曾用的 `session/get_messages` 专用路由为了回答「在不在」而读回
/// 整份历史，已于 2026-09-23 退役——它当时也不是「会话的读接口」，
/// 见 `docs/archive/legacy-route-migration.md` §3.4.1。
///
/// 取的是**本作用域容器**的 VDFS 视图（[`Plugin::get_vfs_provider`]——core 的查询
/// 接口）：容器把子插件按**实例名**列为子目录，而 `route` 分发用的是同一个键，
/// 因此 `session/<sid>` 命中的正是 `session/chat/send` 将要写入的那个会话插件实例
/// ——子智能体分形子树里同样成立。用到的 `vdfs_context` / `VdfsProvider` / `VdfsNode`
/// 全在 `symbio_core`：线路形状（`vdfs/*` 的信封）不进本文件，也不新增插件间依赖。
async fn validate_subsession_exists(
    parent: Arc<dyn Plugin>,
    ctx: &Arc<dyn PluginInvokeRequest>,
    session_id: &str,
) -> Result<(), PluginError> {
    let provider = parent.get_vfs_provider().ok_or_else(|| {
        PluginError::InternalError(
            "agent_run 无法取得本作用域的 VDFS 视图（容器未暴露 provider）".to_string(),
        )
    })?;
    let addr = format!("{PLUGIN_SESSION}/{session_id}");
    let exists = match provider
        .dispatch(&vdfs_context(ctx), &addr, VdfsRequest::Stat)
        .await
    {
        // 节点存在且已有消息 ⇒ 续会话合法。`message_count` 由会话 provider 投影
        // （落库 ∪ 在途），与「消息列表非空」是同一判据。
        Ok(resp) => match resp.into_stat() {
            Some(node) => {
                node.attributes
                    .get("message_count")
                    .and_then(Value::as_u64)
                    .unwrap_or(0)
                    > 0
            }
            // dispatch 成功但响应形状不符：按「节点不存在」处理（与旧缺省实现一致）
            None => false,
        },
        // 只把「节点不存在」读作「会话不存在」；存储故障等照实上抛——
        // 否则一次 IO 抖动会被讲成「LLM 编造了 id」。
        Err(VdfsError::NotFound(_)) => false,
        Err(e) => return Err(e.into()),
    };
    if !exists {
        return Err(PluginError::NotFound(format!(
            "子会话 '{session_id}' 不存在或无消息记录，无法继续该对话。\
             请省略 session_id 参数以新开一个委托会话。"
        )));
    }
    Ok(())
}

/// 登记子会话：经**进程内 VDFS 纯接口**在 `<挂载名>/<sid>` 上写一次。
///
/// 带 `create` 意图 ⇒ 不存在则**就地创建**，且地址末段就是会话 id
/// （[`VdfsProvider::write`] 的「两种目标形态」表）——子会话 id 因此仍由本文件
/// 决定（它是 uuid，会出现在 `agent_run` 的 `session_id` 参数里）。
///
/// ## 为什么不再 `route` 一条路由
///
/// 这里曾经 `fork` 一个 ctx、把 `PATH` 设成 `session/update` 再 `route` 回去。
/// 那条专用路由已于 2026-09-23 退役：它只是「写会话 metadata」的第二份实现，
/// 而 VDFS 的 `write` 本来就覆盖这件事（同一个 provider、同一份浅合并）。
/// 绕路由还为一次纯数据写入白搭了一层 invoke 信封。
///
/// 取 provider 的方式与 [`validate_subsession_exists`] 同款（先例在那里）：
/// 本作用域容器的 VDFS 视图里，`session` 就是 `session/chat/send` 将要写入的
/// 那个插件实例，子智能体分形子树里同样成立。
async fn register_subsession(
    parent: Arc<dyn Plugin>,
    ctx: &Arc<dyn PluginInvokeRequest>,
    session_id: &str,
    metadata: &Value,
) -> Result<(), PluginError> {
    let provider = parent.get_vfs_provider().ok_or_else(|| {
        PluginError::InternalError(
            "agent_run 无法取得本作用域的 VDFS 视图（容器未暴露 provider）".to_string(),
        )
    })?;
    let addr = format!("{PLUGIN_SESSION}/{session_id}");
    let body = json!({ "metadata": metadata }).to_string();
    provider
        .dispatch(
            &vdfs_context(ctx),
            &addr,
            VdfsRequest::Write {
                content: VdfsContent::text(body).with_create(),
            },
        )
        .await?;
    Ok(())
}

// 实时面的**解包**不在这里：`vdfs_change_of` 走 `symbio_core` 的公共入口
// （信封形状是跨模块契约，本文件曾是三份手写副本之一）。这里只留「解出来之后怎么用」。

/// 一条变更是否落在**某条消息节点自身**的地址上（`<sid>/message/<mid>`）。
///
/// 判据是**地址形状**，不是「路径等于某个目录」：会话容器的统一形状是
/// `<sid>/<集合段>/<项 id>`，本桥只认「消息」这一段的**项**——集合目录本身
/// （`<sid>/message`）、更深的层级、以及别的集合段（记忆 / 工作目录 / 子会话）
/// 都不认。这样新增一类集合时，本桥的判据不需要改。
fn is_message_item_addr(msg_prefix: &str, path: &str) -> bool {
    match path.strip_prefix(msg_prefix) {
        Some(rest) => !rest.is_empty() && !rest.contains('/'),
        None => false,
    }
}

/// 转播桥的入参。
///
/// 成组而不是 11 个位置参数：参数表里有一半是「同一件事的两半」（`session_addr`
/// 与 `rel_base` 是**同一个地址**的展示口径与 provider 口径），摊平之后既看不出
/// 这层关系，也容易在调用点把两个口径传反。
struct RelayBridge {
    /// VDFS 变更的收件端（`event_bus` 那一路）
    bus_rx: tokio::sync::mpsc::Receiver<PluginFrame>,
    /// 进程内 VDFS 视图：回读消息节点（`stat` / `read`）
    provider: Option<Arc<dyn VdfsProvider>>,
    /// 回读用的请求上下文（`vdfs_context` 的原料）
    invoke_ctx: Arc<dyn PluginInvokeRequest>,
    /// 摘除订阅要用（`vdfs/unwatch` 与 `unregister_subscriber` 都要它）
    router: Arc<dyn Plugin>,
    /// 子会话的**展示地址**（`<根>/session/<sid>`）——判定变更归属
    session_addr: String,
    /// 子会话在 provider 子树内的地址（`session/<sid>`）——回读用
    rel_base: String,
    conn_id: String,
    child_session_id: String,
    agent_id: Option<String>,
    tool_call_id: String,
}

/// 子会话转播任务：把子会话的**实时面**汇入**执行期出口**。
///
/// ## 一条通道，两类地址（与 `cli/src/client.rs` 同构）
///
/// 会话域的实时面只有一条（ADR-025），落点恒为**被变更节点自身的地址**：
///
/// - `<sid>` = **会话运行态**（容器节点）——本轮结束的唯一判据（一轮里根 Turn 会
///   多次定格，只有会话节点离开 `working` 才是整轮结束）。变更随 `data` 带全量节点
///   视图（与 `stat` 同源构造）⇒ 零回读；无载荷 ⇒ 回读兜底。
/// - `<sid>/<集合段>/<项 id>` = **集合项**，对消息即 `<sid>/message/<mid>`——`data`
///   就是那条 `ChatMessage`，语义在字段上：`delta` 追加 / `content` 替换 /
///   `status = removed` 移除。全量帧自给自足；窄增量帧撞上未知身份 ⇒ 回读基线。
///
/// 于是本桥的读策略是**按字段省**：`delta` 且该 id 已知 ⇒ 一次 I/O 都不做
/// （流式正文的绝大多数帧走这里）；全量帧 ⇒ 直接用载荷；仅「无正文且身份未知」
/// ⇒ `stat` + `read`。
///
/// ## 与「顺序」的关系
///
/// 两类变更**不靠到达顺序**协作。旧版靠「同一条流 + 同一个 `seq` 计数器」推出
/// 「读到不忙 ⇒ 本轮消息帧都已处理」；那条推理的前提（顺序是**传输**属性）已被
/// ADR-025 纠正为「顺序是**节点**属性」（`ChatMessage.seq` 决定显示顺序）。现在
/// 的收尾判据是会话节点的 `status` 本身——回读即得，与谁先到无关。
///
/// ## 本桥是「子会话 → 父视图」的**唯一翻译点**
///
/// 收口前，翻译散在两处：本桥把节点原样转发到工具输出通道，`tool_executor`
/// 再对每一帧做二次分派（user 角色过滤、`parent_id` 锚定、`Assistant → Tool`
/// 角色改写、`Reset`/`Warn` 丢弃）。两处翻译意味着**规则会漂移**，而且
/// `tool_executor` 必须为「工具可以返回事件流」这件事付出一整段帧循环的代价。
/// 现在翻译只剩这里一处，出口下游（转写唯一写入点）只做落地。
///
/// ## 收尾
///
/// 返回 [`RelayOutcome`]：最终答复（= 最后一条**已完成**的助手正文）/
/// 待用户动作载荷 / 子会话错误。优先级与收口前一致——错误 > 待审批 > 文本。
async fn relay_bridge(b: RelayBridge, sink: ExecEventSink, abort: ExecAbortSignal) -> RelayOutcome {
    let RelayBridge {
        mut bus_rx,
        provider,
        invoke_ctx,
        router,
        session_addr,
        rel_base,
        conn_id,
        child_session_id,
        agent_id,
        tool_call_id,
    } = b;
    // 消息变更的落点：信封的 `path` 恒为**那条消息节点自身**的地址
    // （`<sid>/message/<mid>`）。会话是容器、其下是并列的集合，因此本桥按**地址形状**
    // 认人（`<session_addr>/<集合段>/<项 id>`），而不是「目录 + 载荷里的 id」——
    // 后者要求消费端把地址反推回来，且无法推广到第二类集合（任务 / 请求队列）。
    let msg_prefix = format!("{session_addr}/{SEG_MESSAGES}/");

    // 助手正文累积（按消息 id）：最终结果取「最后一条**已完成**的助手正文」而非
    // 「最后处理到的文本」——流式片段或子 agent 的内部独白不能被误当成最终答案。
    let mut text_by_id: HashMap<String, String> = HashMap::new();
    let mut text_order: Vec<String> = Vec::new();
    let mut completed_ids: HashSet<String> = HashSet::new();
    // 已回读过身份的消息 id —— 带 `delta` 的帧据此走**零回读**路径
    let mut known: HashSet<String> = HashSet::new();
    // 最近一次回读到的完整记录（锚定判定要用它的 `parent_id`）
    let mut records: HashMap<String, ChatMessage> = HashMap::new();
    let mut relay_error: Option<String> = None;
    // 子会话内的待用户动作：只**捕获载荷**，不落出口——节点由 `tool_executor`
    // 以统一 id（`result_msg_id`）构造。若此处也落一个（子会话自己的 id），前端
    // 会收到两个 id 但同 parent_id 的审批卡：重复 UI，且 resume 只删得掉一个。
    let mut pending: Option<RelayOutcome> = None;

    loop {
        tokio::select! {
            // 父会话中止 / 本轮收尾（`AbortGuard` 注销时置位）→ 停止转播。
            // 出口没有可关闭的通道，因此中止必须**显式**感知；这条臂取代了
            // 「send 失败 ⇒ 对端已消失」那个隐式信号（它曾把每一次正常收尾
            // 都误读成中止）。
            _ = abort.cancelled() => break,

            // ── VDFS 变更：会话运行态 + 消息（**同一条通道**）──
            maybe = bus_rx.recv() => {
                let Some(frame) = maybe else {
                    // 订阅被摘除：已没有第二条通道可等，直接收尾。
                    break;
                };
                let Some(change) = vdfs_change_of(&frame) else { continue };

                // ① 会话运行态：本轮结束的唯一判据。`data` = 全量节点视图（与
                // `stat` 同源构造，零回读）；无载荷 ⇒ 回读兜底。
                if change.path == session_addr {
                    let node = match change.data {
                        Some(v) => serde_json::from_value::<VdfsNode>(v).ok(),
                        None => stat_node(&provider, &invoke_ctx, &rel_base).await,
                    };
                    let Some(node) = node else { continue; };
                    if node.status != VDFS_STATUS_WORKING {
                        if let Some(err) = node.attributes.get("error").and_then(Value::as_str) {
                            // 子会话以错误结束。错误本身仍是子会话节点的持久状态，
                            // 这里只把它取出来作为本桥的结局（不再经错误帧转播）。
                            relay_error = Some(err.to_string());
                        }
                        break;
                    }
                    continue;
                }

                // ② 消息变更：落点是那条消息**节点自身**的地址
                // （`<session_addr>/message/<mid>`）——集合目录本身、记忆 / 工作目录 /
                // 子会话 / 别的会话 / 更深层级一律不认；`data` 缺失 ⇒ 无载荷，
                // 回读收敛不归本桥管。
                if !is_message_item_addr(&msg_prefix, &change.path) {
                    continue;
                }
                let Some(data) = change.data else { continue };
                let Ok(message) = serde_json::from_value::<ChatMessage>(data) else { continue };
                let mid = message.id.clone();

                // 删除优先：本地索引与父视图同步移除——身份改写、正文累积对已删
                // 节点没有意义。`status = removed` 是消息词汇里的**删除语义**
                // （信封上没有 deleted 取值）。
                if message.status == Some(MessageStatus::Removed) {
                    text_by_id.remove(&mid);
                    text_order.retain(|x| x != &mid);
                    completed_ids.remove(&mid);
                    known.remove(&mid);
                    records.remove(&mid);
                    // 删除按 id 生效（不依赖父子锚点），原样转发。
                    sink.emit(crate::symbio_core::removed_frame(&mid)).await;
                    continue;
                }

                // ③ 纯增量 + 身份已知 ⇒ **零回读**（流式正文的绝大多数帧）
                if let Some(delta) = message.delta.clone() {
                    if known.contains(&mid) {
                        if let Some(buf) = text_by_id.get_mut(&mid) {
                            buf.push_str(&delta);
                        }
                        let mut out = ChatMessage {
                            id: mid.clone(),
                            delta: Some(delta),
                            ..Default::default()
                        };
                        // 锚定：顶层节点（子会话里 `parent_id = None` 的那些）
                        // 挂到本工具调用之下。角色**不**在这里改写——父会话早已按
                        // 首次回读时的完整帧记下了改写后的角色。
                        if records.get(&mid).is_some_and(|r| r.parent_id.is_none()) {
                            out.parent_id = Some(tool_call_id.clone());
                        }
                        sink.emit(out).await;
                        continue;
                    }
                }

                // ④ 全量帧：自给自足——身份 / 正文 / 状态都在载荷里，不再回读。
                //    只有「窄增量帧撞上未知身份」（订阅晚于流开始）或既无 `content`
                //    又无 `delta` 的状态帧才回读基线（历史面 `read` 地址不变）。
                let mut message = if message.content.is_some() {
                    message
                } else {
                    let rel = format!("{rel_base}/{SEG_MESSAGES}/{mid}");
                    let Some(node) = stat_node(&provider, &invoke_ctx, &rel).await else {
                        continue;
                    };
                    let text = read_text(&provider, &invoke_ctx, &rel).await.unwrap_or_default();
                    let Some(m) = message_of_node(&node, text) else { continue };
                    m
                };
                known.insert(mid.clone());
                records.insert(mid.clone(), message.clone());

                // 助手正文：**在锚定之前**累积——锚定会把顶层节点的角色改写成
                // Tool，而"最终答复"的判据是助手正文。
                if message.role == Some(MessageRole::Assistant)
                    && matches!(message.msg_type, Some(MessageType::Text) | None)
                {
                    if !text_by_id.contains_key(&mid) {
                        text_order.push(mid.clone());
                    }
                    text_by_id.insert(
                        mid.clone(),
                        message.content.as_ref().map(|c| c.to_text()).unwrap_or_default(),
                    );
                }
                if matches!(
                    message.status,
                    Some(MessageStatus::Completed)
                        | Some(MessageStatus::Failed)
                        | Some(MessageStatus::Aborted)
                ) {
                    completed_ids.insert(mid.clone());
                }

                // ── 审批/提问冒泡：转成**载荷**，不落节点 ──
                // `prompt.args` 是本工具的续跑参数（`session/resume`
                // 重执行 agent_run 时据此续跑子会话）；`failure_kind`
                // 原样带上（驱动前端审批 UI）。
                if message.msg_type == Some(MessageType::UserPrompt)
                    && message.status == Some(MessageStatus::WaitingUserAction)
                {
                    pending = Some(RelayOutcome::Pending {
                        text: text_by_id.get(&mid).cloned().unwrap_or_default(),
                        prompt: json!({
                            "tool_name": NAME,
                            "args": {
                                "agent_id": agent_id,
                                "session_id": child_session_id,
                                "target_id": message.id,
                            }
                        }),
                        failure_kind: message
                            .meta
                            .as_ref()
                            .and_then(|m| m.get("failure_kind"))
                            .and_then(Value::as_str)
                            .unwrap_or(crate::symbio_core::failure_kind::NEEDS_APPROVAL)
                            .to_string(),
                    });
                    continue;
                }

                // 子会话的委托 prompt（user 消息）不透传：其内容已可见于
                // ToolCall 的请求参数（args.prompt），且 role=user 的临时
                // 节点会在前端获得"编辑"入口（该 id 不在父会话存储中，
                // 操作必然失败）。
                if message.role == Some(MessageRole::User) {
                    continue;
                }

                // ── 锚定到父 ToolCall 之下 ──
                // 子会话的顶层响应节点原 `parent_id = None`，锚定到本工具
                // 调用之下；其角色改为 Tool（工具响应）而非 Assistant，
                // 以符合分型结构 ToolCall(Assistant) → Turn(Tool)，
                // 并能被 flatten 的 find_tool_result 正确识别。
                if message.parent_id.is_none() {
                    message.parent_id = Some(tool_call_id.clone());
                    if message.role == Some(MessageRole::Assistant) {
                        message.role = Some(MessageRole::Tool);
                    }
                }
                sink.emit(message).await;
            }
        }
    }

    // 摘除订阅（与注册严格配对）：先关闸门（引用计数），再从总线摘除收件端
    vdfs_watch(&router, &invoke_ctx, &session_addr, false).await;
    unregister_subscriber(&conn_id);

    // ── 结局判定：错误 > 待审批 > 文本（与收口前逐字一致）──
    if let Some(err) = relay_error {
        return RelayOutcome::Failed(err);
    }
    if let Some(pending) = pending {
        return pending;
    }
    let mut final_result = text_order
        .iter()
        .rev()
        .find(|id| completed_ids.contains(*id))
        .or(text_order.last())
        .and_then(|id| text_by_id.get(id).cloned())
        .unwrap_or_default();
    if final_result.is_empty() {
        final_result = "（子智能体已结束，未产生文本输出）".to_string();
    }
    // 附加子会话 id：LLM 续会话的凭据（下一次调用传 session_id 即继续此对话）。
    final_result.push_str(&format!("\n\n[subagent_session_id: {child_session_id}]"));
    RelayOutcome::Done(final_result)
}

/// 回读一个节点的**视图**（变更不带载荷，状态只能这么拿）。
async fn stat_node(
    provider: &Option<Arc<dyn VdfsProvider>>,
    ctx: &Arc<dyn PluginInvokeRequest>,
    rel: &str,
) -> Option<VdfsNode> {
    provider
        .as_ref()?
        .dispatch(&vdfs_context(ctx), rel, VdfsRequest::Stat)
        .await
        .ok()
        .and_then(|r| r.into_stat())
}

/// 回读一个节点的**正文**。读不到（已删 / 无 provider）返回 `None`。
async fn read_text(
    provider: &Option<Arc<dyn VdfsProvider>>,
    ctx: &Arc<dyn PluginInvokeRequest>,
    rel: &str,
) -> Option<String> {
    let content = provider
        .as_ref()?
        .dispatch(&vdfs_context(ctx), rel, VdfsRequest::Read)
        .await
        .ok()
        .and_then(|r| r.into_read())?;
    content.text
}

/// 子会话在 VDFS 上的**展示地址**（`<根>/session/<sid>`）。
///
/// 两个段各有归属，都不许在这里写死：
/// - **根名**归 vdfs 插件（仓级守卫 S-010 禁止它出现在别处），经 `vdfs/root`
///   取回后当**运行期数据**持有（前端 `schemas/vdfsRoot` 同款做法）；
/// - **挂载段**是容器的分发键——目录名 = 实例名，故取 `PLUGIN_SESSION`。
async fn session_vdfs_addr(
    parent: &Arc<dyn Plugin>,
    ctx: &Arc<dyn PluginInvokeRequest>,
    session_id: &str,
) -> Result<String, PluginError> {
    let c = ctx.fork();
    c.set(PATH, VDFS_ROOT.to_string());
    let resp = parent
        .clone()
        .route(c)
        .await
        .map_err(|e| PluginError::InternalError(format!("vdfs/root 失败: {e}")))?;
    let v: Value = resp
        .get::<Value>()
        .map_err(|e| PluginError::InternalError(format!("vdfs/root 载荷解析失败: {e}")))?;
    let root = v
        .get("path")
        .and_then(Value::as_str)
        .map(|s| s.trim_end_matches('/'))
        .filter(|s| !s.is_empty())
        .ok_or_else(|| {
            PluginError::InternalError(format!("vdfs/root 未返回根地址，无法订阅子会话变更: {v}"))
        })?;
    Ok(format!("{root}/{PLUGIN_SESSION}/{session_id}"))
}

/// 登记 / 摘除一条 VDFS 订阅（与 `vdfs/watch` 的引用计数严格配对）。
///
/// 失败只告警不中断：登记的收益是「能收到中间帧」，而失败的最坏结果是转播
/// 看不到过程（最终文本仍由收尾帧给出）——为此把整个委托判失败不成比例。
///
/// 订阅**只需一个地址**（会话叶子）：`ChangeSubscriptions::notify` 的「相关」判定
/// 是同一子树（自身 / 祖先 / 后代），因此 `<sid>` 的订阅天然覆盖
/// `<sid>/message/<mid>`——一条 `vdfs/watch` 同时收运行态与消息。
async fn vdfs_watch(
    parent: &Arc<dyn Plugin>,
    ctx: &Arc<dyn PluginInvokeRequest>,
    addr: &str,
    subscribe: bool,
) {
    let c = ctx.fork();
    c.set(
        PATH,
        if subscribe { VDFS_WATCH } else { VDFS_UNWATCH }.to_string(),
    );
    let _ = c.set_payload(json!({ "path": addr }));
    if let Err(e) = parent.clone().route(c).await {
        crate::plugin_warn!(
            "agent",
            "agent_run: vdfs/{} 登记失败（{}）: {}",
            if subscribe { "watch" } else { "unwatch" },
            addr,
            e
        );
    }
}

/// 从 AgentDirStore 摘要生成工具描述中的"可用智能体列表"。
pub(crate) fn format_agent_dirs_brief(agent_dirs: &[super::store::AgentDirRecord]) -> String {
    if agent_dirs.is_empty() {
        return "（暂无可用智能体）".to_string();
    }
    agent_dirs
        .iter()
        .map(|r| {
            let desc = if r.manifest.description.is_empty() {
                "(无描述)".to_string()
            } else {
                r.manifest.description.clone()
            };
            format!(
                "- ID: `{}`\n  Name: {}\n  Description: {}",
                r.manifest.id, r.manifest.name, desc
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}
