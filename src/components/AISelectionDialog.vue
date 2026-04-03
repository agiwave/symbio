<template>
  <Teleport to="body">
    <Transition name="slide-up">
      <div 
        v-if="state.visible.value" 
        ref="dialogRef"
        class="ai-selection-dialog"
        :style="state.dialogStyle.value"
      >
        <div class="dialog-header">
          <span class="header-icon">✨</span>
          <span class="dialog-title">AI 助手</span>
          <button class="dialog-close" @click="state.close">×</button>
        </div>
        
        <!-- 选中的文字提示 -->
        <div v-if="state.selectedText.value" class="selected-context">
          <span class="context-label">选中的内容：</span>
          <div class="context-text">
            {{ state.selectedText.value.slice(0, 100) }}{{ state.selectedText.value.length > 100 ? '...' : '' }}
          </div>
        </div>
        
        <div class="dialog-body">
          <div class="messages" ref="messagesRef">
            <div 
              v-for="(msg, idx) in state.messages.value" 
              :key="idx" 
              :class="['msg', msg.role]"
            >
              <div class="msg-content" v-html="renderMarkdown(msg.content)"></div>
            </div>
            <div v-if="state.loading.value" class="msg assistant loading">
              <div class="msg-content">
                <span class="typing-dots">...</span>
              </div>
            </div>
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
            :disabled="!state.input.value.trim() || state.loading.value" 
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
import { ref, nextTick, watch } from 'vue'
import { marked } from 'marked'
import type { AISelectionReturn } from '@/composables/useAISelection'
import { callPlugin } from '@/services/plugin'

const props = defineProps<{
  state: AISelectionReturn
}>()

const messagesRef = ref<HTMLElement | null>(null)
const inputRef = ref<HTMLTextAreaElement | null>(null)
const dialogRef = ref<HTMLElement | null>(null)

// 渲染 Markdown
function renderMarkdown(content: string): string {
  try {
    return marked.parse(content) as string
  } catch {
    return content
  }
}

// 发送消息
async function handleSend() {
  const text = props.state.input.value.trim()
  if (!text || props.state.loading.value) return

  // 添加用户消息
  props.state.messages.value.push({ role: 'user', content: text })
  props.state.input.value = ''
  props.state.loading.value = true

  await nextTick()
  scrollToBottom()

  try {
    const response = await callPlugin<{ content: string }>('/agent/chat', {
      action: 'send',
      session_id: props.state.sessionId,
      messages: props.state.messages.value.map(m => ({ role: m.role, content: m.content })),
      selected_text: props.state.selectedText.value || undefined,
    })
    
    props.state.messages.value.push({ 
      role: 'assistant', 
      content: response.content || '抱歉，无法处理请求。' 
    })
  } catch (error) {
    props.state.messages.value.push({ 
      role: 'assistant', 
      content: `错误: ${error}` 
    })
  } finally {
    props.state.loading.value = false
    nextTick(() => scrollToBottom())
  }
}

// 滚动到底部
function scrollToBottom() {
  if (messagesRef.value) {
    messagesRef.value.scrollTop = messagesRef.value.scrollHeight
  }
}

// 监听可见性，更新 dialogRef
watch(() => props.state.visible.value, (visible) => {
  if (visible) {
    nextTick(() => {
      props.state.dialogRef.value = dialogRef.value
    })
  }
})
</script>

<style scoped>
.ai-selection-dialog {
  position: fixed;
  width: 360px;
  max-height: 60vh;
  background: rgba(255, 255, 255, 0.98);
  border-radius: 16px;
  box-shadow: 0 4px 24px rgba(0, 0, 0, 0.12), 0 0 0 1px rgba(0, 0, 0, 0.05);
  display: flex;
  flex-direction: column;
  overflow: hidden;
  z-index: 2000;
  backdrop-filter: blur(12px);
}

.dialog-header {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 14px;
  border-bottom: 1px solid rgba(0, 0, 0, 0.06);
}

.header-icon {
  font-size: 14px;
}

.dialog-title {
  font-weight: 600;
  font-size: 13px;
  flex: 1;
  color: #1a1a1a;
}

.dialog-close {
  width: 22px;
  height: 22px;
  border: none;
  background: transparent;
  border-radius: 6px;
  cursor: pointer;
  font-size: 16px;
  color: #999;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all 0.15s;
}

.dialog-close:hover {
  background: rgba(0, 0, 0, 0.08);
  color: #333;
}

/* Selected context */
.selected-context {
  padding: 8px 14px;
  background: linear-gradient(135deg, rgba(124, 58, 237, 0.06), rgba(37, 99, 235, 0.06));
}

.context-label {
  font-size: 10px;
  color: #888;
  text-transform: uppercase;
  letter-spacing: 0.5px;
}

.context-text {
  font-size: 12px;
  color: #444;
  margin-top: 4px;
  padding: 6px 10px;
  background: rgba(255, 255, 255, 0.8);
  border-radius: 6px;
  border-left: 2px solid #7c3aed;
}

.dialog-body {
  flex: 1;
  overflow: hidden;
  min-height: 100px;
  max-height: 280px;
}

.messages {
  height: 100%;
  overflow-y: auto;
  padding: 10px;
  display: flex;
  flex-direction: column;
  gap: 8px;
}

.msg {
  max-width: 88%;
  padding: 8px 12px;
  border-radius: 12px;
  font-size: 13px;
  line-height: 1.5;
}

.msg.user {
  align-self: flex-end;
  background: linear-gradient(135deg, #7c3aed, #2563eb);
  color: #fff;
  border-bottom-right-radius: 4px;
}

.msg.assistant {
  align-self: flex-start;
  background: #f4f4f5;
  color: #18181b;
  border-bottom-left-radius: 4px;
}

.msg.assistant.loading .msg-content {
  opacity: 0.6;
}

.msg-content :deep(p) { margin: 0; }
.msg-content :deep(p+p) { margin-top: 8px; }
.msg-content :deep(code) {
  background: rgba(0, 0, 0, 0.1);
  padding: 2px 5px;
  border-radius: 3px;
  font-size: 13px;
}
.msg-content :deep(pre) {
  background: #1e1e1e;
  color: #d4d4d4;
  padding: 10px 12px;
  border-radius: 6px;
  margin: 8px 0;
  overflow-x: auto;
}
.msg-content :deep(pre code) {
  background: transparent;
  padding: 0;
}

.typing-dots {
  animation: dotPulse 1s infinite;
}

@keyframes dotPulse {
  0%, 100% { opacity: 0.3; }
  50% { opacity: 1; }
}

.dialog-footer {
  display: flex;
  gap: 8px;
  padding: 10px 14px;
  border-top: 1px solid rgba(0, 0, 0, 0.06);
}

.dialog-footer textarea {
  flex: 1;
  border: 1px solid rgba(0, 0, 0, 0.1);
  border-radius: 10px;
  padding: 8px 12px;
  font-size: 13px;
  resize: none;
  outline: none;
  font-family: inherit;
  line-height: 1.4;
  max-height: 100px;
  background: rgba(0, 0, 0, 0.02);
  transition: all 0.15s;
}

.dialog-footer textarea:focus {
  border-color: #7c3aed;
  background: #fff;
}

.send-btn {
  width: 36px;
  height: 36px;
  background: linear-gradient(135deg, #7c3aed, #2563eb);
  border: none;
  border-radius: 10px;
  color: #fff;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all 0.15s;
}

.send-btn:hover:not(:disabled) {
  transform: scale(1.05);
  box-shadow: 0 2px 8px rgba(124, 58, 237, 0.3);
}

.send-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

/* Transition */
.slide-up-enter-active {
  transition: all 0.2s cubic-bezier(0.16, 1, 0.3, 1);
}

.slide-up-leave-active {
  transition: all 0.15s cubic-bezier(0.4, 0, 1, 1);
}

.slide-up-enter-from {
  opacity: 0;
  transform: translateY(12px) scale(0.96);
}

.slide-up-leave-to {
  opacity: 0;
  transform: translateY(8px) scale(0.98);
}
</style>
