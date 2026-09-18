/**
 * `meta.prompt` 的数据契约 —— 待用户响应节点的载荷形状
 *
 * 与后端 `ask_user` / 工具确认广播的 `meta.prompt` 对齐（VDFS 只透传，前端只消费）。
 *
 * 两种子形态（判别键 `kind`）：
 * - `question`：向用户提问（`ask_user`），可多问、可多选、含 `Other` 自由输入
 * - `confirm` ：工具执行前的确认，带工具名 / 参数 / 风险等级
 *
 * 本文件是**数据契约层**：零组件知识、零文案。渲染在
 * `components/message/UserPromptNode.vue`。
 */

/** 提问的可选项 */
export interface MessagePromptOption {
  label: string
  description?: string
}

/** 单个问题 */
export interface MessagePromptQuestion {
  id: string
  /** 问题分组标题（可缺省） */
  header?: string
  question: string
  /** 是否多选（缺省 = 单选） */
  multiSelect?: boolean
  options: MessagePromptOption[]
}

/** 工具确认载荷 */
export interface MessagePromptConfirm {
  kind: 'confirm'
  tool_name?: string
  args?: unknown
  risk_level?: string
  description?: string
}

/** 提问载荷 */
export interface MessagePromptQuestions {
  kind: 'question'
  questions: MessagePromptQuestion[]
}

export type MessagePrompt = MessagePromptConfirm | MessagePromptQuestions

/** `Other` 是**约定选项**：勾选后允许自由输入，提交时与普通选项一起上报 */
export const MESSAGE_PROMPT_OTHER = 'Other'

/** 单条问题的作答（提交给后端的形状） */
export interface MessagePromptAnswer {
  question_id: string
  selected: string[]
  custom_input: string | null
}

/** 提问作答的整体载荷（`resume action=answer` 的 `answer` 字段） */
export interface MessagePromptAnswerPayload {
  answers: MessagePromptAnswer[]
}

/**
 * 用户拒绝工具执行时上报的原因（`resume action=reject` 的 `reason` 字段）。
 *
 * 是**协议的一部分**（后端会把它记进会话记录），因此放在契约层而不是渲染组件里。
 */
export const MESSAGE_REJECT_REASON = '用户拒绝'
