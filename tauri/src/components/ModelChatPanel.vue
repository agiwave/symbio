<template>
  <div class="model-chat-panel">
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
      <div v-if="messageTree.length === 0" class="empty-chat">
        <p>开始与 AI 助手对话</p>
        <p class="empty-hint">输入消息后按 Enter 发送，Shift+Enter 换行</p>
      </div>

      <MessageNode
        v-for="node in messageTree"
        :key="node.id"
        :node="node"
        :depth="0"
        @retry="handleRetry"
        @delete="handleDelete"
        @edit="handleEdit"
      />
    </div>

    <!-- 输入控制区域：输入框 + 选项行由 ChatComposer 唯一装配（不再在此各拼一半） -->
    <div class="chat-controls">
      <ChatComposer
        ref="composerRef"
        v-model="inputText"
        v-model:attached-images="attachedImages"
        :is-loading="isLoading"
        :session-id="props.sessionId"
        @submit="handleSendOrAbort"
      />
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, nextTick, watch, computed, onMounted, onBeforeUnmount, provide } from 'vue'
import { useChatConnection, RESUME_KEY, type ResumePayload } from '@/composables/useChatConnection'
import { useChatScroll } from '@/composables/useChatScroll'
import {
  type ChatMessage,
  type MessageContent,
  type ContentPart,
  type ChatRole,
  messageTextOf,
} from '@/schemas/chat_message'
import type { ImageAttachment } from '@/types'
import { logger } from '@/utils/logger'
import { useSessionsStore } from '@/stores/sessions'
import { isWorkingStatus } from '@/schemas/vdfs'
import { CHAT_ROLE_USER } from '@/schemas/chat_message'
// 消息级判定与业务规则全部来自 registry（本组件不解释消息词表，也不读后端 meta 字段）
import {
  isFailedStatus,
  messageIsEphemeral,
  messageRetryTargetOf,
  messageRoleOf,
  messageStatusOf,
} from '@/registry/messageTypes'

import MessageNode from './MessageNode.vue'
import ChatComposer from './chat/ChatComposer.vue'

// Props（多会话缩略窗口架构下，ModelChatPanel 只接收 sessionId）
const props = defineProps<{
  sessionId: string
  onSendComplete?: () => void
  /** 内部 useChatConnection 的 isLoading 变化回调（用于跨组件状态同步） */
  onLoadingChange?: (loading: boolean) => void
}>()

// --- 状态管理 ---
const messagesRef = ref<HTMLElement | null>(null)
/** 输入区（ChatComposer）：发送后复位高度、排队首条消息回填文本与附件 */
const composerRef = ref<{
  resetHeight: () => void
} | null>(null)
const inputText = ref('')
const attachedImages = ref<ImageAttachment[]>([])
// 编辑单条消息的浮层状态
const editing = ref<{ id: string; content: string; isJson: boolean } | null>(null)

// --- Composables ---
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
// 智能体 / 模型 / 运行模式 / 风险等级等会话参数由后端按 session.metadata 解析
// （写入统一经级联选项机制），故此处不再透传任何前端业务选择。
provide(RESUME_KEY, (payload: ResumePayload) => {
  chat.resume(payload)
})

const { isLoading } = chat
const messageTree = chat.messageTree

// 全局 store 引用：错误状态兜底 / 消息计数
const sessionsStore = useSessionsStore()

// --- 计算属性 ---
  // 会话级错误条（"错误是状态、不是节点"的兜底展示）。
  //
  // 两个来源：会话节点的 `attributes.error`（服务端权威）与 send / resume 的本地乐观
  // 错误。但**有失败节点时一律不显示**——那时错误由根级 Turn 承载（⚠ + 重试），
  // 再显示一条会话级横幅就是同一条错误报两遍。
  //
  // 这个判定放在**渲染时**（从节点表算出）而不是事件到达时：事件时判定隐含
  // "错误事件到达的那一刻恰好能看到失败节点"，而两条通道的先后无法保证。
  // 从节点表派生则没有这个问题——同一份节点表必然得到同一个结论
  // （`node-state-streaming.md` §5.2「派生是纯函数」）。
  const sessionError = computed(() => {
    const err = sessionsStore.getSessionError(props.sessionId)
    if (!err) return null
    const hasFailedNode = sessionsStore
      .getSessionMessages(props.sessionId)
      // 「失败节点」的判据与后端 meta 字段的读取都收在 registry/messageTypes：
      // 本组件不解释 `status` 取值，也不碰 `meta.ephemeral` 这种后端字段名。
      .some((m) => isFailedStatus(messageStatusOf(m)) && !messageIsEphemeral(m))
    return hasFailedNode ? null : err
  })

  /** 会话级错误重试：重新发送用户最后一条消息（复用其 id 避免乐观消息重复节点）。
   *  仅用于"无 Failed Turn 节点"的兜底错误；有 Failed Turn 时错误由其节点承载、走 handleRetry。 */
  function handleSessionRetry() {
    const msgs = sessionsStore.getSessionMessages(props.sessionId)
    const lastUser = [...msgs].reverse().find((m) => messageRoleOf(m) === CHAT_ROLE_USER)
    sessionsStore.setSessionError(props.sessionId, null)
    if (!lastUser) return
    const retryMsg: ChatMessage = {
      id: lastUser.id, // 复用原 user 消息 id：重发而非新建节点
      role: CHAT_ROLE_USER,
      content: lastUser.content,
      timestamp: Date.now(),
    }
    chat.send(retryMsg)
  }

  // --- 方法 ---
onMounted(() => {
  nextTick(() => scrollToBottom())
  // 启动看门狗：检测"卡在处理中但长时间无事件"的会话（目标 2 系统保障）
  startWatchdog()

  // 懒创建闭环：新建会话流程中排队的首条消息（文本 + 可选图片附件；
  // 用户在"新建详情"输入、创建完成后经机制选中切到本会话），挂载即注入并发送。
  // 注意：图片的 thumbnailUrl 是 object URL，所有权随载荷转移，此处**不可 revoke**。
  const pending = sessionsStore.consumePendingFirstMessage(props.sessionId)
  if (pending) {
    inputText.value = pending.text
    if (pending.images?.length) attachedImages.value = pending.images
    handleSend()
  }
})

onBeforeUnmount(() => {
  stopWatchdog()
  if (scrollRaf !== null) {
    cancelAnimationFrame(scrollRaf)
    scrollRaf = null
  }
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

  // 构建消息内容
  let content: MessageContent
  if (images.length > 0) {
    const parts: ContentPart[] = []
    if (text) parts.push({ type: 'text', text })
    images.forEach(img => parts.push({
      type: 'image_url',
      image_url: { url: `data:${img.mimeType};base64,${img.base64}` }
    }))
    content = parts
  } else {
    content = text
  }

  const userMessage = {
    id: crypto.randomUUID(),
    role: 'user' as const,
    content,
    timestamp: Date.now()
  }

  // 重置输入状态
  inputText.value = ''
  attachedImages.value.forEach(img => img.thumbnailUrl && URL.revokeObjectURL(img.thumbnailUrl))
  attachedImages.value = []
  composerRef.value?.resetHeight()

  nextTick(() => scrollToBottom())

  const chatMessage: ChatMessage = {
    id: userMessage.id,
    role: userMessage.role as ChatRole,
    content: userMessage.content,
    timestamp: userMessage.timestamp
  }

  chat.send(chatMessage)
}


/**
 * 重试入口：分派规则在 `registry/messageTypes.messageRetryTargetOf`——
 * 「工具调用失败 → 单工具重试」「其余失败 → 整轮重试（叶子回溯到父 Turn）」
 * 是**消息级业务规则**，不属于面板。本面板只负责取节点 + 发恢复请求。
 */
function handleRetry(messageId: string) {
  const msg = sessionsStore.getSessionMessages(props.sessionId).find(m => m.id === messageId)
  if (!msg) return
  const target = messageRetryTargetOf(msg)
  chat.resume({ targetId: target.targetId, action: target.action })
}

/** 删除单条消息（由 store 调后端持久化）；若为 user 消息，删除后将其内容回填输入框 */
async function handleDelete(messageId: string) {
  // 删除前先取出消息：回填空需要其内容
  const msg = sessionsStore.getSessionMessages(props.sessionId).find(m => m.id === messageId)
  const isUserMsg = msg ? messageRoleOf(msg) === CHAT_ROLE_USER : false
  // 删除前先取出正文（回填空需要其内容）；取值走契约层唯一实现，不在此另写形状判定
  const userText = isUserMsg ? messageTextOf(msg?.content) : ''

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
  const text = messageTextOf(c)
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
// 每 15s 检查一次：若运行中（节点 status == working）但已超过阈值时间无事件，则把仍在
// streaming/waiting 的消息持久化为 Failed —— 直接出现在对话流中（带内联重试按钮），
// 切回会话仍能看到上次的错误，无需任何顶部 banner。
let watchdogTimer: ReturnType<typeof setInterval> | null = null
function startWatchdog() {
  stopWatchdog()
  watchdogTimer = setInterval(() => {
    const sid = props.sessionId
    if (!sid) return
    const status = sessionsStore.getSessionStatus(sid)
    if (!isWorkingStatus(status.status)) return
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

// 智能滚动 —— **只 watch 版本号**，不 deep watch 消息树。
//
// `messageTree` 是一棵带 `children` 自引用的树，deep watch 会在每个流式 token
// 上遍历整棵树（O(n²)），而且监听器本身也会被树里任何一处改动唤醒。改成
// 订阅 store 的 `transcriptVersion`（一个数字，只在 `commitMessages` 提交时 +1），
// 再用 rAF 把一帧内的多次提交合并成一次滚动——滚动是**视觉效果**，
// 一帧一次就够，不必逐 token 同步。
let scrollRaf: number | null = null
watch(
  () => sessionsStore.transcriptVersion,
  () => {
    if (scrollRaf !== null) return
    scrollRaf = requestAnimationFrame(() => {
      scrollRaf = null
      smartScroll()
    })
  }
)
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
  color: var(--text-muted);
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
  border-top: 1px solid var(--border-default);
  background: var(--surface-panel);
  flex-shrink: 0;
}

/* ═══════════════════════════════════════════════════════════
   Error banner（会话级错误条）
   ═══════════════════════════════════════════════════════════ */
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

/* 压缩提示条：与错误条同版式，但走 accent 色系——它不是错误，是"需要多等一会儿"。
   压缩窗口内没有任何消息节点可渲染，这条横幅是唯一能说明"系统在做事"的东西。 */
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
