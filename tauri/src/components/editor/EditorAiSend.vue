<!--
  EditorAiSend — 编辑器选区 → AI 输入 的私有桥接协议（独立于三栏实体机制）

  职责：编辑器宿主把选区状态（CodeEditor 的 selection-change）交给本组件，
  有选区时浮出「发送到 AI」按钮；点击后：
    1) setModelContext —— 写入全局 AI 上下文（文件路径 / 完整内容 / 选区 /
       行号 / 所属会话 id），ChatContextBar 渲染为上下文卡片，
       AI 收到消息时 buildContextualMessage 自动拼装 file/选区；
    2) 路由回到会话页（AI 输入所在处），输入框保持空白由用户提问；
    3) requestFocusInput —— 聚焦活跃会话的输入框。

  该协议只依赖 useModelContext（AI 输入通道）与 sessions store（活跃会话），
  与任何页面机制解耦：任何编辑器宿主（CodeEditor / 未来编辑器）传入
  selection 即可复用。
-->
<template>
  <Transition name="fade-pop">
    <button
      v-if="selection && selection.text"
      type="button"
      class="editor-ai-send"
      title="将选中文本作为上下文发送给 AI"
      @click="onSend"
    >
      <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2">
        <path d="M21 11.5a8.38 8.38 0 0 1-.9 3.8 8.5 8.5 0 0 1-7.6 4.7 8.38 8.38 0 0 1-3.8-.9L3 21l1.9-5.7a8.38 8.38 0 0 1-.9-3.8 8.5 8.5 0 0 1 4.7-7.6 8.38 8.38 0 0 1 3.8-.9h.5a8.48 8.48 0 0 1 8 8v.5z" />
      </svg>
      <span>发送到 AI</span>
      <span v-if="selection.endLine > selection.startLine" class="line-badge">
        L{{ selection.startLine }}-{{ selection.endLine }}
      </span>
      <span v-else class="line-badge">L{{ selection.startLine }}</span>
    </button>
  </Transition>
</template>

<script setup lang="ts">
import { useRouter } from 'vue-router'
import { setModelContext, requestFocusInput } from '@/composables/useModelContext'
import { useSessionsStore } from '@/stores/sessions'
import { useToast } from '@/composables/useToast'

export interface EditorSelection {
  text: string
  startLine: number
  endLine: number
}

const props = defineProps<{
  /** 当前编辑文件的路径标识（相对路径或绝对路径，随宿主场景） */
  filePath: string
  /** 文件完整内容（作为 AI 上下文的 fileContent） */
  content: string
  /** 编辑器当前选区（null = 无选区，按钮隐藏） */
  selection: EditorSelection | null
  /** 发送成功后返回的路由（AI 输入所在页面，缺省会话页） */
  backRoute?: string
}>()

const emit = defineEmits<{ (e: 'sent'): void }>()

const router = useRouter()
const sessionsStore = useSessionsStore()
const toast = useToast()

function onSend() {
  if (!props.selection || !props.filePath) return

  const sessionId = sessionsStore.activeId
  if (!sessionId) {
    toast.showToast('error', '当前没有活跃的会话，无法发送')
    return
  }

  // 1) 写入全局 AI 上下文（输入框保持空白，由用户直接提问）
  setModelContext({
    filePath: props.filePath,
    fileContent: props.content,
    selectedText: props.selection.text,
    startLine: props.selection.startLine,
    endLine: props.selection.endLine,
    sessionId,
  })

  emit('sent')

  // 2) 回到会话页（AI 输入所在处）
  router.push(props.backRoute ?? '/')

  // 3) 聚焦该会话的输入框
  requestFocusInput(sessionId)
}
</script>

<style scoped>
.editor-ai-send {
  position: absolute;
  right: 1rem;
  bottom: 1rem;
  z-index: 5;
  display: inline-flex;
  align-items: center;
  gap: 0.4rem;
  padding: 0.35rem 0.7rem;
  border: 1px solid var(--accent);
  border-radius: var(--radius-full);
  background: var(--surface-panel);
  color: var(--accent);
  cursor: pointer;
  font-size: 0.78rem;
  box-shadow: 0 2px 12px rgb(0 0 0 / 25%);
  transition: background var(--motion-fast) var(--motion-ease), color var(--motion-fast) var(--motion-ease);
}
.editor-ai-send:hover {
  background: var(--accent);
  color: var(--text-on-accent);
}
.line-badge {
  font-size: 0.68rem;
  padding: 0 0.35rem;
  border-radius: var(--radius-full);
  background: var(--accent-subtle-bg);
}

.fade-pop-enter-active,
.fade-pop-leave-active {
  transition: opacity var(--motion-fast) var(--motion-ease), transform var(--motion-fast) var(--motion-ease);
}
.fade-pop-enter-from,
.fade-pop-leave-to {
  opacity: 0;
  transform: translateY(0.35rem);
}
</style>
