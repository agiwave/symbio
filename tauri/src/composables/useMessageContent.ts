/**
 * 消息内容呈现 —— 「正文取出来、按什么格式渲染」的唯一实现
 *
 * 拆出来的理由：同一套「内容 → 显示」规则原先写在 1581 行的 `MessageNode` 里，
 * 而它同时被**用户气泡 / 思考 / 正文 / 工具结果**四处使用。抽到这里之后，
 * 各处渲染器只声明「我要正文」或「我要高亮 JSON」，不再各写一份取值逻辑。
 *
 * 纯函数可单测（见 `composables/__tests__/messageContent.spec.ts`），
 * 组合式函数供组件直接消费（自动跟随节点变化）。
 */

import { computed, type ComputedRef } from 'vue'
import { renderMarkdownCached } from '@/composables/useMarkdown'
import {
  MESSAGE_TYPE_REASONING,
  MESSAGE_TYPE_TEXT,
  MESSAGE_TYPE_TOOL_CALL,
  MESSAGE_TYPE_TURN,
  messageTextOf,
  type ChatMessage,
  type MessageContent,
} from '@/schemas/chat_message'
import {
  MESSAGE_FAILURE_NOTE,
  MESSAGE_PREVIEW_MAX,
  isFailedStatus,
  type MessageFacets,
} from '@/registry/messageTypes'

/**
 * 「取正文」本身不在这里实现 —— 它在契约层 `schemas/chat_message.ts::messageTextOf`
 * （与 `MessageContent` 同处一处）。store 与组件都要取正文，而本文件是**组合式**
 * （依赖 Vue 与 Markdown 渲染器），放在这里会让 store 为了取一串文本而牵进整张
 * 渲染依赖图。这里只消费它。
 */

/**
 * 失败原因（面向用户的可读短消息）。
 *
 * 取值优先级：节点 `error` 字段（后端持久化的失败原因）> `meta.error`（旧路径）
 * > 正文 > 兜底文案。**顺序有语义**：节点字段是权威，正文只是最后的兜底。
 */
export function messageErrorTextOf(node: Pick<ChatMessage, 'error' | 'content' | 'meta'>): string {
  const meta = node.meta as Record<string, unknown> | undefined
  const fromMeta = meta?.error
  return (
    node.error ||
    (typeof fromMeta === 'string' ? fromMeta : '') ||
    messageTextOf(node.content) ||
    MESSAGE_FAILURE_NOTE
  )
}

/** 内容是否为**完整可解析的 JSON 对象/数组**（决定用代码块高亮还是 Markdown 渲染） */
export function messageIsJsonContent(text: string): boolean {
  const s = text.trim()
  if (!s || !/^[[{]/.test(s)) return false
  try {
    JSON.parse(s)
    return true
  } catch {
    return false
  }
}

function escapeHtml(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}

/** JSON 语法着色（对已 pretty-print 的文本逐 token 上色） */
function highlightJsonString(s: string): string {
  const esc = escapeHtml(s)
  return esc.replace(
    /("(\\u[a-zA-Z0-9]{4}|\\[^u]|[^\\"])*"(\s*:)?|\b(true|false|null)\b|-?\d+(?:\.\d*)?(?:[eE][+-]?\d+)?)/g,
    (match) => {
      let cls = 'json-num'
      if (/^"/.test(match)) {
        cls = /:$/.test(match) ? 'json-key' : 'json-str'
      } else if (/true|false/.test(match)) cls = 'json-bool'
      else if (/null/.test(match)) cls = 'json-null'
      return `<span class="${cls}">${match}</span>`
    },
  )
}

/**
 * JSON 高亮 HTML。
 *
 * 解析失败时**退化为整段字符串着色**而不是返回空串：流式期间 JSON 尚不完整是常态，
 * 此时显示原始片段比显示空白有用得多。
 */
export function messageHighlightedOf(content: MessageContent | undefined): string {
  const raw = messageTextOf(content)
  if (!raw) return ''
  let obj: unknown
  try {
    obj = JSON.parse(raw)
  } catch {
    return `<span class="json-str">${escapeHtml(raw)}</span>`
  }
  return highlightJsonString(JSON.stringify(obj, null, 2))
}

/** Markdown 渲染 HTML（工具结果走 JSON 高亮，不走 Markdown） */
export function messageRenderedOf(content: MessageContent | undefined, toolResult: boolean): string {
  if (toolResult) return messageHighlightedOf(content)
  const t = messageTextOf(content)
  return t ? renderMarkdownCached(t) : ''
}

/** 是否按 JSON 代码块呈现（工具结果已由 `messageRenderedOf` 处理，失败体不重复渲染 JSON） */
export function messageRenderAsJsonOf(
  content: MessageContent | undefined,
  toolResult: boolean,
  failed: boolean,
): boolean {
  return !toolResult && !failed && messageIsJsonContent(messageTextOf(content))
}

/**
 * 收起态的单行摘要。
 *
 * 分组节点（Turn / 工具调用）优先取**首个文本或思考子节点**的内容——容器自身
 * 通常没有正文。
 *
 * 但取不到这样的子节点时**回落节点自身正文**，不能直接留白：`ToolCall` 的
 * **请求参数就存在它自己的 `content` 里**（参数不分独立子节点，见
 * `ToolCallNode.vue` 文件头），而它在收到工具结果之前**没有任何 text/reasoning
 * 子节点**。若分组分支只看子节点，"运行中的工具行"就会是一片空白——参数明明
 * 已经逐帧流到本地，界面上却要等到结果子节点到达才第一次显示文字，看起来就像
 * 「工具调用要等跑完才显示」。
 *
 * 用于深层级 / 子步骤默认收起时让用户无需展开即知概要。
 */
export function messageSummaryPreviewOf(node: ChatMessage, grouped: boolean): string {
  let source = ''
  if (grouped) {
    const first = (node.children || []).find(
      (c) => c.type === MESSAGE_TYPE_TEXT || c.type === MESSAGE_TYPE_REASONING,
    )
    source = messageTextOf(first?.content) || messageTextOf(node.content)
  } else {
    source = messageTextOf(node.content)
  }
  const raw = source.replace(/\s+/g, ' ').trim()
  if (!raw) return ''
  return raw.length > MESSAGE_PREVIEW_MAX ? raw.slice(0, MESSAGE_PREVIEW_MAX) + '…' : raw
}

/** 组合式封装：跟随节点与 facets 变化的呈现结果 */
export interface MessageContentView {
  text: ComputedRef<string>
  highlighted: ComputedRef<string>
  rendered: ComputedRef<string>
  renderAsJson: ComputedRef<boolean>
  errorText: ComputedRef<string>
  summaryPreview: ComputedRef<string>
}

export function useMessageContent(
  getNode: () => ChatMessage,
  getFacets: () => MessageFacets,
): MessageContentView {
  const text = computed(() => messageTextOf(getNode().content))
  const highlighted = computed(() => messageHighlightedOf(getNode().content))
  const rendered = computed(() => messageRenderedOf(getNode().content, getFacets().toolResult))
  const renderAsJson = computed(() =>
    messageRenderAsJsonOf(
      getNode().content,
      getFacets().toolResult,
      isFailedStatus(getFacets().status),
    ),
  )
  const errorText = computed(() => messageErrorTextOf(getNode()))
  const summaryPreview = computed(() => {
    const f = getFacets()
    return messageSummaryPreviewOf(
      getNode(),
      f.type === MESSAGE_TYPE_TURN || f.type === MESSAGE_TYPE_TOOL_CALL,
    )
  })
  return { text, highlighted, rendered, renderAsJson, errorText, summaryPreview }
}
