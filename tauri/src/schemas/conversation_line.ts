/**
 * 对话线投影 —— 「用户与助手说过的话」与「工作过程」的分界（**纯函数**）。
 *
 * ## 它是 Rust 侧的**镜像**，不是第二个 owner
 *
 * 规则的唯一 owner 是
 * `symbio/src/plugins/session/context/conversation_view.rs`。本文件是它在前端的
 * 镜像：同一条规则有三个消费方——`triage` / `reply` 的请求上下文、前端「对话」面板、
 * 前端「工作」面板的补集。三处必须给出同一个答案，否则会出现「界面看得到、插件
 * 看不到」的错位；那类 bug 没有错误信号，只表现为「助手答得不对」，排查方向会被
 * 带偏到提示词上。
 *
 * **镜像无法自动守卫**：`protocol-mirror-audit` 守的是常量、枚举取值与字段名，
 * 不是谓词。因此两边各自用单测穷举（Rust 侧 `conversation_view.test.rs`，本文件
 * `conversation_line.spec.ts`），**改规则必须同时改两处**——这是这份镜像的代价，
 * 写在这里，而不是留给下一个人去发现。
 *
 * ## 判据只有两条
 *
 * 1. `role = user` 且是**文本节点**；
 * 2. `role = assistant` 且是**文本节点**。
 *
 * 「文本节点」= `type` 缺省或 `text`：挡掉 `tool_call` / `reasoning` / `turn` /
 * `compression` / `user_prompt` 五类。「角色」挡掉 `tool`（工具结果——可能含几万
 * token 的源码或命令输出）与 `system`（注入的框架文本）。
 *
 * ## 助手说的话有两种位置，**都算对话线**
 *
 * 根级的 assistant 文本是「单独说的一句话」（首响 / 汇报）；`turn` 的**子**文本是
 * **回复正文**——用户真正读到的那段回答。两者都是"助手说过的话"，因此判据
 * **不看 `parent_id`**：位置不参与分界。
 *
 * 曾经按"根级"切，代价有两处（都是真的）：插件看不到助手上一轮答过什么（用户追问
 * 「那 LICENSE 呢？」时，判决只看得到问句、看不到自己的回答），前端「对话」面板
 * 也看不到回答本身（回答在 `turn` 组里）——那就不成其为对话。
 */
import {
  CHAT_ROLE_ASSISTANT,
  CHAT_ROLE_USER,
  MESSAGE_TYPE_TEXT,
  type ChatMessage,
} from './chat_message'

/**
 * 一条节点是否属于对话线（判据见文件头，只有两条）。
 *
 * 与 Rust 侧逐字对齐的细节：`type` **缺省视为文本**（Rust 的
 * `matches!(msg_type, None | Some(Text))`），`role` **缺省不算对话线**
 * （Rust 的 `matches!(role, Some(User) | Some(Assistant))`——`None` 落空）。
 */
export function isConversationNode(node: ChatMessage): boolean {
  const type = node.type ?? MESSAGE_TYPE_TEXT
  if (type !== MESSAGE_TYPE_TEXT) return false
  return node.role === CHAT_ROLE_USER || node.role === CHAT_ROLE_ASSISTANT
}

/**
 * 投影出对话线：按**入参顺序**保留属于对话线的节点（**扁平**，不建树）。
 *
 * 对话面板要的是「按时间读下来的一段对话」，树结构在这里没有意义——回复正文本来
 * 就挂在 `turn` 之下，建树只会把它藏进容器里。
 *
 * 与 Rust 侧的差别只有一处：那边收 `limit` 窗口（它服务的是「一次请求带多少上下文」），
 * 前端要的是**整条对话**，因此这里**不提供** `limit`——给了就会有人用。
 */
export function conversationNodesOf(messages: readonly ChatMessage[]): ChatMessage[] {
  return messages.filter(isConversationNode)
}

/**
 * 工作面板的**根节点**：根级里不属于对话线的那些（`turn` 容器 / 压缩 / 待响应）。
 *
 * ## 为什么不"剪掉 turn 的子正文"
 *
 * `turn` 的子文本（回复正文）同时属于对话线。**不剪**是有意的：工作面板回答的是
 * 「这一轮干了什么」，它就该完整——剪掉正文会让一轮看起来"干完活什么都没说"。
 * 代价是回复正文在两个面板各出现一次（对话面板里是独立的一条，工作面板里在
 * `turn` 组内），那正是"过程"与"说过的话"两个视角各自的完整。
 *
 * 因此本函数只过滤**根级**：子节点一个都不动（剪子节点要重建整棵树、还要维护
 * 父子反向引用，为一个展示重复付这个代价不值得）。
 */
export function workRootsOf(roots: readonly ChatMessage[]): ChatMessage[] {
  return roots.filter((node) => !isConversationNode(node))
}
