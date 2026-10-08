<!--
  工具调用渲染器 —— 三段式卡片（请求 / 过程 / 结果）

  三段各自独立响应流，纵向排列：
  · **请求**：ToolCall 自带参数（前端合成子节点，JSON 高亮）；
  · **过程**：子会话 Turn（`role=tool`），仅子会话类工具存在，无则**整段隐藏**；
  · **结果**：工具返回文本（JSON）与 user_prompt（审批 / 提问）。

  请求 / 结果段**不设外层标签**——内层节点自身头部已带「📤 请求 / ↩ 响应」语义，
  外层再加标签属于重复呈现（外层仅保留「过程」标签：子会话 Turn 的头部是智能体名，
  无等价语义可承载）。

  工具级失败与 Turn 级失败是**两个不同粒度**：此处「重试」只重跑这一个工具
  （resume action=retry，删除失败结果 + 原参数重执行），不动整个 Turn。

  ## 前端合成「请求」子节点

  ToolCall 的参数就存在它自己的 `content` 里。渲染期把它**提升**为一个独立的、
  可折叠的子节点（与响应子节点并列），ToolCall 于是成为纯组合节点——
  折叠策略、JSON 高亮、层级缩进全部复用内容节点那一套，无需为它写特例。
  该节点不落后端存储（`seq = -1` 保证排在响应子节点之前）。
-->
<template>
  <NodeShell
    :node="node"
    :facets="facets"
    :depth="depth"
    @retry="emit('retry', $event)"
    @delete="emit('delete', $event)"
    @edit="emit('edit', $event)"
  >
    <div class="tool-sections">
      <!-- 请求（内层节点头部即「请求」，不再加外层标签） -->
      <div v-if="requestNodes.length" class="tool-section">
        <MessageChildren
          :nodes="requestNodes"
          :depth="depth ?? 0"
          parent-type="tool_call"
          @retry="emit('retry', $event)"
          @delete="emit('delete', $event)"
          @edit="emit('edit', $event)"
        />
      </div>

      <!-- 过程：子会话实时流（有些工具有，有些没有）。
           B2：**默认收起成一行状态**——全文展开才渲染，主窗口回到「问答 + 状态行」。 -->
      <div v-if="processTurns.length" class="tool-section">
        <div class="ts-label">过程</div>
        <button
          type="button"
          class="process-status"
          :aria-expanded="processOpen"
          @click="processOpen = !processOpen"
        >
          {{ processStatusText }}
        </button>
        <MessageChildren
          v-if="processOpen"
          :nodes="processTurns"
          :depth="depth ?? 0"
          parent-type="tool_call"
          @retry="emit('retry', $event)"
          @delete="emit('delete', $event)"
          @edit="emit('edit', $event)"
        />
      </div>

      <!-- 结果：工具返回 / 审批提问（内层节点头部即「响应」，不再加外层标签） -->
      <div v-if="resultChildren.length" class="tool-section">
        <MessageChildren
          :nodes="resultChildren"
          :depth="depth ?? 0"
          parent-type="tool_call"
          :parent-failed="isFailed"
          @retry="emit('retry', $event)"
          @delete="emit('delete', $event)"
          @edit="emit('edit', $event)"
        />
      </div>

      <!-- 无响应兜底（历史数据 / 不守不变量的写入方）：父节点自述「没跑」却没有任何
           结果子节点时，把这句话显式说出来。留白会让一次调用看起来凭空消失。 -->
      <div v-if="missingResultNote" class="tool-section missing-result">
        <span class="mr-icon">↩</span>
        <span class="mr-text">{{ missingResultNote }}</span>
      </div>

      <!-- 工具级失败：重试此工具（不动 Turn） -->
      <MessageErrorBox
        v-if="isFailed"
        :text="errorText"
        :retryable="canRetry"
        @retry="emit('retry', node.id)"
      />

      <!-- 失败 ToolCall 的补充参数 UI（failure_kind='error' 时显示） -->
      <div v-if="canSupply" class="tool-actions">
        <button class="supply" @click="showSupplyForm = !showSupplyForm">
          {{ showSupplyForm ? '收起' : '补充参数' }}
        </button>
      </div>
      <div v-if="showSupplyForm" class="supply-form">
        <textarea
          v-model="supplyArgsText"
          class="supply-textarea"
          placeholder='{"key":"value"}  (与原参数浅合并)'
          rows="4"
        />
        <button class="supply-submit" @click="submitSupply">提交补充</button>
      </div>
    </div>
  </NodeShell>
</template>

<script setup lang="ts">
import { computed, inject, ref } from 'vue'
import {
  CHAT_ROLE_ASSISTANT,
  CHAT_ROLE_USER,
  MESSAGE_STATUS_COMPLETED,
  MESSAGE_STATUS_STREAMING,
  MESSAGE_TYPE_TEXT,
  MESSAGE_TYPE_TURN,
  RESUME_ACTION_SUPPLY,
  isInflightMessageStatus,
  messageTextOf,
  type ChatMessage,
} from '@/schemas/chat_message'
import { RESUME_KEY } from '@/composables/useChatConnection'
import { useMessageContent } from '@/composables/useMessageContent'
import {
  canRetryTool,
  canSupplyToolArgs,
  isFailedStatus,
  messageParentSessionId,
  missingResultNoteOf,
  type MessageFacets,
} from '@/registry/messageTypes'
import NodeShell from './NodeShell.vue'
import MessageChildren from './MessageChildren.vue'
import MessageErrorBox from './MessageErrorBox.vue'

const props = defineProps<{
  node: ChatMessage
  facets: MessageFacets
  depth?: number
}>()

const emit = defineEmits<{
  retry: [messageId: string]
  delete: [messageId: string]
  edit: [messageId: string]
}>()

/** 会话恢复入口（由 ModelChatPanel 提供）：supply / 子会话路由都走它 */
const resume = inject(RESUME_KEY)

const children = computed(() => props.node.children || [])
const isFailed = computed(() => isFailedStatus(props.facets.status))
const canRetry = computed(() => canRetryTool(props.facets))
const canSupply = computed(() => canSupplyToolArgs(props.facets))

const { text, errorText } = useMessageContent(
  () => props.node,
  () => props.facets,
)

/** 请求参数是否可渲染（流式过程中即便 JSON 尚不完整也展示原始片段） */
const hasRequestArgs = computed(() => !!props.node.content && text.value.trim().length > 0)

/**
 * 合成「请求」子节点。
 *
 * 状态只跟随「流式 / 已结束」两态，**不跟随 ToolCall 的 failed**：
 * 失败信息由 ToolCall 自身的交代条承担，否则请求 JSON 会被误判成错误体。
 */
const toolRequestNode = computed<ChatMessage | null>(() => {
  if (!hasRequestArgs.value) return null
  const base = props.node
  return {
    id: `${base.id}__request`,
    type: MESSAGE_TYPE_TEXT,
    role: CHAT_ROLE_ASSISTANT,
    name: 'request',
    content: base.content,
    status:
      base.status === MESSAGE_STATUS_STREAMING
        ? MESSAGE_STATUS_STREAMING
        : MESSAGE_STATUS_COMPLETED,
    meta: { ...(base.meta || {}), __toolRequest: true },
    timestamp: base.timestamp,
    seq: -1, // 始终排在响应子节点之前
  }
})
const requestNodes = computed<ChatMessage[]>(() =>
  toolRequestNode.value ? [toolRequestNode.value] : [],
)
/** 过程 = 子会话 Turn；结果 = 其余（工具返回 / user_prompt） */
const processTurns = computed<ChatMessage[]>(() =>
  children.value.filter((c) => c.type === MESSAGE_TYPE_TURN),
)
const resultChildren = computed<ChatMessage[]>(() =>
  children.value.filter((c) => c.type !== MESSAGE_TYPE_TURN),
)

// ── B2：过程段收起成一行状态（三字段与后端 `WorkerProgress` 逐字同名） ──
//
// **后端零改动**：状态全部由**已在手的转写**现算——不另存状态、不发请求。
// 字段名一旦与后端投影漂移，界面与注入给模型的快照会自相矛盾；故三处逐字同名，
// 并把字段名写进状态行文本，测试据此断言（改一个字就红）。

/** 步骤条数上限（取最近的）——与后端 `STEPS_MAX` 同值同向。 */
const STEPS_MAX = 8
/** 单步字符上限——与后端 `STEP_MAX_CHARS` 同值。 */
const STEP_MAX_CHARS = 60

/** 判据 1 第二半：过程段子树里是否仍有在途节点（递归，Turn 自身也算）。 */
function subtreeInFlight(nodes: ChatMessage[]): boolean {
  return nodes.some(
    (n) => isInflightMessageStatus(n.status) || subtreeInFlight(n.children ?? []),
  )
}

/** 判据 3：非空内容才产出步骤；取最近 `STEPS_MAX` 条、单步截 `STEP_MAX_CHARS`。 */
function collectSteps(turns: ChatMessage[]): string[] {
  const out: string[] = []
  const walk = (nodes: ChatMessage[]) => {
    for (const n of nodes) {
      // user 是输入不是进展（与后端投影同一口径）
      if (n.role !== CHAT_ROLE_USER) {
        const text = messageTextOf(n.content).trim()
        // 「无内容节点不产出步骤」：组合节点（turn / tool_call）没有 content ⇒ 空串跳过
        if (text) {
          out.push(text.length > STEP_MAX_CHARS ? `${text.slice(0, STEP_MAX_CHARS)}…` : text)
        }
      }
      walk(n.children ?? [])
    }
  }
  walk(turns)
  return out.slice(-STEPS_MAX)
}

/** 过程段状态（B2 三字段），由已在手的转写现算。 */
const workerStatus = computed(() => {
  const turns = processTurns.value
  return {
    // 判据 1：父工具没结束，**或**过程段里仍有在途节点——只看父状态会漏报
    // （父工具常先拿到部分结果）。
    in_flight: isInflightMessageStatus(props.node.status) || subtreeInFlight(turns),
    // 判据 2：过程段的 Turn 条数 = 后端按 user 消息计数，语义一致、数值同源。
    rounds: turns.length,
    // 判据 3：无内容节点不产出步骤——状态行宁可少说也不编。
    steps: collectSteps(turns),
  }
})

/** 过程段默认收起（B2 的目标形态：主窗口回到「问答 + 状态行」）。 */
const processOpen = ref(false)

/** 状态行文本——字段名逐字可见，测试据此断言三处同名。 */
const processStatusText = computed(() => {
  const s = workerStatus.value
  const steps = s.steps.length ? `、steps=[${s.steps.join('；')}]` : ''
  const state = s.in_flight ? '在途' : '已结束'
  return `in_flight=${s.in_flight}（${state}）、rounds=${s.rounds}${steps}`
})

/**
 * 「有请求、无响应」的兜底文案（`registry` 里的判定，本组件只负责画）。
 *
 * 只在**真的没有结果子节点**时给：有子节点就以子节点为准（宁可信真实的，
 * 也不让兜底文案与真结果并排出现）。判定只看父节点自述的终态，见
 * `missingResultNoteOf`。
 */
const missingResultNote = computed<string | null>(() =>
  resultChildren.value.length ? null : missingResultNoteOf(props.node),
)

// ── 补充参数表单 ───────────────────────────────────────────
const showSupplyForm = ref(false)
const supplyArgsText = ref('')

function submitSupply() {
  let parsed: unknown
  try {
    parsed = JSON.parse(supplyArgsText.value || '{}')
  } catch {
    return // JSON 解析失败：静默忽略（后续可加错误提示）
  }
  if (!resume) return
  resume({
    targetId: props.node.id,
    action: RESUME_ACTION_SUPPLY,
    args: parsed,
    // 子会话工具调用需路由回子会话（否则 resume 会打到父会话）
    targetSessionId: messageParentSessionId(props.node),
  })
  showSupplyForm.value = false
}
</script>

<style scoped>
/* 工具调用三段式卡片：请求 / 过程 / 结果（各自独立响应流，纵向排列） */
.tool-sections {
  display: flex;
  flex-direction: column;
  gap: var(--msg-gap, 0.6rem);
}
.tool-section {
  display: flex;
  flex-direction: column;
  gap: 0.3rem;
}
/* 过程段状态行（B2）：默认收起的一行——字段名逐字可见，点击展开全文 */
.process-status {
  display: block;
  width: 100%;
  text-align: left;
  background: none;
  border: none;
  padding: 2px 0;
  font: inherit;
  color: inherit;
  cursor: pointer;
}

.ts-label {
  font-size: 0.68rem;
  font-weight: 600;
  letter-spacing: 0.08em;
  color: var(--text-muted);
}
/* 无响应兜底：中性色而非错误色——「没轮到执行」不是故障。 */
.missing-result {
  flex-direction: row;
  align-items: baseline;
  gap: 0.35rem;
  font-size: 0.78rem;
  color: var(--text-muted);
}
.mr-icon {
  flex-shrink: 0;
}
.mr-text {
  word-break: break-word;
}

/* ── 工具失败补充参数 UI ── */
.tool-actions {
  display: flex;
  gap: 0.4rem;
}
.supply {
  background: var(--color-supply-bg);
  border: 1px solid var(--color-prompt-border);
  color: var(--color-chip-tool-fg);
  border-radius: 0.375rem;
  padding: 0.2rem 0.6rem;
  font-size: 0.76rem;
  cursor: pointer;
}
.supply:hover {
  background: var(--color-prompt-bg);
}
.supply-form {
  display: flex;
  flex-direction: column;
  gap: 0.35rem;
  background: var(--color-msg-card);
  border: 1px solid var(--color-supply-border);
  border-radius: 0.5rem;
  padding: 0.5rem 0.6rem;
}
.supply-textarea {
  width: 100%;
  box-sizing: border-box;
  border: 1px solid var(--color-supply-border);
  border-radius: 0.375rem;
  padding: 0.35rem 0.45rem;
  font-family: 'JetBrains Mono', ui-monospace, monospace;
  font-size: 0.76rem;
  line-height: 1.5;
  resize: vertical;
  outline: none;
}
.supply-textarea:focus {
  border-color: var(--accent);
}
.supply-submit {
  align-self: flex-start;
  background: var(--accent);
  color: var(--text-on-accent);
  border: none;
  border-radius: 0.375rem;
  padding: 0.3rem 0.9rem;
  font-size: 0.78rem;
  cursor: pointer;
}
.supply-submit:hover {
  background: var(--accent-hover);
}
</style>
