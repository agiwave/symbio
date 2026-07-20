<!--
  对话消息渲染（Content / Composite 分型模型 · 统一折叠式节点）

  设计要点：
  - 所有节点（内容节点 + 组合节点）使用**完全一致**的展示与交互：
    一个可点击折叠的「头部」（图标 + 标题 + 状态标签 + 折叠箭头）+ 一个折叠体。
  - 层级（无限递归的分形会话流）只靠「缩进 + 左侧引导竖线」表达，不再叠加任何外框，
    杜绝「竖线 + 外框」同时出现的视觉冲突。
  - 折叠策略（主流习惯，参考 ChatGPT / Claude / Cursor）：
      · 主回复 / 用户消息 → 默认展开（用户要读的正文）。
      · 思考过程 / 工具调用 / 工具结果 等子步骤 → 默认收起为单行（标题 + 摘要预览）。
      · 流模式 / 失败 / 待审批 → 始终展开（看实时内容与错误）。
      · 深层级（子 agent 嵌套内部，depth ≥ 3）→ 一律收起为单行摘要，避免长对话过深。
      · 用户手动点击头部收拢/展开后，以其选择为准（覆盖默认）。
  - 语义对应：工具调用 = 子会话流（组合节点），其自身 content 携带请求参数（JSON）；
      渲染时请求参数被提升为 ToolCall 的一个「前端合成子节点」（标题「请求」），与响应子节点
      （Turn/Text, role=Tool）并列，使 ToolCall 成为与其他层级完全一致的纯组合节点。
      流模式响应(Turn, role=Tool) ≈ 主会话中「agent 响应」，可再嵌套 ToolCall，形成无限分型。
  - 统一显示规则（根级 / 子级一致，杜绝风格分裂）：
      · Turn 在任意层级都渲染为「图标 + 名称」头部（根级=「AI 助手」，子级=「↳ 子智能体/智能体名」），
        其正文（思考/文本）一律内联为该 Turn 的折叠体 —— 不随 role 不同而被拆成独立的「响应」块。
      · 「响应」标签仅保留给直接挂在 ToolCall 下的原始工具返回（role=Tool 的纯文本/JSON 结果）。
      · 同层级内容的左右起始位置对齐：靠「统一背景框 + 统一内边距(--card-pad-*)」；
        层级靠「统一缩进(--nest-indent) + 左侧引导竖线(depth≥2)」表达，头部与子内容左缘齐平。
      · 纵向间隔全由单一变量 --msg-gap 驱动（顶层 / Turn 体 / 工具子节点共用），机制化保障一致。
-->
<template>
  <div class="msg" :class="[typeClass, statusClass, isUser ? 'user' : '', depth && depth > 1 ? 'nested' : '']">
    <!-- 统一头部：所有节点一致的可点击折叠栏（直接文本回复内联，不重复头部） -->
    <div v-if="!isResponseText" class="node-head" :class="headClass" @click="toggle">
      <span class="node-icon">{{ icon }}</span>
      <span class="node-title">{{ title }}</span>
      <span v-if="isHeartbeat" class="node-heartbeat" title="系统心跳任务自动发送">♥ 心跳</span>
      <!-- 收起态展示单行摘要（深层级 / 子步骤默认收起时，让用户无需展开即知内容） -->
      <span v-if="!effectiveOpen && summaryPreview" class="node-preview">{{ summaryPreview }}</span>
      <span v-if="statusTag" class="node-tag" :class="tagClass">{{ statusTag }}</span>
      <span v-if="isStreaming" class="node-live">{{ isToolCall ? '调用中…' : '回复中…' }}</span>
      <!-- 悬停操作：仅用户消息可编辑；仅 root 级节点可删除（删除会连带其后所有消息） -->
      <span class="node-actions" @click.stop>
        <button v-if="isUser" class="node-act" title="编辑" @click.stop="emit('edit', node.id)">✎</button>
        <button v-if="!node.parent_id" class="node-act" title="删除" @click.stop="emit('delete', node.id)">🗑</button>
      </span>
    </div>

    <div v-show="effectiveOpen" class="node-body">
      <!-- 用户消息：右对齐气泡（折叠体内容） -->
      <div v-if="isUser" class="user-row">
        <div class="bubble user-bubble">
          <div class="markdown-body" v-html="rendered" />
        </div>
      </div>

      <!-- 思考过程 -->
      <div v-else-if="isReasoning" class="markdown-body" v-html="rendered" />

      <!-- 文本 / 工具请求 / 工具返回 -->
      <template v-else-if="isTextLike">
        <div v-if="isFailed" class="error-box">
          <span class="err-icon">⚠</span>
          <span class="err-text">{{ errorText }}</span>
          <button v-if="canRetry" class="retry" @click="emit('retry', node.id)">重试</button>
        </div>
        <pre
          v-else-if="renderAsJson || isToolResult"
          :class="['json', isToolRequest ? 'req' : '']"
          v-html="highlight(props.node.content)"
        />
        <div v-else class="markdown-body" v-html="rendered" />
      </template>

      <!-- 待用户响应（user_prompt：ask_user 提问 / 工具确认） -->
      <template v-else-if="isUserPrompt">
        <div class="user-prompt" :class="{ answered: isAnswered }">
          <template v-if="prompt && prompt.kind === 'question'">
            <div v-for="q in prompt.questions" :key="q.id" class="up-question">
              <div v-if="q.header" class="up-header">{{ q.header }}</div>
              <div class="up-qtext">{{ q.question }}</div>
              <div class="up-options">
                <label
                  v-for="opt in q.options"
                  :key="opt.label"
                  class="up-option"
                  :class="{ active: (selected[q.id] || []).includes(opt.label) }"
                >
                  <input
                    :type="q.multiSelect ? 'checkbox' : 'radio'"
                    :name="q.id"
                    :value="opt.label"
                    :checked="(selected[q.id] || []).includes(opt.label)"
                    :disabled="isAnswered"
                    @change="onToggleOption(q.id, !!q.multiSelect, opt.label)"
                  />
                  <span class="up-opt-label">{{ opt.label }}</span>
                  <span v-if="opt.description" class="up-opt-desc">{{ opt.description }}</span>
                </label>
                <!-- 'Other' 自由输入 -->
                <label
                  v-if="q.options.some(o => o.label === 'Other')"
                  class="up-option up-option-other"
                  :class="{ active: (selected[q.id] || []).includes('Other') }"
                >
                  <input
                    type="checkbox"
                    :name="q.id + '__other'"
                    :checked="(selected[q.id] || []).includes('Other')"
                    :disabled="isAnswered"
                    @change="onToggleOption(q.id, true, 'Other')"
                  />
                  <span class="up-opt-label">Other</span>
                  <input
                    v-model="customInput[q.id]"
                    class="up-other-input"
                    type="text"
                    placeholder="请输入…"
                    :disabled="isAnswered || !(selected[q.id] || []).includes('Other')"
                  />
                </label>
              </div>
            </div>
            <button class="up-submit" :disabled="isAnswered" @click="submitQuestions">
              {{ isAnswered ? '已提交' : '提交' }}
            </button>
          </template>

          <template v-else-if="prompt && prompt.kind === 'confirm'">
            <div class="up-confirm">
              <div class="up-confirm-tool">
                <span class="up-tool-name">{{ prompt.tool_name }}</span>
                <span v-if="prompt.risk_level" class="up-risk" :class="'risk-' + prompt.risk_level">{{ prompt.risk_level }}</span>
              </div>
              <div class="up-confirm-desc">{{ prompt.description }}</div>
              <pre v-if="prompt.args !== undefined" class="up-args">{{ JSON.stringify(prompt.args, null, 2) }}</pre>
            </div>
            <div class="up-confirm-btns">
              <button class="up-approve" :disabled="isAnswered" @click="submitConfirm(true)">
                {{ isAnswered ? '已处理' : '批准执行' }}
              </button>
              <button class="up-reject" :disabled="isAnswered" @click="submitConfirm(false)">
                {{ isAnswered ? '已处理' : '拒绝' }}
              </button>
            </div>
          </template>
        </div>
      </template>

      <!-- 组合节点（Turn / ToolCall）：递归渲染子节点（内部的子会话流） -->
      <template v-else>
        <!-- 待用户响应区（ask_user 提问 / 工具确认）—— 与失败重试同构的特殊失败态 -->
        <div v-if="isUserPrompt && isWaiting" class="user-prompt">
          <div v-if="prompt?.kind === 'confirm'" class="up-confirm">
            <div class="up-desc">{{ prompt?.description || ('工具：' + (prompt?.tool_name || 'tool')) }}</div>
            <div class="up-meta">
              <span class="up-tag" :class="riskClass">{{ prompt?.risk_level }}</span>
              <span class="up-tool">工具：{{ prompt?.tool_name }}</span>
            </div>
            <div class="approval-btns">
              <button class="approve" @click="submitConfirm(true)">批准执行</button>
              <button class="reject" @click="submitConfirm(false)">拒绝</button>
            </div>
          </div>

          <div v-else-if="prompt?.kind === 'question'" class="up-question">
            <div v-for="q in prompt?.questions || []" :key="q.id" class="up-q">
              <div class="up-q-head" v-if="q.header">{{ q.header }}</div>
              <div class="up-q-text">{{ q.question }}</div>
              <div class="up-opts">
                <label
                  v-for="opt in q.options"
                  :key="opt.label"
                  class="up-opt"
                  :class="{ 'up-opt-sel': (selected[q.id] || []).includes(opt.label) }"
                >
                  <input
                    v-if="q.multiSelect"
                    type="checkbox"
                    :checked="(selected[q.id] || []).includes(opt.label)"
                    @change="onToggleOption(q.id, true, opt.label)"
                  />
                  <input
                    v-else
                    type="radio"
                    :name="q.id"
                    :checked="(selected[q.id] || []).includes(opt.label)"
                    @change="onToggleOption(q.id, false, opt.label)"
                  />
                  <span class="up-opt-label">{{ opt.label }}</span>
                  <span v-if="opt.description" class="up-opt-desc">{{ opt.description }}</span>
                </label>
              </div>
              <!-- "Other" 自定义输入 -->
              <input
                v-if="(q.options || []).some(o => (o.label || '').toLowerCase() === 'other')"
                class="up-other"
                :placeholder="`自定义（Other）`"
                :value="customInput[q.id] || ''"
                @input="customInput[q.id] = ($event.target as HTMLInputElement).value"
              />
            </div>
            <button class="approve up-submit" @click="submitQuestions">提交</button>
          </div>
        </div>

        <!-- ToolCall：纯组合节点。请求参数作为「前端合成子节点」(请求) 与响应子节点（Turn/Text）并排，
             全部走递归 <MessageNode>，保证「请求 / 响应」都是可折叠的子节点，结构与其他层级完全一致。 -->
        <div v-if="isToolCall" class="tool-children">
          <MessageNode
            v-for="child in renderedChildren"
            :key="child.id"
            :node="child"
            :depth="(depth ?? 0) + 1"
            :parent-type="type"
            @retry="emit('retry', $event)"
            @delete="emit('delete', $event)"
            @edit="emit('edit', $event)"
          />
        </div>

        <!-- 普通 Turn：直接递归子节点 -->
        <template v-else>
          <MessageNode
            v-for="child in node.children"
            :key="child.id"
            :node="child"
            :depth="(depth ?? 0) + 1"
            :parent-type="type"
            @retry="emit('retry', $event)"
            @delete="emit('delete', $event)"
            @edit="emit('edit', $event)"
          />
        </template>

        <div v-if="isFailed" class="error-box">
          <span class="err-icon">⚠</span>
          <span class="err-text">{{ errorText }}</span>
          <button v-if="canRetry" class="retry" @click="emit('retry', node.id)">重试</button>
        </div>

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
      </template>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, inject, ref } from 'vue'
import { useMarkdown } from '@/composables/useMarkdown'
import type { ChatMessage, MessageContent } from '@/services/model'

const props = defineProps<{
  node: ChatMessage
  /** 嵌套深度：0 = 主会话根级；>0 = 工具/子 agent 内部的子会话流层级 */
  depth?: number
  /** 父节点类型：用于识别「Turn 的直接文本回复」，避免与 Turn 头部重复的冗余标题 */
  parentType?: string
}>()

const emit = defineEmits<{
  retry: [messageId: string]
  delete: [messageId: string]
  edit: [messageId: string]
}>()

const { renderMarkdown } = useMarkdown()
// 会话恢复（retry_turn/retry/approve/reject/supply/answer）：由 ModelChatPanel 提供，
// 内部走 chat/resume 的 target_id（稳定锚点：Failed Turn 或 ToolCall） + targetSessionId（子会话工具调用需路由回子会话）。
interface ResumePayload {
  targetId: string
  action: 'retry_turn' | 'retry' | 'approve' | 'reject' | 'supply' | 'answer'
  args?: unknown
  reason?: string
  answer?: unknown
  targetSessionId?: string
}
const resume = inject<(payload: ResumePayload) => void>('resume')

// user_prompt 子节点的 meta.parent_session_id：子会话工具调用恢复需路由回子会话
function parentSessionId(): string | undefined {
  return (props.node.meta as Record<string, any> | undefined)?.parent_session_id
}
// user_prompt / 失败子节点的父 ToolCall id（恢复锚点）
function parentToolCallId(): string | undefined {
  return props.node.parent_id || undefined
}

// ── user_prompt 表单（后端 ask_user / 工具确认广播）────────────
// meta.prompt 结构：
//   question: { kind: 'question', questions: [{ id, header, question, multiSelect, options: [{label, description}] }] }
//   confirm:  { kind: 'confirm', tool_name, args, risk_level, description }
interface PromptOption {
  label: string
  description?: string
}
interface PromptQuestion {
  id: string
  header?: string
  question: string
  multiSelect?: boolean
  options: PromptOption[]
}
interface Prompt {
  kind: 'question' | 'confirm'
  questions?: PromptQuestion[]
  tool_name?: string
  args?: unknown
  risk_level?: string
  description?: string
}

const prompt = computed<Prompt | null>(() => {
  const meta = props.node.meta as Record<string, any> | undefined
  const p = meta?.prompt as Prompt | undefined
  return p ?? null
})
// 已回答：状态不再是 waiting_user_action 时，表单禁用并改为已提交态
const isAnswered = computed(() => status.value !== 'waiting_user_action')
// 表单本地状态：每个 questionId -> 选中的 label 列表；含 'Other' 时的自由文本
const selected = ref<Record<string, string[]>>({})
const customInput = ref<Record<string, string>>({})

function onToggleOption(questionId: string, multiSelect: boolean, label: string) {
  const cur = selected.value[questionId] || []
  if (multiSelect) {
    // 多选：切换该 label；'Other' 与普通选项可共存
    const next = cur.includes(label) ? cur.filter((l) => l !== label) : [...cur, label]
    selected.value = { ...selected.value, [questionId]: next }
  } else {
    // 单选：直接替换为该 label
    selected.value = { ...selected.value, [questionId]: [label] }
  }
}

function submitConfirm(approved: boolean) {
  const tcId = parentToolCallId()
  if (!tcId || !resume) return
  resume({
    targetId: tcId,
    action: approved ? 'approve' : 'reject',
    reason: approved ? undefined : '用户拒绝',
    targetSessionId: parentSessionId(),
  })
}

function submitQuestions() {
  const tcId = parentToolCallId()
  if (!tcId || !resume) return
  const answers = (prompt.value?.questions || []).map((q) => ({
    question_id: q.id,
    selected: selected.value[q.id] || [],
    custom_input: selected.value[q.id]?.includes('Other')
      ? (customInput.value[q.id] || '')
      : null,
  }))
  resume({
    targetId: tcId,
    action: 'answer',
    answer: { answers },
    targetSessionId: parentSessionId(),
  })
}

const riskClass = computed(() => {
  const r = (prompt.value?.risk_level || '').toLowerCase()
  if (r === 'high') return 'risk-high'
  if (r === 'medium') return 'risk-med'
  return 'risk-low'
})

// ── 类型判定 ───────────────────────────────────────────────
const role = computed(() => props.node.role || 'assistant')
const type = computed(() => props.node.type || 'text')
const status = computed(() => props.node.status || 'completed')

const isUser = computed(() => role.value === 'user')
// 系统心跳任务自动发送的消息（后端 trigger_heartbeat 在 meta.heartbeat 打标）
const isHeartbeat = computed(
  () => !!(props.node.meta as Record<string, any> | undefined)?.heartbeat,
)
const isTurn = computed(() => type.value === 'turn')
const isToolCall = computed(() => type.value === 'tool_call')
const isReasoning = computed(() => type.value === 'reasoning')
// 待用户响应的提问/确认节点（后端 ask_user / 工具确认广播的 user_prompt 类型）
const isUserPrompt = computed(() => type.value === 'user_prompt')
// 原始工具返回：直接挂在 ToolCall 下的 role=Tool 文本节点（JSON 结果），标题「响应」。
// 必须限定 parentType='tool_call'：子 agent 响应 Turn 内部的文本子节点 role 也是 Tool，
// 但若误判为「工具返回」会被渲染成独立「响应」块，破坏与根级 Turn 的一致性（见 isResponseText）。
const isToolResult = computed(
  () => role.value === 'tool' && type.value === 'text' && props.parentType === 'tool_call',
)
// ToolCall 自带的请求参数（content 为 JSON 文本）是否可渲染（流式过程中即便 JSON 尚不完整也展示原始片段）
const hasRequestArgs = computed(
  () => isToolCall.value && !!props.node.content && textContent.value.trim().length > 0,
)
// 工具「请求参数」前端合成子节点（不进入后端存储，仅渲染期把 ToolCall 自身 content 提升为
// 一个独立、可折叠的子节点，与响应子节点（Turn/Text）并列，使 ToolCall 成为纯组合节点）。
const toolRequestNode = computed<ChatMessage | null>(() => {
  if (!isToolCall.value || !hasRequestArgs.value) return null
  const base = props.node
  return {
    id: `${base.id}__request`,
    type: 'text',
    role: 'assistant',
    name: 'request',
    content: base.content,
    // 请求节点只展示「已发出的请求」，不跟随 ToolCall 的 failed 状态
    // （失败信息由 ToolCall 自身的 error-box 承担，避免请求 JSON 被误判为错误体）。
    status: base.status === 'streaming' ? 'streaming' : 'completed',
    meta: { ...(base.meta || {}), __toolRequest: true },
    timestamp: base.timestamp,
    sort_index: -1, // 始终排在响应子节点之前
  }
})
// ToolCall 实际渲染的子节点 = 合成「请求」节点（若有）+ 后端真实响应子节点
const renderedChildren = computed<ChatMessage[]>(() => {
  const kids = props.node.children || []
  const req = toolRequestNode.value
  return req ? [req, ...kids] : kids
})
// 是否为合成「请求」子节点（用于标题 / 图标 / 渲染分支的特判）
const isToolRequest = computed(() => !!(props.node.meta as Record<string, any> | undefined)?.__toolRequest)
// 其余（非用户 / 非 Turn / 非 ToolCall / 非 Reasoning / 非 UserPrompt）：文本 / 请求 / 返回等叶子内容节点
const isTextLike = computed(
  () => !isUser.value && !isTurn.value && !isToolCall.value && !isReasoning.value && !isUserPrompt.value,
)
// 子会话响应：role=Tool 的 Turn，即「某个工具内部的流模式响应」（≈主会话 agent 响应）
const isSubSession = computed(() => isTurn.value && role.value === 'tool')
// 直接文本回复：Turn 的直接文本子节点，正文作为 Turn 主体内联展示（不重复头部，
// 由父 Turn 头部统一代表，折叠也随父 Turn）。
// 不限定 role=assistant：子智能体响应 Turn(role=Tool) 的文本子节点 role 同为 Tool，
// 但其本质仍是「该 Turn 的回复正文」，必须内联；否则会被误判为独立「响应」块，
// 导致根级 Turn（AI 助手 + 正文）与子 agent Turn（响应块）风格不一致。
const isResponseText = computed(
  () => isTextLike.value && props.parentType === 'turn',
)

const isStreaming = computed(() => status.value === 'streaming')
const isFailed = computed(() => status.value === 'failed')
const isWaiting = computed(() => status.value === 'waiting_user_action')

const typeClass = computed(() => `type-${type.value}`)
const statusClass = computed(() => `status-${status.value}`)

// ── 折叠状态（统一：所有节点都可点击头部折叠）───────────────
// 主流折叠默认态（参考 ChatGPT / Claude / Cursor）：
//   · 主回复 / 用户消息 → 展开（用户要读的正文）
//   · 思考过程 / 工具调用 / 工具结果 等子步骤 → 默认收起为单行
//   · 流模式 / 失败 / 待审批 → 始终展开（看实时内容与错误）
// 深层级（子 agent 嵌套内部，depth ≥ 3）一律收起为单行摘要，避免长对话过深。
//   （子 agent 自身的响应回合 depth=2 已由 defaultOpen 收起；此处针对其更深的内部步骤）
// 用户手动收拢/展开后以其选择为准（userToggled 优先）。
const DEEP_COLLAPSE_LEVEL = 3
const isDeep = computed(() => (props.depth ?? 0) >= DEEP_COLLAPSE_LEVEL)
const defaultOpen = computed(() => {
  const st = status.value
  // 待用户响应 / 失败 → 始终展开（用户需看到并操作）
  if (st === 'streaming' || st === 'waiting_user_action' || st === 'failed') return true
  // 工具调用节点：若有 failure_kind meta（可重试/补充），也展开
  const meta = props.node.meta as Record<string, any> | undefined
  if (meta?.failure_kind && isToolCall.value) return true
  if (role.value === 'user') return true
  if (type.value === 'turn') return role.value !== 'tool'   // 根级助手回合展开；子 agent 响应回合收起
  if (type.value === 'reasoning') return false
  if (type.value === 'tool_call') return false
  if (type.value === 'text') return role.value !== 'tool'   // 助手文本展开；工具结果收起
  return true
})
const userToggled = ref(false)
const userOpen = ref(true)
const effectiveOpen = computed(() => {
  if (userToggled.value) return userOpen.value
  if (isDeep.value) return false
  return defaultOpen.value
})
function toggle() {
  // 基于「当前实际展示态」取反，而非固定初始值：否则默认收起节点首次点击会从
  // 初始 userOpen(true) 翻成 false，展示态不变（仍收起），表现为"第一次点击无反应"。
  const nowOpen = userToggled.value
    ? userOpen.value
    : isDeep.value
      ? false
      : defaultOpen.value
  userToggled.value = true
  userOpen.value = !nowOpen
}

// ── 头部展示信息（统一图标 / 标题 / 状态标签）────────────────
const icon = computed(() => {
  if (isUser.value) return '👤'
  if (isUserPrompt.value) return '❓'
  if (isToolRequest.value) return '📤'
  if (isReasoning.value) return '💭'
  if (isToolCall.value) return '🔧'
  if (isTurn.value) return isSubSession.value ? '↳' : 'AI'
  if (isToolResult.value) return '↩'
  return '💬'
})
// Turn 标题统一为「该智能体的名称」：根级助手回合显示助手名，子智能体响应回合显示子智能体名，
// 从机制上保证任意层级的 Turn 都是「图标 + 名称」的一致外观（不再有裸「响应」块）。
const agentName = computed(() => {
  const meta = props.node.meta as Record<string, any> | undefined
  const fromMeta = meta?.agent_name || meta?.agent_id
  return props.node.name || fromMeta || (isSubSession.value ? '子智能体' : '助手')
})
const title = computed(() => {
  if (isUser.value) return '你'
  if (isUserPrompt.value) return prompt.value?.kind === 'confirm' ? '工具确认' : '提问'
  if (isToolRequest.value) return '请求'
  if (isReasoning.value) return '思考'
  if (isToolCall.value) return props.node.name || '工具'
  if (isTurn.value) return agentName.value
  if (isToolResult.value) return '响应'
  return role.value === 'assistant' ? '助手' : '消息'
})
const statusTag = computed(() => {
  if (!isToolCall.value) return ''
  if (isFailed.value) return '失败'
  if (isStreaming.value) return '调用中…'
  return ''
})
const tagClass = computed(() => {
  if (isWaiting.value) return 'warn'
  if (isFailed.value) return 'err'
  return 'sub'
})
const headClass = computed(() => ({
  user: isUser.value,
  sub: isSubSession.value,
  tool: isToolCall.value,
  reasoning: isReasoning.value,
}))

// ── 内容渲染 ───────────────────────────────────────────────
const textContent = computed(() => {
  const c = props.node.content
  if (!c) return ''
  if (typeof c === 'string') return c
  if (typeof c === 'object' && 'text' in c) return (c as { text: string }).text
  if (typeof c === 'object' && 'parts' in c) {
    return (c as { parts: Array<{ text?: string }> }).parts.map((p) => p.text || '').join('\n')
  }
  return ''
})

// 收起态单行摘要：组合节点取首个文本/思考子节点内容；内容节点取自身文本。
// 用于深层级 / 子步骤默认收起时，让用户无需展开即知概要。
const previewSource = computed(() => {
  if (isTurn.value || isToolCall.value) {
    const kids = props.node.children || []
    const first = kids.find((c) => c.type === 'text' || c.type === 'reasoning')
    if (first) {
      const c = first.content
      if (typeof c === 'string') return c
      if (c && typeof c === 'object' && 'text' in c) return (c as { text: string }).text
    }
    return ''
  }
  return textContent.value
})
const summaryPreview = computed(() => {
  const raw = previewSource.value.replace(/\s+/g, ' ').trim()
  if (!raw) return ''
  return raw.length > 80 ? raw.slice(0, 80) + '…' : raw
})

const rendered = computed(() => {
  if (isToolResult.value) return highlight(props.node.content)
  const t = textContent.value
  return t ? renderMarkdown(t) : ''
})

// 内容为纯 JSON（如工具请求参数 / 工具返回）时以代码块高亮展示，保持会话流内可读且统一
function isJsonContent(t: string): boolean {
  const s = t.trim()
  if (!s || !/^[\[{]/.test(s)) return false
  try {
    JSON.parse(s)
    return true
  } catch {
    return false
  }
}
const renderAsJson = computed(
  () => !isToolResult.value && !isFailed.value && isJsonContent(textContent.value),
)

const errorText = computed(() => {
  // 优先用消息自身的 error 字段（后端持久化的失败原因），
  // 回退到 meta.error（旧路径），再回退到文本内容。
  const meta = props.node.meta as Record<string, any> | undefined
  return (
    props.node.error ||
    (meta?.error as string | undefined) ||
    textContent.value ||
    '执行失败'
  )
})
const canRetry = computed(() => {
  if (!isFailed.value) return false
  // 工具调用失败（带 failure_kind）→ 可重试（走 resume retry）
  if (isToolCall.value) return true
  // 普通文本/响应失败 → 可重试（走 resume retry_turn）
  return role.value !== 'tool' && !isToolResult.value && !isToolRequest.value
})
// 工具调用失败且 failure_kind='error' → 可补充参数重新执行
const canSupply = computed(
  () =>
    isToolCall.value &&
    isFailed.value &&
    (props.node.meta as Record<string, any> | undefined)?.failure_kind === 'error',
)
// 补充参数表单（JSON 文本框 + 提交按钮）
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
    action: 'supply',
    args: parsed,
    targetSessionId: parentSessionId(),
  })
  showSupplyForm.value = false
}

// ── JSON / 文本高亮 ────────────────────────────────────────
function escapeHtml(s: string): string {
  return s.replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;')
}
function highlight(content?: MessageContent | string): string {
  const raw =
    typeof content === 'string'
      ? content
      : textContentOf(content)
  if (!raw) return ''
  let obj: unknown
  try {
    obj = JSON.parse(raw)
  } catch {
    return `<span class="json-str">${escapeHtml(raw)}</span>`
  }
  const pretty = JSON.stringify(obj, null, 2)
  return highlightJsonString(pretty)
}
function textContentOf(content?: MessageContent | string): string {
  if (!content) return ''
  if (typeof content === 'string') return content
  if (typeof content === 'object' && 'text' in content) return (content as { text: string }).text
  return ''
}
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
</script>

<style scoped>
.msg {
  width: 100%;
}
/* 分形层级：仅用「缩进 + 左侧引导竖线」表达嵌套深度（类邮件/论坛主题串），
   不再叠加任何外框，彻底避免「竖线 + 外框」的视觉冲突。
   注意：缩进只作用于 depth>=2 的层级（见模板 :class）。depth=1 的子步骤（思考过程 /
   工具调用）保持与主回合头部（AI 头像）左边缘齐平 —— 这是「头像与思考过程左对齐」的关键：
   顶层 Turn 头部 icon 位于 msg 容器左缘（x=0），depth=1 子节点不再缩进，故内容左缘同样为 x=0，
   二者天然对齐；更深层级才施加统一缩进并绘制引导竖线。 */
.msg.nested {
  margin-left: var(--nest-indent, 0.7rem);
  padding-left: var(--nest-indent, 0.7rem);
  border-left: 2px solid #e8edf4;
}

/* ── Markdown 内容（v-html 注入，需用 :deep 穿透 scoped 样式）── */
.msg :deep(.markdown-body) {
  font-size: 0.9rem;
  line-height: 1.6;
  color: var(--color-text, #0f172a);
  word-break: break-word;
  overflow-wrap: anywhere;
  max-width: 100%;
}
.msg :deep(.markdown-body > *:first-child) {
  margin-top: 0;
}
.msg :deep(.markdown-body > *:last-child) {
  margin-bottom: 0;
}
.msg :deep(.markdown-body p) {
  margin: 0 0 0.5rem;
}
.msg :deep(.markdown-body h1),
.msg :deep(.markdown-body h2),
.msg :deep(.markdown-body h3),
.msg :deep(.markdown-body h4) {
  margin: 0.75rem 0 0.4rem;
  line-height: 1.3;
  font-weight: 600;
}
.msg :deep(.markdown-body h1) {
  font-size: 1.15rem;
}
.msg :deep(.markdown-body h2) {
  font-size: 1.05rem;
}
.msg :deep(.markdown-body h3) {
  font-size: 0.98rem;
}
.msg :deep(.markdown-body h4) {
  font-size: 0.9rem;
}
.msg :deep(.markdown-body ul),
.msg :deep(.markdown-body ol) {
  margin: 0.5rem 0;
  padding-left: 1.6rem;
  list-style-position: outside;
}
.msg :deep(.markdown-body ul) {
  list-style: disc;
}
.msg :deep(.markdown-body ol) {
  list-style: decimal;
}
.msg :deep(.markdown-body li) {
  margin: 0.2rem 0;
}
.msg :deep(.markdown-body li > ul),
.msg :deep(.markdown-body li > ol) {
  margin: 0.2rem 0;
}
.msg :deep(.markdown-body a) {
  color: var(--color-primary, #667eea);
  text-decoration: none;
}
.msg :deep(.markdown-body a:hover) {
  text-decoration: underline;
}
.msg :deep(.markdown-body code) {
  background: rgba(15, 23, 42, 0.06);
  padding: 0.1rem 0.3rem;
  border-radius: 4px;
  font-family: 'JetBrains Mono', ui-monospace, monospace;
  font-size: 0.85em;
}
.msg :deep(.markdown-body pre) {
  background: #0f172a;
  color: #e2e8f0;
  padding: 0.6rem 0.8rem;
  border-radius: 8px;
  overflow-x: auto;
  margin: 0.5rem 0;
}
.msg :deep(.markdown-body pre code) {
  background: transparent;
  padding: 0;
  font-size: 0.82rem;
}
.msg :deep(.markdown-body blockquote) {
  margin: 0.5rem 0;
  padding-left: 0.8rem;
  border-left: 3px solid var(--color-border, #e2e8f0);
  color: var(--color-text-secondary, #475569);
}
.msg :deep(.markdown-body table) {
  border-collapse: collapse;
  margin: 0.5rem 0;
  display: block;
  overflow-x: auto;
}
.msg :deep(.markdown-body th),
.msg :deep(.markdown-body td) {
  border: 1px solid var(--color-border, #e2e8f0);
  padding: 0.3rem 0.5rem;
  font-size: 0.85rem;
}
.msg :deep(.markdown-body img) {
  max-width: 100%;
  border-radius: 6px;
}

/* ── 统一节点头部（所有节点一致）── */
.node-head {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  cursor: pointer;
  user-select: none;
  padding: 0.16rem 0.3rem;
  margin: 0 -0.3rem;
  border-radius: 6px;
  font-size: 0.8rem;
  transition: background 0.12s ease;
}
.node-head:hover {
  background: rgba(99, 102, 241, 0.06);
}
/* 用户消息整体右对齐；头像置于文字右侧（主流 IM 习惯），折叠箭头在最右。
   用 order 重排而非 row-reverse，避免箭头被推到最左。 */
.node-head.user {
  justify-content: flex-end;
}
.node-head.user .node-title {
  order: 1;
}
.node-head.user .node-icon {
  order: 2;
}
.node-icon {
  width: 18px;
  height: 18px;
  border-radius: 6px;
  font-size: 0.62rem;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  background: #eef2f7;
  color: #64748b;
}
.node-head.user .node-icon {
  background: #dbeafe;
  color: #2563eb;
}
.node-head.sub .node-icon {
  background: #e0f2fe;
  color: #0284c7;
}
.node-head.tool .node-icon {
  background: #eef2ff;
  color: #4f46e5;
}
.node-head.reasoning .node-icon {
  background: #f3e8ff;
  color: #7c3aed;
}
.node-title {
  font-weight: 500;
  font-size: 0.8rem;
  color: var(--color-text-secondary, #475569);
  flex-shrink: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.node-head.sub .node-title {
  color: #0369a1;
}
/* 收起态单行摘要（标题之后、状态标签之前，省略号截断） */
.node-preview {
  flex: 1;
  min-width: 0;
  font-size: 0.74rem;
  color: var(--color-text-muted, #94a3b8);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.node-live {
  font-size: 0.72rem;
  color: #6366f1;
}
.node-head.sub .node-live {
  color: #0ea5e9;
}

/* ── 悬停操作（编辑 / 删除）──
   默认隐藏，鼠标悬停整条消息时显示在头部右侧。
   用 @click.stop 阻止冒泡触发头部折叠。 */
.node-actions {
  display: none;
  align-items: center;
  gap: 0.15rem;
  margin-left: 0.25rem;
  flex-shrink: 0;
}
.msg:hover > .node-head .node-actions {
  display: inline-flex;
}
.node-act {
  border: none;
  background: rgba(100, 116, 139, 0.1);
  color: #64748b;
  border-radius: 4px;
  width: 18px;
  height: 18px;
  font-size: 0.7rem;
  line-height: 1;
  cursor: pointer;
  padding: 0;
  display: inline-flex;
  align-items: center;
  justify-content: center;
}
.node-act:hover {
  background: rgba(239, 68, 68, 0.16);
  color: #dc2626;
}
.node-tag {
  font-size: 0.68rem;
  padding: 0.05rem 0.4rem;
  border-radius: 999px;
}
.node-tag.sub {
  background: #e0e7ff;
  color: #4338ca;
}
.node-tag.warn {
  background: #fef3c7;
  color: #92400e;
}
.node-tag.err {
  background: #fee2e2;
  color: #b91c1c;
}
.node-heartbeat {
  display: inline-flex;
  align-items: center;
  font-size: 0.7rem;
  font-weight: 500;
  color: #ef4444;
  background: rgba(239, 68, 68, 0.12);
  border-radius: 4px;
  padding: 0 0.35rem;
  margin-left: 0.25rem;
  flex-shrink: 0;
}

/* ── 折叠体 ──
   兄弟元素（思考过程 / 工具调用 / 文本回复）之间、以及头部到首个子元素的纵向间隔，
   全部引用统一变量 --msg-gap，确保「助手 ↔ 思考过程」与「思考过程 ↔ 工具调用」等
   所有间隔完全一致（机制化保障，而非散落硬编码）。 */
.node-body {
  display: flex;
  flex-direction: column;
  gap: var(--msg-gap, 0.6rem);
  padding: var(--msg-gap, 0.6rem) 0 0;
}

/* ── 用户消息 ── */
.user-row {
  display: flex;
  justify-content: flex-end;
}
.user-bubble {
  max-width: 80%;
  background: linear-gradient(135deg, #3b82f6 0%, #2563eb 100%);
  color: #fff;
  border-radius: 14px 14px 4px 14px;
  padding: 0.6rem 0.85rem;
}
/* 气泡内的 markdown 直接继承白色，避免被全局深色 .markdown-body 规则覆盖 */
.user-bubble :deep(.markdown-body),
.user-bubble :deep(.markdown-body h1),
.user-bubble :deep(.markdown-body h2),
.user-bubble :deep(.markdown-body h3),
.user-bubble :deep(.markdown-body h4),
.user-bubble :deep(.markdown-body p),
.user-bubble :deep(.markdown-body li),
.user-bubble :deep(.markdown-body blockquote) {
  color: #fff;
}
.user-bubble :deep(.markdown-body a) {
  color: #fff;
  text-decoration: underline;
}
.user-bubble :deep(.markdown-body code) {
  background: rgba(255, 255, 255, 0.22);
  color: #fff;
}
.user-bubble :deep(.markdown-body pre) {
  background: rgba(15, 23, 42, 0.45);
  color: #e2e8f0;
}
.user-bubble :deep(.markdown-body pre code) {
  color: #e2e8f0;
}
.user-bubble :deep(.markdown-body blockquote) {
  border-left-color: rgba(255, 255, 255, 0.5);
}
.user-bubble :deep(.markdown-body p) {
  margin: 0 0 0.5rem;
}
.user-bubble :deep(.markdown-body > *:last-child) {
  margin-bottom: 0;
}

/* ── 统一内容卡片（思考过程 / 工具调用 / 文本回复）──
   所有响应内容节点共用同一套背景、圆角与内边距，从机制上杜绝「有的有框、有的没框」
   导致的内容错位：背景框自带内边距，框内文字左缘统一落在同一位置，同层级天然对齐。
   语义由头部图标 / 标题区分；用户气泡（自带蓝色背景、右对齐）除外。 */
.msg.type-reasoning,
.msg.type-tool_call,
.msg.type-text:not(.user) {
  background: #f6f8fb;
  border-radius: 10px;
  padding: var(--card-pad-y, 0.3rem) var(--card-pad-x, 0.5rem);
}

/* ── JSON 代码块 ── */
.json {
  margin: 0;
  padding: 0.5rem 0.6rem;
  background: #0f172a;
  color: #e2e8f0;
  border-radius: 8px;
  font-family: 'JetBrains Mono', ui-monospace, monospace;
  font-size: 0.76rem;
  line-height: 1.5;
  overflow-x: auto;
  max-height: 320px;
  white-space: pre;
}
/* 工具请求参数：在深色代码块左侧加一道蓝色强调边，强化「请求」语义 */
.json.req {
  border-left: 3px solid #3b82f6;
}
/* 工具调用：请求 / 响应 均为其可折叠子节点，统一竖向排列（间隔与全局一致） */
.tool-children {
  display: flex;
  flex-direction: column;
  gap: var(--msg-gap, 0.6rem);
}
.json-key {
  color: #7dd3fc;
}
.json-str {
  color: #86efac;
}
.json-num {
  color: #fca5a5;
}
.json-bool {
  color: #c4b5fd;
}
.json-null {
  color: #f9a8d4;
}

/* ── 待用户响应（user_prompt 提问 / 工具确认）── */
.user-prompt {
  border: 1px solid #c7d2fe;
  background: #eef2ff;
  border-radius: 8px;
  padding: 0.6rem 0.7rem;
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}
.user-prompt.answered {
  opacity: 0.7;
}
.up-question {
  display: flex;
  flex-direction: column;
  gap: 0.3rem;
}
.up-header {
  font-size: 0.78rem;
  font-weight: 600;
  color: #4338ca;
}
.up-qtext {
  font-size: 0.82rem;
  color: #1e293b;
}
.up-options {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}
.up-option {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  padding: 0.3rem 0.45rem;
  border: 1px solid #dbe2f0;
  border-radius: 6px;
  background: #fff;
  font-size: 0.8rem;
  cursor: pointer;
}
.up-option.active {
  border-color: #6366f1;
  background: #eef2ff;
}
.up-opt-label {
  font-weight: 500;
  color: #1e293b;
}
.up-opt-desc {
  font-size: 0.74rem;
  color: #64748b;
}
.up-option-other {
  gap: 0.35rem;
}
.up-other-input {
  flex: 1;
  border: 1px solid #dbe2f0;
  border-radius: 4px;
  padding: 0.2rem 0.4rem;
  font-size: 0.78rem;
}
.up-submit {
  align-self: flex-start;
  background: #4f46e5;
  color: #fff;
  border: none;
  border-radius: 6px;
  padding: 0.35rem 0.9rem;
  font-size: 0.8rem;
  cursor: pointer;
}
.up-submit:disabled {
  background: #94a3b8;
  cursor: default;
}
.up-confirm {
  display: flex;
  flex-direction: column;
  gap: 0.3rem;
}
.up-confirm-tool {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}
.up-tool-name {
  font-size: 0.85rem;
  font-weight: 600;
  color: #1e293b;
  font-family: 'JetBrains Mono', ui-monospace, monospace;
}
.up-risk {
  font-size: 0.68rem;
  padding: 0.05rem 0.4rem;
  border-radius: 999px;
  background: #e2e8f0;
  color: #475569;
}
.up-risk.high {
  background: #fee2e2;
  color: #b91c1c;
}
.up-risk.medium {
  background: #fef3c7;
  color: #92400e;
}
.up-confirm-desc {
  font-size: 0.8rem;
  color: #334155;
}
.up-args {
  margin: 0;
  padding: 0.5rem 0.6rem;
  background: #0f172a;
  color: #e2e8f0;
  border-radius: 8px;
  font-family: 'JetBrains Mono', ui-monospace, monospace;
  font-size: 0.74rem;
  line-height: 1.5;
  overflow-x: auto;
  max-height: 240px;
  white-space: pre;
}
.up-confirm-btns {
  display: flex;
  gap: 0.5rem;
}
.up-approve,
.up-reject {
  flex: 1;
  padding: 0.35rem;
  border-radius: 6px;
  font-size: 0.8rem;
  cursor: pointer;
  border: 1px solid transparent;
}
.up-approve {
  background: #16a34a;
  color: #fff;
}
.up-approve:disabled,
.up-reject:disabled {
  cursor: default;
  opacity: 0.7;
}
.up-reject {
  background: #fff;
  border-color: #fca5a5;
  color: #b91c1c;
}

/* ── 错误 ── */
.error-box {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  background: #fef2f2;
  border: 1px solid #fecaca;
  border-radius: 8px;
  padding: 0.45rem 0.6rem;
  font-size: 0.8rem;
  color: #b91c1c;
}
.err-icon {
  flex-shrink: 0;
}
.err-text {
  flex: 1;
  word-break: break-word;
}
.retry {
  flex-shrink: 0;
  background: #fff;
  border: 1px solid #fca5a5;
  color: #b91c1c;
  border-radius: 6px;
  padding: 0.2rem 0.6rem;
  font-size: 0.76rem;
  cursor: pointer;
}

/* ── 工具失败补充参数 UI ── */
.tool-actions {
  display: flex;
  gap: 0.4rem;
}
.supply {
  background: #fff;
  border: 1px solid #c7d2fe;
  color: #4338ca;
  border-radius: 6px;
  padding: 0.2rem 0.6rem;
  font-size: 0.76rem;
  cursor: pointer;
}
.supply:hover {
  background: #eef2ff;
}
.supply-form {
  display: flex;
  flex-direction: column;
  gap: 0.35rem;
  background: #f6f8fb;
  border: 1px solid #dbe2f0;
  border-radius: 8px;
  padding: 0.5rem 0.6rem;
}
.supply-textarea {
  width: 100%;
  box-sizing: border-box;
  border: 1px solid #dbe2f0;
  border-radius: 6px;
  padding: 0.35rem 0.45rem;
  font-family: 'JetBrains Mono', ui-monospace, monospace;
  font-size: 0.76rem;
  line-height: 1.5;
  resize: vertical;
  outline: none;
}
.supply-textarea:focus {
  border-color: #6366f1;
}
.supply-submit {
  align-self: flex-start;
  background: #4f46e5;
  color: #fff;
  border: none;
  border-radius: 6px;
  padding: 0.3rem 0.9rem;
  font-size: 0.78rem;
  cursor: pointer;
}
.supply-submit:hover {
  background: #4338ca;
}
</style>
