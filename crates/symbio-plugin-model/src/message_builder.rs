//! ChatNode → `NativeMessage` 的请求视图构造
//!
//! 负责：
//! - 把会话树拍平成协议所需的 native message（[`flatten_chat_messages`]）
//! - 剔除标记（`meta.exclude_from_context`）与工具结果配对的读侧口径
//!
//! 本文件**不落库**：轮次产生的新消息由会话侧写入点落格，这里只读已有节点。
//!
//! ## 换算分成三个模块
//!
//! | 段 | 住哪 | 只回答 |
//! |---|---|---|
//! | 树形路径 | [`tree::RequestTree`] | 「在哪、归谁」：父子、wire id 配对、谁已被聚合、哪几条思考还留着 |
//! | 消息形状换算 | [`shape::aggregate_run`] / [`shape::plain_root`] | 「有这个东西时长成什么形状」 |
//! | 跨轮降级 | [`degrade`] | 「这个东西缺了怎么办」+ 补完还要扔什么 |
//!
//! 分界的价值在**报错时指得出来**：「把 A 的结果配给 B 的调用」红在 tree，
//! 「配对了但发成错角色」红在 shape，「降级没生效」红在 degrade。合成一坨时
//! 这三类只会笼统报成「请求包不对」，而 275 行里找一格错位正是历史上两次事故
//! （树形路径拼错、降级后恢复轮丢失）的现场。
//!
//! 三段都是**纯函数**（不含 IO），但**不下沉 `symbio_core`**：判据是 D-023 的
//! 依赖方数量——`flatten_chat_messages` 的四个调用点（anthropic / gemini /
//! openai_chat / openai_responses）**同住 model 一个模块**，一个模块依赖的内容
//! 一律留在该模块内；core 只放契约不放实现。第二个模块出现时再谈上提。

mod degrade;
mod shape;
mod tree;

use symbio_core::chat_node::ChatNode;

use super::types::*;

use shape::RunOutput;
use tree::RequestTree;

/// 请求包**剔除标记**的键名（`meta.exclude_from_context = true`）。
///
/// ## 为什么是一个 `meta` 键而不是一个 `NodeKind` 取值
///
/// 「这条节点算不算对话内容」不是**节点类型**的属性，是**消费场景**的属性：
/// 同一条首响节点，对话面板要显示它（用户真的看到过），模型请求包必须剔除它
/// （否则连续两条 `assistant`）。把它做成 `NodeKind` 取值，等于把"某个消费
/// 场景的判断"固化进 schema——下一个场景（审计导出 / 检索索引）还要再加一个类型。
///
/// `meta` 是自由 JSON 且已有先例（`meta.kind = "recall_context"`），加一个键是
/// **数据变化**，不是 schema 变化。
///
/// ## 与对话线投影的分工（看着矛盾，其实各管一条线）
///
/// 本标记管**工作线**（发给模型的包）；`plugins/loop/context/conversation_view.rs`
/// 管**对话线**（判决 / 措辞 / 对话面板），它**不看**这个键。详见那个文件的模块文档。
pub const META_EXCLUDE_FROM_CONTEXT: &str = "exclude_from_context";

/// 该节点是否被标记为「不进请求包」
fn excluded_from_context(m: &ChatNode) -> bool {
    m.meta
        .as_ref()
        .and_then(|meta| meta.get(META_EXCLUDE_FROM_CONTEXT))
        .and_then(|v| v.as_bool())
        .unwrap_or(false)
}

/// 将存储的细粒度 ChatNode 树扁平化，转换为 API 所需的 NativeMessage 列表。
///
/// 树结构与扁平化规则见 [`shape`]（形状那一半）与 [`tree`]（路径那一半）的模块文档；
/// 中断与缺件的处置见 [`degrade`]。本函数只剩编排：建索引 → 按列表序发根级节点 →
/// 清洗。三条剔除规则按原顺序排在这里，顺序是行为的一部分：
///
/// 1. **已被聚合吃掉的子孙**不再单独发出（否则 reasoning / tool_call 参数 / tool_result
///    在请求里重复一遍）；
/// 2. **非根节点**跳过（会在父节点处理时被合并）；
/// 3. **标了「不进请求包」的节点**剔除（C-D3，与 `m.is_compression()` 同一条处置，
///    区别只是判据来自 `meta` 而不是节点类型）。
pub fn flatten_chat_messages(messages: &[ChatNode]) -> Vec<NativeMessage> {
    let tree = RequestTree::of(messages);
    let mut result: Vec<NativeMessage> = Vec::new();

    for m in messages {
        // 1. 已被聚合消费（作为某 Turn 的子节点）的节点不单独发出
        if tree.is_consumed(m.id.as_str()) {
            continue;
        }
        // 2. 非根节点会在其父节点处理时被合并
        if !tree.is_root(m) {
            continue;
        }
        // 3. 「不进请求包」的节点在此剔除（C-D3）——对话面节点（首响 / 进度汇报）是
        //    **给用户看的一句话**，不是工作线的一环：
        //
        //    1. 它会以根级 `role = assistant` 的身份插在用户消息与 Turn 之间，
        //       于是同一轮出现**连续两条 `assistant`**——部分协议直接 400；
        //    2. 就算协议容忍，它也会把"界面上的话"当成"模型说过的话"再喂回去，
        //       同一份信息在两条线上各留一份，越滚越偏。
        if excluded_from_context(m) {
            continue;
        }

        if m.is_run() {
            let RunOutput {
                assistant,
                tool_results,
            } = shape::aggregate_run(m, &tree);
            result.push(assistant);
            result.extend(tool_results);
        } else if m.is_compression() {
            // 上下文压缩节点**不是对话内容**：它是系统对历史的一次整理动作，
            // 只服务于前端呈现与事后审计。发给模型纯属噪音——既占上下文，
            // 又会把"系统整理过上下文"当成一条事实陈述摆进对话序列。
        } else {
            // User / System 等根级内容节点
            result.push(shape::plain_root(m, &tree));
        }
    }

    degrade::clean_request(result)
}

// ChatNode 的**构造**不在本插件：`llm_build_assistant_messages` /
// `llm_build_tool_message` / `TurnStreamChildIds` / `TurnOutput::into_messages`
// 生产上只有 session 一个消费方，已随 D-023 从 `symbio_core::llm::turn` 下沉到
// `plugins/session/message_build.rs`。本文件是**读侧**（存储树 → 请求包），
// 测试用同形状的本地 fixture（理由见 tests 模块文件头）。

#[cfg(test)]
#[path = "message_builder.test.rs"]
mod tests;
