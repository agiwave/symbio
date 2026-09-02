<!--
  对话消息渲染（Content / Composite 分型模型 · Turn 响应分组 + 统一折叠式节点）

  设计要点（对齐 Claude / Codex 类智能体会话流，v2 重设计）：
  - **根级助手 Turn = 响应分组（透明容器）**，三种形态：
      ① 等待骨架：Turn 已创建但无任何子节点且运行中 → 三点脉动 +「正在思考…」；
      ② 子节点直排：思考/正文/工具出现后容器完全隐藏（无头部、无外框），子节点直接纵排；
      ③ 组级错误条：Turn 失败 →「重试」入口（思考/正文/工具请求的失败都归 Turn 重试，
         resume action=retry_turn：删除响应子树 → 重新走 LLM 请求）。
    子会话 Turn（role=tool）不适用此形态，保留「↳ 子智能体」折叠节点，作为工具「过程」段。
  - **统一折叠式节点**（内容节点 + 工具节点）：可点击折叠的「头部」（图标 + 标题 + 状态标签）
    + 折叠体。折叠策略：
      · 用户消息 / 待审批 → 默认展开；
      · 思考 → **始终单行**（流式中 = 「思考中…」呼吸动效；完成后 = 「思考」+ 摘要预览）；
      · 工具调用 → **始终单行**（名称 + 状态标签；失败红色标签 + 悬停 ↻ 就地重试），
        例外：内含待审批子节点 / 可补充参数时展开；
      · 正文（助手文本）→ 始终展开；工具结果 / 子 agent 内部文本 → 收起；
      · 深层级（depth ≥ 3）一律收起；用户手动切换后以其选择为准。
  - **工具调用三段式卡片**（展开后）：请求（参数 JSON）/ 过程（子会话实时流，无则整段隐藏）/
    结果（工具返回 + 审批提问），三段各自独立响应流。
  - **失败重试两级分派**（由 ModelChatPanel.handleRetry 按 msg.type 路由）：
      · tool_call 失败 → resume retry（删除失败结果 → 原参数重执行工具）；
      · 其余失败（Turn 及其叶子）→ resume retry_turn（删除响应子树 → 重新 LLM 请求）。
  - 层级（无限递归的分形会话流）只靠「缩进 + 左侧引导竖线」表达（depth ≥ 2），
    纵向间隔全由单一变量 --msg-gap 驱动。
  - 失败终态只信服务端（persist_failure 广播 Failed Update），前端不做启发式标记。
-->
<template>
  <!-- 根级助手 Turn：响应分组（透明容器，对齐 Claude / Codex 会话流）
       三种形态：
       ① 等待骨架：Turn 已创建但尚无任何子节点且运行中 → 「正在思考…」动效
       ② 子节点直排：思考/正文/工具出现后容器完全隐藏，子节点直接纵排
       ③ 组级错误条：Turn 失败 → 重试入口（思考/正文/工具请求的失败都归 Turn 重试） -->
  <div v-if="isRootTurn" class="msg turn-group" :class="[statusClass]">
    <span class="turn-actions" @click.stop>
      <button class="node-act" title="删除" @click.stop="emit('delete', node.id)">🗑</button>
    </span>
    <!-- ① 等待态 -->
    <div v-if="isTurnPending" class="turn-pending">
      <span class="turn-pending-dots"><span /><span /><span /></span>
      <span class="turn-pending-text">{{ agentName }} 正在思考…</span>
    </div>
    <!-- ② 子节点直排 + ③ 组级错误条 -->
    <template v-else>
      <MessageNode
        v-for="child in node.children"
        :key="child.id"
        :node="child"
        :depth="(depth ?? 0) + 1"
        parent-type="turn"
        @retry="emit('retry', $event)"
        @delete="emit('delete', $event)"
        @edit="emit('edit', $event)"
      />
      <div v-if="isFailed" class="error-box turn-error">
        <span class="err-icon">⚠</span>
        <span class="err-text">{{ errorText }}</span>
        <button class="retry" @click="emit('retry', node.id)">重试</button>
      </div>
    </template>
  </div>

  <!-- 其余节点：统一折叠式节点 -->
  <div v-else class="msg" :class="[typeClass, statusClass, isUser ? 'user' : '', depth && depth > 1 ? 'nested' : '']">
    <!-- 统一头部：所有节点一致的可点击折叠栏（直接文本回复内联，不重复头部） -->
    <div v-if="!isResponseText" class="node-head" :class="headClass" @click="toggle">
      <span class="node-icon">{{ icon }}</span>
      <span class="node-title">{{ title }}</span>
      <span v-if="isHeartbeat" class="node-heartbeat" title="系统心跳任务自动发送">♥ 心跳</span>
      <!-- 收起态展示单行摘要（深层级 / 子步骤默认收起时，让用户无需展开即知内容） -->
      <span v-if="!effectiveOpen && summaryPreview" class="node-preview">{{ summaryPreview }}</span>
      <span v-if="statusTag" class="node-tag" :class="tagClass">{{ statusTag }}</span>
      <span v-if="isStreaming && !isReasoning && !isToolCall" class="node-live">回复中…</span>
      <!-- 悬停操作：用户消息可编辑；失败工具可就地重试；仅 root 级节点可删除 -->
      <span class="node-actions" @click.stop>
        <button v-if="isUser" class="node-act" title="编辑" @click.stop="emit('edit', node.id)">✎</button>
        <button v-if="canRetry" class="node-act" title="重试此工具" @click.stop="emit('retry', node.id)">↻</button>
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

      <!-- 文本 / 工具请求 / 工具返回。
           失败呈现（要求6/7 的分派）：
           · Turn 子节点的失败 → 由根级 Turn 组错误条统一呈现（isResponseText 不再重复报错）；
           · 工具结果的失败 → 此处仅显示错误文本，重试入口在工具行（↻）与工具卡内。 -->
      <template v-else-if="isTextLike">
        <div v-if="isFailed && !isResponseText" class="error-box">
          <span class="err-icon">⚠</span>
          <span class="err-text">{{ errorText }}</span>
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

      <!-- 组合节点（ToolCall）：三段式卡片（请求 / 过程 / 结果），三段各自独立响应流。
           · 请求：ToolCall 自带参数（前端合成子节点，JSON 高亮）；
           · 过程：子会话 Turn（role=tool），仅子会话类工具存在，无则整段隐藏；
           · 结果：工具返回文本（JSON）与 user_prompt（审批/提问）。
           Turn 的组级呈现（等待骨架/透明分组/组级重试）见模板顶部 isRootTurn 分支。 -->
      <template v-else>
        <div v-if="isToolCall" class="tool-sections">
          <!-- 请求 -->
          <div v-if="toolRequestNode" class="tool-section">
            <div class="ts-label">请求</div>
            <MessageNode
              :node="toolRequestNode"
              :depth="(depth ?? 0) + 1"
              parent-type="tool_call"
              @retry="emit('retry', $event)"
              @delete="emit('delete', $event)"
              @edit="emit('edit', $event)"
            />
          </div>
          <!-- 过程：子会话实时流（有些工具有，有些没有） -->
          <div v-if="processTurns.length" class="tool-section">
            <div class="ts-label">过程</div>
            <MessageNode
              v-for="t in processTurns"
              :key="t.id"
              :node="t"
              :depth="(depth ?? 0) + 1"
              parent-type="tool_call"
              @retry="emit('retry', $event)"
              @delete="emit('delete', $event)"
              @edit="emit('edit', $event)"
            />
          </div>
          <!-- 结果：工具返回 / 审批提问 -->
          <div v-if="resultChildren.length" class="tool-section">
            <div class="ts-label">结果</div>
            <MessageNode
              v-for="r in resultChildren"
              :key="r.id"
              :node="r"
              :depth="(depth ?? 0) + 1"
              parent-type="tool_call"
              @retry="emit('retry', $event)"
              @delete="emit('delete', $event)"
              @edit="emit('edit', $event)"
            />
          </div>

          <!-- 工具级失败：重试此工具（不动 Turn —— 与 Turn 级重试是两个不同粒度） -->
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
// ToolCall 实际渲染的子节点分组（三段式）：
// 请求 = 前端合成节点（toolRequestNode）；过程 = 子会话 Turn；结果 = 其余（工具返回 / user_prompt）
const processTurns = computed<ChatMessage[]>(() =>
  (props.node.children || []).filter((c) => c.type === 'turn'),
)
const resultChildren = computed<ChatMessage[]>(() =>
  (props.node.children || []).filter((c) => c.type !== 'turn'),
)
// 是否为合成「请求」子节点（用于标题 / 图标 / 渲染分支的特判）
const isToolRequest = computed(() => !!(props.node.meta as Record<string, any> | undefined)?.__toolRequest)
// 其余（非用户 / 非 Turn / 非 ToolCall / 非 Reasoning / 非 UserPrompt）：文本 / 请求 / 返回等叶子内容节点
const isTextLike = computed(
  () => !isUser.value && !isTurn.value && !isToolCall.value && !isReasoning.value && !isUserPrompt.value,
)
// 子会话响应：role=Tool 的 Turn，即「某个工具内部的流模式响应」（≈主会话 agent 响应）
const isSubSession = computed(() => isTurn.value && role.value === 'tool')
// 根级助手 Turn：响应分组容器（等待骨架 / 透明分组 / 组级错误条）。
// 仅根级助手回合走此形态；子会话 Turn（role=tool）保留「↳ 子智能体」折叠节点形态，
// 作为工具「过程」段的嵌套流展示。
const isRootTurn = computed(() => isTurn.value && role.value !== 'tool')
// 等待态：Turn 已广播但尚无任何子节点且仍在运行 → 显示「正在思考…」骨架
const isTurnPending = computed(
  () => isRootTurn.value && isStreaming.value && !(props.node.children || []).length,
)
// Turn 的直接**正文**子节点：内联展示（无头部，由 Turn 分组直接纵排承载）。
// 仅限 text——思考节点必须保留单行行头（新设计：思考始终单行可折叠），
// 否则流式思考会以裸 Markdown 块呈现，破坏「思考中…」动效与折叠交互。
const isResponseText = computed(
  () => props.parentType === 'turn' && type.value === 'text',
)

const isStreaming = computed(() => status.value === 'streaming')
const isFailed = computed(() => status.value === 'failed')
const isWaiting = computed(() => status.value === 'waiting_user_action')

const typeClass = computed(() => `type-${type.value}`)
const statusClass = computed(() => `status-${status.value}`)

// ── 折叠状态（统一：所有节点都可点击头部折叠）───────────────
// 折叠默认态（对齐行业智能体会话流）：
//   · 用户消息 / 待审批 → 展开
//   · 思考：**始终单行**（流式中 = 「思考中…」动效提示，完成后 = 「思考」单行摘要）
//   · 工具调用：**始终单行**（名称 + 状态标签），除非内含待审批子节点（否则审批入口被折叠隐藏）
//     或可补充参数（需要操作入口）
//   · 正文（助手文本）→ 始终展开；工具结果/子 agent 内部文本 → 收起
//   · 深层级（depth ≥ 3）一律收起为单行摘要
//   · 用户手动点击后以其选择为准（userToggled 优先）。
const DEEP_COLLAPSE_LEVEL = 3
const isDeep = computed(() => (props.depth ?? 0) >= DEEP_COLLAPSE_LEVEL)
const defaultOpen = computed(() => {
  const st = status.value
  // 待用户响应（审批/提问）→ 始终展开（用户需看到并操作）
  if (st === 'waiting_user_action') return true
  if (role.value === 'user') return true
  // 工具调用：单行为主；例外 = 内含待审批子节点（否则审批入口被折叠隐藏）
  // 或可恢复失败（会话因该失败暂停，需要操作入口，后端 recoverable 标记）
  if (isToolCall.value) {
    if ((props.node.children || []).some((c) => c.status === 'waiting_user_action')) return true
    const meta = props.node.meta as Record<string, any> | undefined
    if (meta?.recoverable) return true
    return false
  }
  // 思考：始终单行（不展开）
  if (isReasoning.value) return false
  // 文本：助手正文展开；工具结果/子 agent 内部文本收起
  if (type.value === 'text') return role.value !== 'tool'
  if (st === 'streaming' || st === 'failed') return true
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
  // 思考：流式中「思考中…」+ 头部动效；完成后「思考」单行
  if (isReasoning.value) return isStreaming.value ? '思考中…' : '思考'
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
  thinking: isReasoning.value && isStreaming.value,
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
  if (!isFailed.value || !isToolCall.value) return false
  // 仅「会话因该失败而暂停」的可恢复失败才可就地重试（meta.recoverable，服务端标记）：
  // resume 在会话忙碌时会拒绝；auto 模式下运行中的工具失败 = 信息性（错误结果已喂给
  // LLM 继续处理），不显示重试。思考/正文/工具请求的失败归 Turn 级重试（组级错误条）。
  const meta = props.node.meta as Record<string, any> | undefined
  return meta?.recoverable === true
})
// 可恢复的工具失败且 failure_kind='error' → 可补充参数重新执行
const canSupply = computed(
  () =>
    canRetry.value &&
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
  border-left: 2px solid var(--color-nested-line);
}

/* ── 根级 Turn 响应分组（透明容器）──
   容器本身无任何视觉框：子节点直排，仅作为等待骨架与组级错误条的挂载点。 */
.msg.turn-group {
  position: relative;
  display: flex;
  flex-direction: column;
  gap: var(--msg-gap, 0.6rem);
}
/* 悬停删除入口：默认隐藏，悬停整组时右上角浮现 */
.msg.turn-group .turn-actions {
  position: absolute;
  top: -0.2rem;
  right: 0;
  display: none;
  z-index: 1;
}
.msg.turn-group:hover .turn-actions {
  display: inline-flex;
}
/* 等待骨架：Turn 已创建但尚无子节点 → 三点脉动 + 「正在思考…」 */
.turn-pending {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.35rem 0.1rem;
}
.turn-pending-dots {
  display: inline-flex;
  gap: 4px;
}
.turn-pending-dots span {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--color-primary);
  animation: turn-pulse 1.2s infinite ease-in-out;
}
.turn-pending-dots span:nth-child(2) {
  animation-delay: 0.2s;
}
.turn-pending-dots span:nth-child(3) {
  animation-delay: 0.4s;
}
@keyframes turn-pulse {
  0%,
  80%,
  100% {
    opacity: 0.25;
    transform: scale(0.85);
  }
  40% {
    opacity: 1;
    transform: scale(1);
  }
}
.turn-pending-text {
  font-size: 0.8rem;
  color: var(--color-text-muted, #94a3b8);
}
/* 思考行流式动效：标题呼吸闪烁（单行态，不展开内容） */
.node-head.thinking .node-title {
  animation: think-pulse 1.4s ease-in-out infinite;
}
@keyframes think-pulse {
  0%,
  100% {
    opacity: 0.55;
  }
  50% {
    opacity: 1;
  }
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
  background: var(--color-msg-card);
  padding: 0.1rem 0.3rem;
  border-radius: 4px;
  font-family: 'JetBrains Mono', ui-monospace, monospace;
  font-size: 0.85em;
}
.msg :deep(.markdown-body pre) {
  background: var(--color-code-bg);
  color: var(--color-code-fg);
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
  background: var(--color-chip-bg);
  color: var(--color-chip-fg);
}
.node-head.user .node-icon {
  background: var(--color-chip-user-bg);
  color: var(--color-chip-user-fg);
}
.node-head.sub .node-icon {
  background: var(--color-chip-sub-bg);
  color: var(--color-chip-sub-fg);
}
.node-head.tool .node-icon {
  background: var(--color-chip-tool-bg);
  color: var(--color-chip-tool-fg);
}
.node-head.reasoning .node-icon {
  background: var(--color-chip-reasoning-bg);
  color: var(--color-chip-reasoning-fg);
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
  color: var(--color-chip-sub-fg);
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
  color: var(--color-chip-tool-fg);
}
.node-head.sub .node-live {
  color: var(--color-chip-sub-fg);
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
  color: var(--color-chip-fg);
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
  background: var(--color-tag-sub-bg);
  color: var(--color-tag-sub-fg);
}
.node-tag.warn {
  background: var(--color-tag-warn-bg);
  color: var(--color-tag-warn-fg);
}
.node-tag.err {
  background: var(--color-tag-err-bg);
  color: var(--color-tag-err-fg);
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
  background: var(--color-user-bubble);
  color: var(--color-user-bubble-fg);
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

/* ── 响应流排版（现代扁平 agent 风格）──
   助手正文（type-text:not(.user)）采用无框扁平设计：直接落在聊天背景上，
   仅保留与节点头一致的左右内边距，靠纵向间距分组，去掉背景卡片以减少视觉装饰。
   思考过程 / 工具调用保留极淡卡片 + 弱化描边，仅作内容分组，保持清晰不喧宾夺主。 */
.msg.type-text:not(.user) {
  padding: var(--card-pad-y, 0.3rem) var(--card-pad-x, 0.5rem);
}
.msg.type-reasoning {
  background: var(--color-msg-card);
  border-radius: 10px;
  padding: var(--card-pad-y, 0.3rem) var(--card-pad-x, 0.5rem);
}
.msg.type-tool_call {
  background: var(--color-msg-card);
  border: 1px solid var(--color-msg-card-border);
  border-radius: 10px;
  padding: var(--card-pad-y, 0.3rem) var(--card-pad-x, 0.5rem);
}

/* ── JSON 代码块 ── */
.json {
  margin: 0;
  padding: 0.5rem 0.6rem;
  background: var(--color-code-bg);
  color: var(--color-code-fg);
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
  border-left: 3px solid var(--color-user-bubble);
}
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
.ts-label {
  font-size: 0.68rem;
  font-weight: 600;
  letter-spacing: 0.08em;
  color: var(--color-text-muted, #94a3b8);
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
  border: 1px solid var(--color-prompt-border);
  background: var(--color-prompt-bg);
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
  color: var(--color-chip-tool-fg);
}
.up-qtext {
  font-size: 0.82rem;
  color: var(--color-text);
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
  border: 1px solid var(--color-option-border);
  border-radius: 6px;
  background: var(--color-option-bg);
  font-size: 0.8rem;
  cursor: pointer;
}
.up-option.active {
  border-color: var(--color-option-active-border);
  background: var(--color-option-active-bg);
}
.up-opt-label {
  font-weight: 500;
  color: var(--color-text);
}
.up-opt-desc {
  font-size: 0.74rem;
  color: var(--color-text-muted);
}
.up-option-other {
  gap: 0.35rem;
}
.up-other-input {
  flex: 1;
  border: 1px solid var(--color-option-border);
  border-radius: 4px;
  padding: 0.2rem 0.4rem;
  font-size: 0.78rem;
}
.up-submit {
  align-self: flex-start;
  background: var(--color-primary);
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
  color: var(--color-text);
  font-family: 'JetBrains Mono', ui-monospace, monospace;
}
.up-risk {
  font-size: 0.68rem;
  padding: 0.05rem 0.4rem;
  border-radius: 999px;
  background: var(--color-chip-bg);
  color: var(--color-chip-fg);
}
.up-risk.high {
  background: var(--color-tag-err-bg);
  color: var(--color-tag-err-fg);
}
.up-risk.medium {
  background: var(--color-tag-warn-bg);
  color: var(--color-tag-warn-fg);
}
.up-confirm-desc {
  font-size: 0.8rem;
  color: var(--color-text-secondary);
}
.up-args {
  margin: 0;
  padding: 0.5rem 0.6rem;
  background: var(--color-code-bg);
  color: var(--color-code-fg);
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
  background: var(--color-option-bg);
  border-color: var(--color-error-border);
  color: var(--color-error-fg);
}

/* ── 错误 ── */
.error-box {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  background: var(--color-error-bg);
  border: 1px solid var(--color-error-border);
  border-radius: 8px;
  padding: 0.45rem 0.6rem;
  font-size: 0.8rem;
  color: var(--color-error-fg);
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
  background: var(--color-option-bg);
  border: 1px solid var(--color-error-border);
  color: var(--color-error-fg);
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
  background: var(--color-supply-bg);
  border: 1px solid var(--color-prompt-border);
  color: var(--color-chip-tool-fg);
  border-radius: 6px;
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
  border-radius: 8px;
  padding: 0.5rem 0.6rem;
}
.supply-textarea {
  width: 100%;
  box-sizing: border-box;
  border: 1px solid var(--color-supply-border);
  border-radius: 6px;
  padding: 0.35rem 0.45rem;
  font-family: 'JetBrains Mono', ui-monospace, monospace;
  font-size: 0.76rem;
  line-height: 1.5;
  resize: vertical;
  outline: none;
}
.supply-textarea:focus {
  border-color: var(--color-primary);
}
.supply-submit {
  align-self: flex-start;
  background: var(--color-primary);
  color: #fff;
  border: none;
  border-radius: 6px;
  padding: 0.3rem 0.9rem;
  font-size: 0.78rem;
  cursor: pointer;
}
.supply-submit:hover {
  background: var(--color-primary-dark);
}
</style>
