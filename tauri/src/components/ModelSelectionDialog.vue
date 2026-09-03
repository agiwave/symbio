<template>
  <Teleport to="body">
    <Transition name="slide-up">
      <div 
        v-if="state.visible.value" 
        ref="dialogRef"
        class="ai-selection-dialog"
        :style="state.dialogStyle.value"
        :class="{ dragging: state.isDragging.value }"
      >
        <div 
          class="dialog-header" 
          @mousedown="handleDragStart"
        >
          <!-- 拖动手柄 -->
          <div class="drag-handle" title="拖动移动">
            <svg width="12" height="12" viewBox="0 0 12 12" fill="currentColor">
              <circle cx="3" cy="3" r="1.5" />
              <circle cx="9" cy="3" r="1.5" />
              <circle cx="3" cy="9" r="1.5" />
              <circle cx="9" cy="9" r="1.5" />
            </svg>
          </div>
          <span class="header-icon">✨</span>
          <span class="dialog-title">Model \u52a9\u624b</span>
          <button class="dialog-close" @click.stop="state.close">×</button>
        </div>
        
        <!-- 选中的内容提示 -->
        <div v-if="state.selectedText.value" class="selected-context">
          <div class="context-header">
            <span class="context-label">选中的内容</span>
            <span v-if="selectionInfo.filePath" class="file-path">
              📄 {{ getRelativePath(selectionInfo.filePath) }}
            </span>
            <span v-if="selectionInfo.startLine && selectionInfo.endLine" class="line-range">
              📍 行 {{ selectionInfo.startLine }}-{{ selectionInfo.endLine }}
            </span>
          </div>
          <div class="context-text">
            {{ state.selectedText.value.slice(0, 100) }}{{ state.selectedText.value.length > 100 ? '...' : '' }}
          </div>
        </div>

        <div class="dialog-body">
          <div class="messages" ref="messagesRef">
            <div v-if="chat.messageTree.value.length === 0 && initialLoadDone" class="empty-chat">
              <p>开始与 Model 对话</p>
            </div>
            
            <MessageNode
              v-for="node in chat.messageTree.value"
              :key="node.id"
              :node="node"
              @delete="handleDelete"
              @edit="handleEdit"
            />
          </div>
        </div>
        
        <div class="dialog-footer">
          <textarea
            v-model="state.input.value"
            :placeholder="state.selectedText.value ? '针对选中内容提问...' : '输入问题...'"
            @keydown.enter.exact.prevent="handleSend"
            @keydown.escape.exact="state.close"
            ref="inputRef"
            rows="1"
          ></textarea>
          <button 
            @click="handleSend" 
            :disabled="!state.input.value.trim() || chat.isLoading.value" 
            class="send-btn"
          >
            <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <line x1="22" y1="2" x2="11" y2="13"></line>
              <polygon points="22 2 15 22 11 13 2 9 22 2"></polygon>
            </svg>
          </button>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<script setup lang="ts">
import { ref, nextTick, watch, computed } from 'vue'
import type { ModelSelectionReturn } from '@/composables/useModelSelection'
import { type ChatMessage } from '@/services/model'
import { useModelContext, buildContextualMessage } from '@/composables/useModelContext'
import { useChatScroll } from '@/composables/useChatScroll'
import { useChatConnection } from '@/composables/useChatConnection'
import { useSessionsStore } from '@/stores/sessions'
import { logger } from '@/utils/logger'
import { getSessionConfig } from '@/services/config'
import { listModelProviders } from '@/services/modelProviders'
import MessageNode from './MessageNode.vue'

const props = defineProps<{
  state: ModelSelectionReturn
  // 当前文档信息（由父组件传入）
  currentFilePath?: string
  currentFileContent?: string
}>()

// --- 状态管理 ---
const agentId = ref<string>('default_assistant')
const providerId = ref<string>('')
const sessionsStore = useSessionsStore()

// TODO: 
// watch(agentId, async (newVal) => {
//   if (newVal && props.state.sessionId) {
//     try {
//       await callPlugin('session/update', {
//         session_id: props.state.sessionId,
//         metadata: { agent_id: newVal }
//       }, undefined, { session_id: props.state.sessionId })
//     } catch (e) {
//       console.error('Failed to update session agent:', e)
//     }
//   }
// })

// 使用全局 AI 上下文
const { context } = useModelContext()

// 使用共享 composables
const messagesRef = ref<HTMLElement | null>(null)
const { scrollToBottom } = useChatScroll(messagesRef)

// 使用统一的聊天连接（消息由全局 store + sessionBusWatcher 驱动）
const chat = useChatConnection({
  sessionId: props.state.sessionId
})

const inputRef = ref<HTMLTextAreaElement | null>(null)
const dialogRef = ref<HTMLElement | null>(null)

// 历史加载状态
const initialLoadDone = ref(false)

// 获取选区信息
const selectionInfo = computed(() => {
  const saved = props.state.savedSelection.value
  if (!saved) return {}
  return {
    filePath: saved.filePath,
    startLine: saved.startLine,
    endLine: saved.endLine,
    fullContent: saved.fullContent
  }
})

// 获取相对路径
function getRelativePath(fullPath: string): string {
  const parts = fullPath.split(/[\\\/]/)
  if (parts.length > 2) {
    return '.../' + parts.slice(-2).join('/')
  }
  return fullPath
}

// 加载历史消息：直接对齐全局 store（chat.messageTree 读取同一份 sessionMessages），
// 不再维护一份死状态的 props.state.messages。
async function loadHistory() {
  try {
    await sessionsStore.loadMessages(props.state.sessionId)
  } catch (err) {
    logger.error('ModelSelectionDialog', 'Failed to load history:', err)
  } finally {
    initialLoadDone.value = true
    await nextTick()
    scrollToBottom()
  }
}

// 从消息体中提取纯文本（content 可能是 string 或 { text }）
function messageText(c: unknown): string {
  if (typeof c === 'string') return c
  if (c && typeof c === 'object' && 'text' in c) return (c as { text: string }).text
  return ''
}

// 删除消息：仅 root 级节点可删（按钮可见性在 MessageNode 内控制）。
// 后端会删除目标消息及其之后所有消息；若为 user 消息，删除后把内容回填输入框。
async function handleDelete(messageId: string) {
  const msg = sessionsStore.getSessionMessages(props.state.sessionId).find(m => m.id === messageId)
  const isUserMsg = msg?.role === 'user'
  const userText = isUserMsg ? messageText(msg!.content) : ''
  try {
    await sessionsStore.deleteMessage(props.state.sessionId, messageId)
    if (isUserMsg && userText) {
      const cur = props.state.input.value ?? ''
      const sep = cur.trim().length > 0 ? '\n\n' : ''
      props.state.input.value = cur + sep + userText
    }
  } catch (e) {
    logger.error('ModelSelectionDialog', 'deleteMessage 失败', e)
  }
}

// 编辑消息：选区助手弹窗不内联编辑浮层，复用"回填输入框"语义——仅用户消息可编辑。
function handleEdit(messageId: string) {
  const msg = sessionsStore.getSessionMessages(props.state.sessionId).find(m => m.id === messageId)
  if (!msg || msg.role !== 'user') return
  const text = messageText(msg.content)
  if (!text) return
  const cur = props.state.input.value ?? ''
  const sep = cur.trim().length > 0 ? '\n\n' : ''
  props.state.input.value = cur + sep + text
}

// 拖拽处理
function handleDragStart(e: MouseEvent) {
  if ((e.target as HTMLElement).closest('.dialog-close')) return
  props.state.startDrag(e)
}

// 发送消息
async function handleSend() {
  const text = props.state.input.value.trim()
  if (!text || chat.isLoading.value) return

  const contextualContent = buildContextualMessage(text, context.value)
  
  const userMessage: ChatMessage = {
    id: crypto.randomUUID(),
    role: 'user',
    type: 'text',
    content: contextualContent,
    status: 'completed',
    timestamp: Math.floor(Date.now() / 1000)
  }

  // store 会在收到会话事件后自动 append，无需手动维护 state.messages
  props.state.input.value = ''
  
  await nextTick()
  scrollToBottom()

  // 发送请求
  chat.send(userMessage, agentId.value, providerId.value || undefined)
}

// 监听可见性
watch(() => props.state.visible.value, async (visible, wasVisible) => {
  if (visible) {
    if (!wasVisible) {
      initialLoadDone.value = false
      
      // Load agentId and providerId from session config or session metadata（来自 session/list 缓存）
      try {
        let loadedAgent = false
        let loadedProvider = false
        const md = sessionsStore.list.find(s => s.id === props.state.sessionId)?.metadata
        if (md?.agent_id) {
          agentId.value = md.agent_id
          loadedAgent = true
        }
        if (md?.provider_id) {
          providerId.value = md.provider_id
          loadedProvider = true
        }

        const config = await getSessionConfig()
        if (!loadedAgent && config.default_agent_id) {
          agentId.value = config.default_agent_id
        }
        if (!loadedProvider) {
          try {
            const cfg = await listModelProviders()
            providerId.value = cfg.default_provider_id ?? ''
          } catch (e) {
            logger.warn('ModelSelectionDialog', 'Failed to load Model providers', e)
          }
        }
      } catch (err) {
        logger.error('ModelSelectionDialog', 'Failed to load session config', err)
      }
    }
    
    nextTick(() => {
      props.state.dialogRef.value = dialogRef.value
      inputRef.value?.focus()
    })
    
    await loadHistory()
  }
})

// 监听选区变化
watch(() => props.state.selectedText.value, () => {
  if (props.state.visible.value) {
    nextTick(() => {
      inputRef.value?.focus()
    })
  }
})
</script>

<style scoped>
.ai-selection-dialog {
  position: fixed;
  width: 22.5rem;
  max-height: 60vh;
  background: var(--surface-overlay);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-2);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  z-index: var(--z-dialog);
  user-select: none;
}

.ai-selection-dialog.dragging {
  cursor: grabbing;
}

.dialog-header {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-3);
  border-bottom: 1px solid var(--border-subtle);
  cursor: grab;
}

.drag-handle {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 1.5rem;
  height: 1.5rem;
  color: var(--text-muted);
  cursor: grab;
}

.header-icon {
  font-size: var(--font-size-base);
}

.dialog-title {
  font-weight: var(--font-weight-semibold);
  font-size: var(--font-size-sm);
  flex: 1;
  color: var(--text-primary);
}

.dialog-close {
  width: 1.375rem;
  height: 1.375rem;
  border: none;
  background: transparent;
  cursor: pointer;
  font-size: var(--font-size-md);
  color: var(--text-muted);
  border-radius: var(--radius-sm);
}
.dialog-close:hover { color: var(--text-primary); }

.selected-context {
  padding: var(--space-2) var(--space-3);
  background: var(--accent-subtle-bg);
}

.context-header {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  margin-bottom: var(--space-1);
}

.context-label {
  font-size: var(--font-size-xs);
  color: var(--text-muted);
  text-transform: uppercase;
}

.context-text {
  font-size: var(--font-size-xs);
  color: var(--text-secondary);
  padding: var(--space-2) var(--space-3);
  background: var(--surface-card);
  border-radius: var(--radius-md);
}

.dialog-body {
  flex: 1;
  min-height: 0;
  overflow: hidden;
  display: flex;
  flex-direction: column;
}

.messages {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: var(--space-2);
}

.dialog-footer {
  display: flex;
  gap: var(--space-2);
  padding: var(--space-2) var(--space-3);
  border-top: 1px solid var(--border-subtle);
}

.dialog-footer textarea {
  flex: 1;
  border: 1px solid var(--border-default);
  border-radius: var(--radius-lg);
  padding: var(--space-2) var(--space-3);
  font-size: var(--font-size-sm);
  resize: none;
  outline: none;
  color: var(--text-primary);
  background: var(--surface-overlay);
  font-family: inherit;
}
.dialog-footer textarea:focus { border-color: var(--accent); }

.send-btn {
  width: 2.25rem;
  height: 2.25rem;
  background: var(--accent);
  border: none;
  border-radius: var(--radius-lg);
  color: var(--text-on-accent);
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.typing-indicator {
  display: inline-flex;
  gap: var(--space-1);
}

.typing-indicator span {
  width: 0.25rem;
  height: 0.25rem;
  background: var(--text-muted);
  border-radius: var(--radius-full);
}

.empty-chat {
  text-align: center;
  color: var(--text-muted);
  padding: var(--space-5);
}

.slide-up-enter-active, .slide-up-leave-active {
  transition: all 0.2s ease;
}
.slide-up-enter-from, .slide-up-leave-to {
  opacity: 0;
  transform: translateY(0.625rem);
}
</style>
