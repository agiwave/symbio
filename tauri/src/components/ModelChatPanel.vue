<template>
  <div class="model-chat-panel">
    <!-- 错误 banner（onMounted silent 错误、模型/provider 加载失败等） -->
    <Transition name="banner">
      <div v-if="initError" class="init-error-banner" role="alert">
        <span class="banner-icon">⚠</span>
        <span class="banner-text">{{ initError }}</span>
        <button class="banner-close" @click="initError = null" title="关闭">×</button>
      </div>
    </Transition>

    <!-- 会话级错误条（兜底：无 Failed Turn 节点、但会话整体因错误中止时展示；
         有 Failed Turn 时错误由根级 Turn 节点承载，不在此重复显示） -->
    <Transition name="banner">
      <div v-if="sessionError" class="session-error-banner" role="alert">
        <span class="banner-icon">⚠</span>
        <span class="banner-text">{{ sessionError }}</span>
        <button class="banner-retry" @click="handleSessionRetry">重试</button>
        <button
          class="banner-close"
          @click="sessionsStore.setSessionError(props.sessionId, null)"
          title="关闭"
        >×</button>
      </div>
    </Transition>

    <!-- 编辑单条消息的浮层 -->
    <Transition name="banner">
      <div v-if="editing" class="edit-overlay" @click.self="editing = null">
        <div class="edit-box">
          <div class="edit-title">编辑消息</div>
          <textarea
            v-model="editing.content"
            class="edit-area"
            :placeholder="editing.isJson ? 'JSON 内容' : '消息内容'"
          />
          <div class="edit-btns">
            <button class="edit-save" @click="saveEdit">保存</button>
            <button class="edit-cancel" @click="editing = null">取消</button>
          </div>
        </div>
      </div>
    </Transition>

    <!-- 消息历史区域 -->
    <div class="chat-messages" ref="messagesRef" @scroll="handleScroll">
      <div v-if="messageTree.length === 0 && !initError" class="empty-chat">
        <p>开始与 AI 助手对话</p>
        <p class="empty-hint">输入消息后按 Enter 发送，Shift+Enter 换行</p>
      </div>

      <MessageNode
        v-for="(node, i) in messageTree"
        :key="node.id"
        :node="node"
        :depth="0"
        :is-last="i === messageTree.length - 1"
        @retry="handleRetry"
        @delete="handleDelete"
        @edit="handleEdit"
      />
    </div>

    <!-- 输入控制区域 -->
    <div class="chat-controls">
      <!-- 上下文预览栏 -->
      <ChatContextBar
        :context="context"
        :visible="hasContext"
        :session-id="sessionId"
      />

      <!-- 输入区域 -->
      <ChatInputArea
        ref="inputAreaRef"
        v-model="inputText"
        v-model:attached-images="attachedImages"
        :is-loading="isLoading"
        @submit="handleSendOrAbort"
      />

      <!-- 配置切换栏（工作目录 + 智能体 + 对话模式） -->
      <div class="settings-row">
        <WorkdirPicker
          :session-id="sessionId"
          :workdir="workdir"
          :message-count="messageCount"
        />
        <div class="settings-divider" />
        <ChatSettings
          v-model:agent-id="agentId"
          v-model:available-agents="availableAgents"
          v-model:model-provider-id="modelProviderId"
          v-model:available-model-providers="availableModelProviders"
          :session-id="sessionId"
          @open-heartbeat="showSettings = true"
        />
      </div>
    </div>

    <!-- 会话设置弹窗（心跳任务配置 + 立即执行一次） -->
    <SessionSettingsDialog
      v-if="showSettings && activeSession"
      :session="activeSession"
      @close="showSettings = false"
    />
  </div>
</template>

<script setup lang="ts">
import { ref, nextTick, watch, computed, onMounted, onBeforeUnmount, provide } from 'vue'
import { useChatConnection, type ResumePayload } from '@/composables/useChatConnection'
import { useModelContext, buildContextualMessage, resetModelContext } from '@/composables/useModelContext'
import { useChatScroll } from '@/composables/useChatScroll'
import { type ChatMessage, type MessageContent, type ContentPart, type ChatRole } from '@/services/model'
import type { ImageAttachment } from '@/types'
import { callPlugin } from '@/services/plugin'
import { logger } from '@/utils/logger'
import { useSessionsStore } from '@/stores/sessions'
import { listModelProviders } from '@/services/modelProviders'
import { listResources } from '@/services/resources'
import type { ModelProviderConfig } from '@/schemas/model_providers'

import MessageNode from './MessageNode.vue'
import ChatContextBar from './chat/ChatContextBar.vue'
import ChatInputArea from './chat/ChatInputArea.vue'
import ChatSettings from './chat/ChatSettings.vue'
import WorkdirPicker from './chat/WorkdirPicker.vue'
import SessionSettingsDialog from '@/components/session/SessionSettingsDialog.vue'

// Props（多会话缩略窗口架构下，ModelChatPanel 只接收 sessionId）
const props = withDefaults(defineProps<{
  sessionId: string
  onSendComplete?: () => void
  /** 是否把 Model 上下文（文件 / 选区）拼到消息里并显示 ChatContextBar。默认 true */
  showContext?: boolean
  /** 内部 useChatConnection 的 isLoading 变化回调（用于跨组件状态同步） */
  onLoadingChange?: (loading: boolean) => void
}>(), {
  showContext: true
})

// --- 状态管理 ---
const messagesRef = ref<HTMLElement | null>(null)
const inputAreaRef = ref<any>(null)
const inputText = ref('')
const attachedImages = ref<ImageAttachment[]>([])
/** 当前选定的智能体 id；null = 不使用 Agent（纯工具模式，会话由 session 编排） */
const agentId = ref<string | null>(null)
const availableAgents = ref<any[]>([])
const modelProviderId = ref<string>('')
const availableModelProviders = ref<ModelProviderConfig[]>([])

// 初始化阶段错误（顶部 banner 显示，**不再静默**）
const initError = ref<string | null>(null)
// 编辑单条消息的浮层状态
const editing = ref<{ id: string; content: string; isJson: boolean } | null>(null)

/**
 * 抑制 mount 阶段从 session.metadata 加载 agent_id/provider_id 时触发的 watcher 回写。
 *
 * 问题：onMounted 里 `agentId.value = sessionMeta.agent_id` 会触发 watcher，
 * watcher 又调 session/update 把同样的值写回 metadata —— 冗余的网络请求。
 *
 * 机制：watcher 用 `flush: 'sync'` 同步触发，开头检查 `loadedFromMeta`：
 * - mount 阶段从 metadata 加载时 `loadedFromMeta=false` → watcher 直接 return
 * - sessionMeta 块结束后置 `loadedFromMeta=true` → 后续默认值设置
 *   （default_provider_id / default_agent_id / 自动选第一个 agent）触发的 watcher 正常持久化
 */
const loadedFromMeta = ref(false)

function showInitError(prefix: string, err: any) {
  const msg = err instanceof Error ? err.message : String(err || '未知错误')
  initError.value = `${prefix}：${msg}`
  logger.warn('ModelChatPanel', `${prefix}: ${msg}`, err)
}

// 同步 Agent 变更至 session metadata 中（null = 清除绑定：纯工具模式）
// flush: 'sync' 确保 loadedFromMeta 守卫在 mount 阶段同步生效（见 loadedFromMeta 注释）
watch(agentId, async (newVal) => {
  if (!loadedFromMeta.value) return
  if (props.sessionId) {
    try {
      await callPlugin('session/update', {
        session_id: props.sessionId,
        metadata: { agent_id: newVal || null }
      }, undefined, { session_id: props.sessionId })
    } catch (e) {
      logger.error('ModelChatPanel', 'Failed to update session agent', e)
    }
  }
}, { flush: 'sync' })

// 同步 Model Provider 变更至 session metadata
// flush: 'sync' 确保 loadedFromMeta 守卫在 mount 阶段同步生效（见 loadedFromMeta 注释）
watch(modelProviderId, async (newVal) => {
  if (!loadedFromMeta.value) return
  if (props.sessionId) {
    try {
      await callPlugin('session/update', {
        session_id: props.sessionId,
        metadata: { provider_id: newVal || null }
      }, undefined, { session_id: props.sessionId })
    } catch (e) {
      logger.error('ModelChatPanel', 'Failed to update session provider_id', e)
    }
  }
}, { flush: 'sync' })

// --- Composables ---
const { context, pendingInputInject } = useModelContext()
const { scrollToBottom, smartScroll, handleScroll } = useChatScroll(messagesRef)

const chat = useChatConnection({
  sessionId: props.sessionId,
  onSendComplete: props.onSendComplete,
})

// 向下级组件（MessageNode 内联 user_prompt 表单 / 失败 ToolCall 重试/补充）提供
// 会话恢复接口（retry_turn/retry/approve/reject/supply/answer 统一入口）。
// 内部走统一 chat/send 接口的 resume 分支：target_id 为稳定锚点（Failed Turn 或 ToolCall），
// targetSessionId 用于子会话工具调用的恢复路由（子智能体的 user_prompt 通过
// parent_session_id 指回子会话）。
// 注入当前选中的 modelProviderId，确保 resume 时也使用窗口选择的 Provider
// （与 send 同级别，由 useChatConnection 透传到后端 resolve_session_params）。
provide('resume', (payload: ResumePayload) => {
  chat.resume({
    ...payload,
    providerId: modelProviderId.value || undefined,
  })
})

const { isLoading } = chat
const messageTree = chat.messageTree

// 全局 store 引用：用于读取 workdir / messages
const sessionsStore = useSessionsStore()

// 会话设置弹窗（心跳任务配置）。由 ChatSettings 的「心跳」按钮触发打开。
const showSettings = ref(false)
const activeSession = computed(() =>
  sessionsStore.list.find(s => s.id === props.sessionId) ?? null,
)

// --- 计算属性 ---
const hasContext = computed(() => {
  if (!context.value) return false
  return !!(context.value.filePath || context.value.selectedText)
})

// 监听"输入注入"请求：把外部组件（文件编辑器等）发来的文本
// 写入当前会话的 inputText，并聚焦输入框。
watch(
  () => pendingInputInject.value,
  (req) => {
    if (!req) return
    if (req.sessionId !== props.sessionId) return

    // 已有内容时换行拼接，避免覆盖用户正在输入的内容
    const cur = inputText.value ?? ''
    const sep = cur.trim().length > 0 ? '\n\n' : ''
    inputText.value = cur + sep + req.text

    // 清空 inject 槽位
    pendingInputInject.value = null

    // 聚焦 + 光标定位
    nextTick(() => {
      const ta: HTMLTextAreaElement | undefined = inputAreaRef.value?.textarea?.value
        ?? inputAreaRef.value?.textarea
      if (!ta) return
      ta.focus()
      // focusEnd=false → 把光标放在文本开头，让用户先看到上下文
      const pos = req.focusEnd ? ta.value.length : 0
      try {
        ta.setSelectionRange(pos, pos)
      } catch (_) { /* 某些环境下不支持 */ }
    })
  }
)

// 当前会话的工作目录（用于 WorkdirPicker 显示）
const workdir = computed<string | null>(() => {
  return sessionsStore.activeWorkdir ?? null
})

// 在 store 加载消息后用 store 自身的 messages 数组作为权威来源，
// 避免 ModelChatPanel 内 messageTree 与 WorkdirPicker 看到的 messageCount 不一致
  const messageCount = computed(() => {
    return sessionsStore.getSessionMessages(props.sessionId).length
  })

  // 会话级错误状态（"错误是状态、不是节点"的兜底展示）：仅当没有任何 Failed Turn 节点、
  // 但会话整体因错误中止（transport 级失败 / send 在首帧前失败）时非空。此时没有"造成
  // 中止的节点"可挂重试，错误作为会话级状态展示在会话错误条，并许可重试。
  const sessionError = computed(() => sessionsStore.getSessionError(props.sessionId))

  /** 会话级错误重试：重新发送用户最后一条消息（复用其 id 避免乐观消息重复节点）。
   *  仅用于"无 Failed Turn 节点"的兜底错误；有 Failed Turn 时错误由其节点承载、走 handleRetry。 */
  function handleSessionRetry() {
    const msgs = sessionsStore.getSessionMessages(props.sessionId)
    const lastUser = [...msgs].reverse().find((m) => m.role === 'user')
    sessionsStore.setSessionError(props.sessionId, null)
    if (!lastUser) return
    const retryMsg: ChatMessage = {
      id: lastUser.id, // 复用原 user 消息 id：重发而非新建节点
      role: 'user',
      content: lastUser.content,
      timestamp: Date.now(),
      agent_id: agentId.value ?? undefined,
    }
    chat.send(retryMsg, agentId.value, modelProviderId.value || undefined)
  }

  // --- 方法 ---
onMounted(async () => {
  // 清掉旧的 banner
  initError.value = null

  // 并行加载：agents（统一资源协议 resources/list）/ providers
  const results = await Promise.allSettled([
    listResources('agent'),
    listModelProviders()
  ])

  // 1. agents（ResourceSummary → 智能体选项：id/name/description）
  if (results[0].status === 'fulfilled') {
    const list = results[0].value.items
    if (Array.isArray(list)) {
      availableAgents.value = list.map((it) => ({
        id: it.id,
        name: it.name || it.id,
        description: it.description || it.summary || '',
      }))
    }
  } else {
    showInitError('加载智能体列表失败', results[0].reason)
  }

  // 2. session metadata（来自 session/list 缓存）
  const sessionMeta = sessionsStore.list.find(s => s.id === props.sessionId)?.metadata
  let loadedProvider = false
  if (sessionMeta) {
    if (sessionMeta.agent_id) {
      agentId.value = sessionMeta.agent_id
    }
    if (sessionMeta.provider_id) {
      modelProviderId.value = sessionMeta.provider_id
      loadedProvider = true
    }
  }
  // mount 阶段从 metadata 加载完成，放开 watcher 守卫——
  // 后续默认值设置（default_provider_id）触发的 watcher 会正确持久化到 session.metadata。
  loadedFromMeta.value = true

  // 3. model providers
  if (results[1].status === 'fulfilled') {
    const cfg = results[1].value
    availableModelProviders.value = Object.values(cfg.providers ?? {})
    if (!loadedProvider && !modelProviderId.value) {
      modelProviderId.value = cfg.default_provider_id ?? ''
    }
  } else {
    showInitError('加载模型提供商失败', results[1].reason)
  }

  // 4. 校验当前选中的智能体是否仍有效；失效则置空（回到"不使用 Agent"纯工具模式）。
  // 不再强制默认选择智能体——会话默认不绑定 agent，若不选则纯工具模式照常运行；
  // 只有会话 metadata 显式记录了 agent_id 时才会携带（重启/重进会话后沿用上次选择）。
  if (agentId.value && !availableAgents.value.some(a => a.id === agentId.value)) {
    agentId.value = null
  }

  nextTick(() => scrollToBottom())
  // 启动看门狗：检测"卡在处理中但长时间无事件"的会话（目标 2 系统保障）
  startWatchdog()
})

onBeforeUnmount(() => {
  stopWatchdog()
})

// Handle send or abort logic
function handleSendOrAbort() {
  if (isLoading.value) {
    // While loading, only allow abort (no queuing new messages)
    chat.abort()
  } else if (inputText.value.trim() || attachedImages.value.length > 0) {
    handleSend()
  }
}

// 核心发送逻辑
function handleSend() {
  const text = inputText.value.trim()
  const images = [...attachedImages.value]
  if (!text && images.length === 0) return

  const ctx = context.value
  const contextualContent = ctx && (ctx.filePath || ctx.selectedText)
    ? buildContextualMessage(text, ctx)
    : text

  // 构建消息内容
  let content: MessageContent
  if (images.length > 0) {
    const parts: ContentPart[] = []
    if (contextualContent) parts.push({ type: 'text', text: contextualContent })
    images.forEach(img => parts.push({
      type: 'image_url',
      image_url: { url: `data:${img.mimeType};base64,${img.base64}` }
    }))
    content = parts
  } else {
    content = contextualContent
  }

  const userMessage = {
    id: crypto.randomUUID(),
    role: 'user' as const,
    content,
    timestamp: Date.now(),
    agent_id: agentId.value
  }

  // 重置输入状态
  inputText.value = ''
  attachedImages.value.forEach(img => img.thumbnailUrl && URL.revokeObjectURL(img.thumbnailUrl))
  attachedImages.value = []
  inputAreaRef.value?.resetHeight()

  // 消息已发送并附带上下文后，清空 Model 上下文，
  // ChatContextBar 卡片会随之隐藏（避免下一次输入还误带旧选区）
  if (ctx && (ctx.filePath || ctx.selectedText)) {
    resetModelContext()
  }

  nextTick(() => scrollToBottom())

  const chatMessage: ChatMessage = {
    id: userMessage.id,
    role: userMessage.role as ChatRole,
    content: userMessage.content,
    timestamp: userMessage.timestamp
  }

  chat.send(chatMessage, agentId.value, modelProviderId.value || undefined)
}


function handleRetry(messageId: string) {
  const msg = sessionsStore.getSessionMessages(props.sessionId).find(m => m.id === messageId)
  if (!msg) return

  // 工具调用失败 → resume retry（删除旧失败子节点 + 重新执行工具 + 生成新响应）
  if (msg.type === 'tool_call' && msg.status === 'failed') {
    chat.resume({
      targetId: messageId,
      action: 'retry',
    })
    return
  }

  // LLM 失败 → resume retry_turn（删除 Failed Turn 及其所有子孙节点 + 重新走 LLM 请求）
  // 失败节点可能是 Turn 本身，也可能是其下的 Text/Reasoning/Thinking 叶子；
  // 后端 process_retry_turn 要求 target_id 指向 Failed Turn（msg_type=turn），
  // 因此对叶子节点需回溯到父 Turn。
  const turnId = msg.type === 'turn'
    ? messageId
    : (msg.parent_id || messageId)
  chat.resume({
    targetId: turnId,
    action: 'retry_turn',
  })
}

/** 删除单条消息（由 store 调后端持久化）；若为 user 消息，删除后将其内容回填输入框 */
async function handleDelete(messageId: string) {
  // 删除前先取出消息：回填空需要其内容
  const msg = sessionsStore.getSessionMessages(props.sessionId).find(m => m.id === messageId)
  const isUserMsg = msg?.role === 'user'
  const content = msg?.content
  const userText = isUserMsg
    ? (typeof content === 'string'
        ? content
        : (content && typeof content === 'object' && 'text' in content
            ? (content as { text: string }).text
            : ''))
    : ''

  try {
    await sessionsStore.deleteMessage(props.sessionId, messageId)
    // 仅当用户消息被删时，把其内容回填到输入框，方便用户修改后重发（不自动发送）
    if (isUserMsg && userText) {
      const cur = inputText.value ?? ''
      const sep = cur.trim().length > 0 ? '\n\n' : ''
      inputText.value = cur + sep + userText
    }
  } catch (e) {
    logger.error('ModelChatPanel', 'deleteMessage 失败', e)
  }
}

/** 打开编辑浮层：把消息内容载入文本框 */
function handleEdit(messageId: string) {
  const msg = sessionsStore.getSessionMessages(props.sessionId).find(m => m.id === messageId)
  if (!msg) return
  const c = msg.content
  const isJson = typeof c === 'string' && /^[\\[{]/.test(c.trim())
  const text = typeof c === 'string'
    ? c
    : (c && typeof c === 'object' && 'text' in c ? (c as { text: string }).text : '')
  editing.value = { id: messageId, content: text, isJson }
}

/** 保存编辑：更新 store + 持久化到后端 */
async function saveEdit() {
  if (!editing.value) return
  const { id, content } = editing.value
  const msg = sessionsStore.getSessionMessages(props.sessionId).find(m => m.id === id)
  if (!msg) {
    editing.value = null
    return
  }
  const newContent: MessageContent = content
  await sessionsStore.updateMessage(props.sessionId, {
    ...msg,
    content: newContent,
  } as ChatMessage)
  editing.value = null
  nextTick(() => scrollToBottom())
}

// ── 看门狗：会话卡在"处理中"且长时间无业务事件（疑似后台崩溃 / 断流）──
// 每 15s 检查一次：若 is_working 但已超过阈值时间无事件，则把仍在
// streaming/waiting 的消息持久化为 Failed —— 直接出现在对话流中（带内联重试按钮），
// 切回会话仍能看到上次的错误，无需任何顶部 banner。
let watchdogTimer: ReturnType<typeof setInterval> | null = null
function startWatchdog() {
  stopWatchdog()
  watchdogTimer = setInterval(() => {
    const sid = props.sessionId
    if (!sid) return
    const status = sessionsStore.getSessionStatus(sid)
    if (!status.is_working) return
    const reason = sessionsStore.getSessionStaleReason(sid)
    if (!reason) return
    // 把卡死的消息持久化为 Failed（会出现在对话流中，带内联重试），不再用顶部 banner
    sessionsStore.persistStuckFailure(sid, reason).catch((e) => {
      logger.warn('ModelChatPanel', 'persistStuckFailure 失败', e)
    })
  }, 15000)
}
function stopWatchdog() {
  if (watchdogTimer !== null) {
    clearInterval(watchdogTimer)
    watchdogTimer = null
  }
}

// 智能滚动
watch(messageTree, () => nextTick(() => smartScroll()), { deep: true, flush: 'post' })
</script>

<style scoped>
.model-chat-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  overflow: hidden;
  background: var(--color-chat-bg);
}

.chat-messages {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 1rem;
  display: flex;
  flex-direction: column;
  /* ── 统一纵向节奏（机制化保障一致性）──
     所有响应元素之间的纵向间隔都引用同一变量，下游 MessageNode 经 CSS 继承复用，
     杜绝散落的 0.75/0.5/0.4rem 多重间距导致的「忽大忽小」。 */
  --msg-gap: 0.4rem;         /* 统一纵向间隔：顶层节点 / Turn 子元素 / 工具子元素 全部共用（紧凑） */
  --nest-indent: 0.7rem;      /* 嵌套缩进步长（仅 depth>=2 的层级使用，见 MessageNode） */
  --card-pad-x: 0.5rem;       /* 卡片型节点统一左右内边距 */
  gap: var(--msg-gap);
}

.empty-chat {
  text-align: center;
  color: var(--color-text-muted);
  padding: 3rem 1rem;
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.empty-chat p {
  margin: 0;
}

.empty-hint {
  font-size: 0.8rem;
  opacity: 0.7;
}

.chat-controls {
  padding: 0.75rem 1rem;
  border-top: 1px solid var(--color-border);
  background: var(--color-surface);
  flex-shrink: 0;
}

.settings-row {
  display: flex;
  align-items: center;
  gap: 0.25rem;
  flex-wrap: wrap;
  margin-top: 0.5rem;
}

.settings-divider {
  width: 1px;
  height: 1rem;
  background: var(--color-border);
  margin: 0 0.5rem;
  flex-shrink: 0;
}

/* ═══════════════════════════════════════════════════════════
   Init error banner
   ═══════════════════════════════════════════════════════════ */
.init-error-banner {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.5rem 0.85rem;
  background: var(--color-banner-bg);
  border-bottom: 1px solid var(--color-banner-border);
  color: var(--color-banner-fg);
  font-size: 0.82rem;
  flex-shrink: 0;
}

.banner-icon {
  font-size: 1rem;
  flex-shrink: 0;
}

.banner-text {
  flex: 1;
  word-break: break-word;
}

.banner-close {
  background: transparent;
  border: none;
  color: var(--color-banner-fg);
  font-size: 1.2rem;
  line-height: 1;
  cursor: pointer;
  padding: 0 0.3rem;
  border-radius: 0.25rem;
  flex-shrink: 0;
}

.banner-close:hover {
  background: rgba(146, 64, 14, 0.1);
}

/* 会话级错误条（与 init-error-banner 同视觉语言，但语义不同：
   它是"错误是状态、不是节点"的兜底展示，仅当无任何 Failed Turn 节点时出现） */
.session-error-banner {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.5rem 0.85rem;
  background: var(--color-error-bg);
  border: 1px solid var(--color-error-border);
  border-left: 0.1875rem solid var(--color-error-fg);
  color: var(--color-error-fg);
  font-size: 0.82rem;
  flex-shrink: 0;
}

.banner-retry {
  flex-shrink: 0;
  background: var(--color-option-bg);
  border: 1px solid var(--color-error-border);
  color: var(--color-error-fg);
  border-radius: 0.375rem;
  padding: 0.2rem 0.7rem;
  font-size: 0.78rem;
  cursor: pointer;
}

.banner-retry:hover {
  background: var(--color-error-bg);
}

.banner-enter-active,
.banner-leave-active {
  transition: all 0.25s ease;
}

.banner-enter-from,
.banner-leave-to {
  opacity: 0;
  transform: translateY(-0.5rem);
}

/* ── 编辑单条消息的浮层（edit-overlay / edit-box …）──────── */

.edit-overlay {
  position: fixed;
  inset: 0;
  z-index: 100;
  background: rgba(0, 0, 0, 0.45);
  display: flex;
  align-items: center;
  justify-content: center;
}

.edit-box {
  width: min(34rem, 92vw);
  background: var(--surface-overlay);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-2);
  padding: var(--space-4);
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.edit-title {
  font-size: var(--font-size-base);
  font-weight: var(--font-weight-medium);
  color: var(--text-primary);
}

.edit-area {
  min-height: 8rem;
  padding: var(--space-2) var(--space-3);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: var(--surface-sunken);
  color: var(--text-primary);
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: var(--font-size-sm);
  line-height: 1.5;
  resize: vertical;
}

.edit-area:focus {
  outline: none;
  border-color: var(--accent);
}

.edit-btns {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-2);
}

.edit-save,
.edit-cancel {
  padding: var(--space-2) var(--space-4);
  font-size: var(--font-size-sm);
  border-radius: var(--radius-md);
  border: 1px solid var(--border-default);
  cursor: pointer;
}

.edit-save {
  background: var(--accent);
  border-color: var(--accent);
  color: var(--text-on-accent);
}

.edit-save:hover {
  background: var(--accent-hover);
  border-color: var(--accent-hover);
}

.edit-cancel {
  background: transparent;
  color: var(--text-secondary);
}

.edit-cancel:hover {
  background: var(--surface-hover);
}
</style>
