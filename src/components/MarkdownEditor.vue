<template>
  <div class="notion-editor" ref="containerRef">
    <!-- 编辑器容器 -->
    <div ref="editorRef" class="editor-root"></div>
    
    <!-- AI 对话框 -->
    <Teleport to="body">
      <Transition name="dialog">
        <div v-if="showAIDialog" class="ai-dialog-overlay" @click.self="closeAIDialog">
          <div class="ai-dialog">
            <div class="ai-dialog-header">
              <span class="ai-header-icon">✨</span>
              <span class="ai-dialog-title">AI 助手</span>
              <button class="ai-dialog-close" @click="closeAIDialog">×</button>
            </div>
            <div class="ai-dialog-body">
              <div class="ai-messages" ref="messagesRef">
                <div v-for="(msg, idx) in aiMessages" :key="idx" :class="['ai-msg', msg.role]">
                  <div class="ai-msg-content" v-html="renderMarkdown(msg.content)"></div>
                </div>
                <div v-if="aiLoading" class="ai-msg assistant loading">
                  <div class="ai-msg-content">
                    <span class="typing-dots">...</span>
                  </div>
                </div>
              </div>
            </div>
            <div class="ai-dialog-footer">
              <textarea
                v-model="aiInput"
                placeholder="输入问题... (Enter 发送)"
                @keydown.enter.exact.prevent="sendAIMessage"
                @keydown.escape.exact="closeAIDialog"
                ref="aiInputRef"
                rows="1"
              ></textarea>
              <button @click="sendAIMessage" :disabled="!aiInput.trim() || aiLoading" class="ai-send-btn">
                <svg width="18" height="18" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <line x1="22" y1="2" x2="11" y2="13"></line>
                  <polygon points="22 2 15 22 11 13 2 9 22 2"></polygon>
                </svg>
              </button>
            </div>
          </div>
        </div>
      </Transition>
    </Teleport>
    
    <!-- 快捷键提示 -->
    <Transition name="fade">
      <div v-if="!showAIDialog" class="shortcut-hint">
        <kbd>/</kbd> 命令菜单 · <kbd>Ctrl</kbd><kbd>K</kbd> AI 助手
      </div>
    </Transition>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, onUnmounted, nextTick, shallowRef, watch } from 'vue'
import { Crepe, CrepeFeature } from '@milkdown/crepe'
import '@milkdown/crepe/theme/common/style.css'
import '@milkdown/crepe/theme/frame.css'
import { callPlugin } from '@/services/plugin'
import { marked } from 'marked'

const props = defineProps<{
  modelValue: string
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string]
}>()

// DOM refs
const containerRef = ref<HTMLElement | null>(null)
const editorRef = ref<HTMLElement | null>(null)
const messagesRef = ref<HTMLElement | null>(null)
const aiInputRef = ref<HTMLTextAreaElement | null>(null)

// Editor instance
const crepe = shallowRef<Crepe | null>(null)

// AI dialog
const showAIDialog = ref(false)
const aiInput = ref('')
const aiMessages = ref<{ role: 'user' | 'assistant'; content: string }[]>([])
const aiLoading = ref(false)

// Initialize editor
async function initEditor() {
  if (!editorRef.value) return
  
  const defaultContent = props.modelValue || `# 开始创作

欢迎使用编辑器。直接输入内容，或使用 Markdown 语法。

- **粗体** 和 *斜体*
- \`行内代码\` 和代码块
- [链接](https://example.com)
- 列表和引用

按 **/** 打开命令菜单，**Ctrl+K** 呼出 AI 助手。
`

  crepe.value = new Crepe({
    root: editorRef.value,
    defaultValue: defaultContent,
    features: {
      [CrepeFeature.BlockEdit]: true,      // Slash 命令 + Block Handle
      [CrepeFeature.Toolbar]: true,         // 选中文本工具栏
      [CrepeFeature.LinkTooltip]: true,     // 链接悬浮编辑
      [CrepeFeature.ImageBlock]: true,      // 图片块支持
      [CrepeFeature.CodeMirror]: true,      // 代码块增强
      [CrepeFeature.Placeholder]: true,     // 占位符
      [CrepeFeature.Cursor]: true,          // 光标样式
      [CrepeFeature.ListItem]: true,        // 列表项图标
      [CrepeFeature.Table]: true,           // 表格支持
    },
    featureConfigs: {
      [CrepeFeature.Placeholder]: {
        text: '输入 / 打开命令菜单...',
        mode: 'block',
      },
      [CrepeFeature.Toolbar]: {
        boldIcon: '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M15.6 10.79c.97-.67 1.65-1.77 1.65-2.79 0-2.26-1.75-4-4-4H7v14h7.04c2.09 0 3.71-1.7 3.71-3.79 0-1.52-.86-2.82-2.15-3.42zM10 6.5h3c.83 0 1.5.67 1.5 1.5s-.67 1.5-1.5 1.5h-3v-3zm3.5 9H10v-3h3.5c.83 0 1.5.67 1.5 1.5s-.67 1.5-1.5 1.5z"/></svg>',
        italicIcon: '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M10 4v3h2.21l-3.42 8H6v3h8v-3h-2.21l3.42-8H18V4z"/></svg>',
        strikethroughIcon: '<svg viewBox="0 0 24 24" fill="currentColor"><path d="M10 19h4v-3h-4v3zM5 4v3h5v3h4V7h5V4H5zM3 14h18v-2H3v2z"/></svg>',
        codeIcon: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="16 18 22 12 16 6"/><polyline points="8 6 2 12 8 18"/></svg>',
        linkIcon: '<svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/></svg>',
      },
    },
  })
  
  // Listen for content changes
  crepe.value.on((listener) => {
    listener.markdownUpdated((_, markdown) => {
      emit('update:modelValue', markdown)
    })
  })
  
  await crepe.value.create()
}

// Update content when modelValue changes externally
watch(() => props.modelValue, (newValue) => {
  if (crepe.value && newValue !== undefined) {
    // Only update if significantly different to avoid cursor jump
    const currentMarkdown = crepe.value.getMarkdown()
    if (currentMarkdown !== newValue) {
      crepe.value.setMarkdown(newValue)
    }
  }
})

// AI Dialog
function openAIDialog() {
  showAIDialog.value = true
  nextTick(() => aiInputRef.value?.focus())
}

function closeAIDialog() {
  showAIDialog.value = false
}

async function sendAIMessage() {
  if (!aiInput.value.trim() || aiLoading.value) return
  
  const userMessage = aiInput.value.trim()
  aiMessages.value.push({ role: 'user', content: userMessage })
  aiInput.value = ''
  aiLoading.value = true
  
  try {
    const response = await callPlugin<{ content: string }>('/agent/chat', {
      action: 'send',
      messages: aiMessages.value.map(m => ({ role: m.role, content: m.content }))
    })
    aiMessages.value.push({ role: 'assistant', content: response.content || '抱歉，无法处理请求。' })
  } catch (error) {
    aiMessages.value.push({ role: 'assistant', content: `错误: ${error}` })
  } finally {
    aiLoading.value = false
    nextTick(() => {
      messagesRef.value?.scrollTo({ top: messagesRef.value.scrollHeight, behavior: 'smooth' })
    })
  }
}

function renderMarkdown(content: string): string {
  return marked(content) as string
}

// Keyboard shortcuts
function handleKeydown(e: KeyboardEvent) {
  if ((e.ctrlKey || e.metaKey) && e.key === 'k') {
    e.preventDefault()
    openAIDialog()
  }
}

// Destroy editor
async function destroyEditor() {
  if (crepe.value) {
    try {
      crepe.value.destroy()
    } catch (e) {
      console.error('Destroy error:', e)
    }
    crepe.value = null
  }
}

// Lifecycle
onMounted(() => {
  initEditor()
  document.addEventListener('keydown', handleKeydown)
})

onUnmounted(() => {
  destroyEditor()
  document.removeEventListener('keydown', handleKeydown)
})

defineExpose({ openAIDialog })
</script>

<style scoped>
.notion-editor {
  position: relative;
  height: 100%;
  width: 100%;
  background: #fff;
  display: flex;
  flex-direction: column;
}

.editor-root {
  flex: 1;
  overflow-y: auto;
  padding: 32px 48px;
  min-height: 0;
}

/* Crepe Editor Overrides - Notion-like */
.editor-root :deep(.crepe) {
  font-family: -apple-system, BlinkMacMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif;
  font-size: 16px;
  line-height: 1.6;
  color: #37352f;
  outline: none;
  min-height: 100%;
}

.editor-root :deep(.crepe .ProseMirror) {
  outline: none;
  min-height: 100%;
}

/* Headings */
.editor-root :deep(.crepe h1) {
  font-size: 2.25rem;
  font-weight: 700;
  margin: 0 0 0.5rem;
  line-height: 1.2;
  letter-spacing: -0.03em;
  color: #37352f;
}

.editor-root :deep(.crepe h2) {
  font-size: 1.5rem;
  font-weight: 600;
  margin: 1rem 0 0.375rem;
  line-height: 1.3;
}

.editor-root :deep(.crepe h3) {
  font-size: 1.25rem;
  font-weight: 600;
  margin: 0.75rem 0 0.25rem;
}

/* Paragraph */
.editor-root :deep(.crepe p) {
  margin: 0.25rem 0;
}

/* Code */
.editor-root :deep(.crepe code) {
  background: rgba(135, 131, 120, 0.15);
  color: #eb5757;
  padding: 0.2em 0.4em;
  border-radius: 3px;
  font-family: 'SF Mono', 'Fira Code', Consolas, monospace;
  font-size: 85%;
}

.editor-root :deep(.crepe pre) {
  background: #f7f6f3;
  border-radius: 4px;
  padding: 16px;
  margin: 8px 0;
  overflow-x: auto;
}

.editor-root :deep(.crepe pre code) {
  background: transparent;
  color: inherit;
  padding: 0;
  font-size: 14px;
}

/* Blockquote */
.editor-root :deep(.crepe blockquote) {
  border-left: 3px solid #37352f;
  padding-left: 16px;
  margin: 8px 0;
  color: #37352f;
}

/* Lists */
.editor-root :deep(.crepe ul),
.editor-root :deep(.crepe ol) {
  margin: 4px 0;
  padding-left: 24px;
}

.editor-root :deep(.crepe li) {
  margin: 2px 0;
}

.editor-root :deep(.crepe li p) {
  margin: 0;
}

/* Tables */
.editor-root :deep(.crepe table) {
  border-collapse: collapse;
  width: 100%;
  margin: 8px 0;
}

.editor-root :deep(.crepe th),
.editor-root :deep(.crepe td) {
  border: 1px solid #e0e0e0;
  padding: 8px 12px;
  text-align: left;
}

.editor-root :deep(.crepe th) {
  background: #f7f6f3;
  font-weight: 600;
}

/* Links */
.editor-root :deep(.crepe a) {
  color: #2383e2;
  text-decoration: underline;
  text-underline-offset: 2px;
}

.editor-root :deep(.crepe a:hover) {
  color: #0077d4;
}

/* HR */
.editor-root :deep(.crepe hr) {
  border: none;
  border-top: 1px solid #e0e0e0;
  margin: 16px 0;
}

/* Images */
.editor-root :deep(.crepe img) {
  max-width: 100%;
  border-radius: 4px;
  margin: 8px 0;
}

/* Selection highlight */
.editor-root :deep(.crepe ::selection) {
  background: rgba(35, 131, 226, 0.28);
}

/* Block Handle - 拖拽手柄样式优化 */
.editor-root :deep(.crepe-block-handle) {
  opacity: 0;
  transition: opacity 0.15s ease;
}

.editor-root :deep(.crepe-block-handle:hover),
.editor-root :deep(.ProseMirror-selectednode + .crepe-block-handle) {
  opacity: 1;
}

/* Slash Menu - 命令菜单样式 */
.editor-root :deep(.crepe-slash-menu) {
  background: #fff;
  border-radius: 8px;
  box-shadow: 0 4px 24px rgba(0, 0, 0, 0.15);
  border: 1px solid #e5e5e5;
  padding: 8px 0;
  min-width: 280px;
  max-height: 400px;
  overflow-y: auto;
}

.editor-root :deep(.crepe-slash-menu-item) {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 8px 16px;
  cursor: pointer;
  transition: background 0.1s;
}

.editor-root :deep(.crepe-slash-menu-item:hover),
.editor-root :deep(.crepe-slash-menu-item.selected) {
  background: #f5f5f5;
}

.editor-root :deep(.crepe-slash-menu-icon) {
  width: 40px;
  height: 40px;
  border-radius: 4px;
  background: #f7f6f3;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.editor-root :deep(.crepe-slash-menu-label) {
  font-weight: 500;
  font-size: 14px;
  color: #37352f;
}

.editor-root :deep(.crepe-slash-menu-desc) {
  font-size: 12px;
  color: #787774;
}

/* Toolbar - 选中文本工具栏 */
.editor-root :deep(.crepe-toolbar) {
  background: #1f1f1f;
  border-radius: 6px;
  padding: 4px 6px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.25);
}

.editor-root :deep(.crepe-toolbar-item) {
  background: transparent;
  border: none;
  color: #fff;
  padding: 6px;
  border-radius: 4px;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: background 0.1s;
}

.editor-root :deep(.crepe-toolbar-item:hover) {
  background: rgba(255, 255, 255, 0.12);
}

.editor-root :deep(.crepe-toolbar-item.active) {
  background: rgba(255, 255, 255, 0.2);
}

/* Link Tooltip */
.editor-root :deep(.crepe-link-tooltip) {
  background: #fff;
  border-radius: 8px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.15);
  border: 1px solid #e5e5e5;
  padding: 8px 12px;
}

.editor-root :deep(.crepe-link-tooltip input) {
  border: 1px solid #e5e5e5;
  border-radius: 4px;
  padding: 6px 10px;
  font-size: 14px;
  outline: none;
  min-width: 200px;
}

.editor-root :deep(.crepe-link-tooltip input:focus) {
  border-color: #2383e2;
}

/* Placeholder */
.editor-root :deep(.crepe .crepe-placeholder) {
  color: #9b9a97;
  pointer-events: none;
  position: absolute;
  top: 0;
  left: 0;
}

/* Shortcut Hint */
.shortcut-hint {
  position: fixed;
  bottom: 16px;
  right: 16px;
  background: #1f1f1f;
  color: #fff;
  padding: 6px 12px;
  border-radius: 6px;
  font-size: 12px;
  pointer-events: none;
  z-index: 100;
}

.shortcut-hint kbd {
  background: rgba(255, 255, 255, 0.15);
  padding: 2px 6px;
  border-radius: 4px;
  margin: 0 2px;
  font-family: inherit;
}

/* AI Dialog */
.ai-dialog-overlay {
  position: fixed;
  inset: 0;
  background: rgba(0, 0, 0, 0.5);
  backdrop-filter: blur(4px);
  z-index: 2000;
  display: flex;
  align-items: center;
  justify-content: center;
}

.ai-dialog {
  width: 480px;
  max-width: 90vw;
  max-height: 75vh;
  background: #fff;
  border-radius: 12px;
  box-shadow: 0 20px 50px rgba(0, 0, 0, 0.2);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.ai-dialog-header {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 14px 16px;
  border-bottom: 1px solid #e5e5e5;
}

.ai-header-icon {
  font-size: 18px;
}

.ai-dialog-title {
  font-weight: 600;
  font-size: 15px;
  flex: 1;
}

.ai-dialog-close {
  width: 28px;
  height: 28px;
  border: none;
  background: transparent;
  border-radius: 6px;
  cursor: pointer;
  font-size: 20px;
  color: #666;
  display: flex;
  align-items: center;
  justify-content: center;
}

.ai-dialog-close:hover {
  background: #f0f0f0;
}

.ai-dialog-body {
  flex: 1;
  overflow: hidden;
  min-height: 200px;
}

.ai-messages {
  height: 100%;
  overflow-y: auto;
  padding: 16px;
  display: flex;
  flex-direction: column;
  gap: 12px;
}

.ai-msg {
  max-width: 88%;
  padding: 10px 14px;
  border-radius: 12px;
  font-size: 14px;
  line-height: 1.5;
}

.ai-msg.user {
  align-self: flex-end;
  background: linear-gradient(135deg, #7c3aed, #2563eb);
  color: #fff;
  border-bottom-right-radius: 4px;
}

.ai-msg.assistant {
  align-self: flex-start;
  background: #f4f4f5;
  color: #18181b;
  border-bottom-left-radius: 4px;
}

.ai-msg.assistant.loading .ai-msg-content {
  opacity: 0.6;
}

.ai-msg-content :deep(p) { margin: 0; }
.ai-msg-content :deep(p+p) { margin-top: 8px; }
.ai-msg-content :deep(code) {
  background: rgba(0,0,0,0.1);
  padding: 2px 5px;
  border-radius: 3px;
  font-size: 13px;
}
.ai-msg-content :deep(pre) {
  background: #1e1e1e;
  color: #d4d4d4;
  padding: 10px 12px;
  border-radius: 6px;
  margin: 8px 0;
  overflow-x: auto;
}
.ai-msg-content :deep(pre code) {
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

.ai-dialog-footer {
  display: flex;
  gap: 8px;
  padding: 12px 16px;
  border-top: 1px solid #e5e5e5;
  background: #fafafa;
}

.ai-dialog-footer textarea {
  flex: 1;
  border: 1px solid #e0e0e0;
  border-radius: 8px;
  padding: 10px 14px;
  font-size: 14px;
  resize: none;
  outline: none;
  font-family: inherit;
  line-height: 1.4;
  max-height: 120px;
}

.ai-dialog-footer textarea:focus {
  border-color: #7c3aed;
}

.ai-send-btn {
  width: 40px;
  height: 40px;
  background: linear-gradient(135deg, #7c3aed, #2563eb);
  border: none;
  border-radius: 8px;
  color: #fff;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  transition: all 0.15s;
}

.ai-send-btn:hover:not(:disabled) {
  transform: scale(1.02);
}

.ai-send-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

/* Transitions */
.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.15s ease;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}

.dialog-enter-active,
.dialog-leave-active {
  transition: all 0.2s ease;
}

.dialog-enter-from,
.dialog-leave-to {
  opacity: 0;
}

.dialog-enter-from .ai-dialog,
.dialog-leave-to .ai-dialog {
  transform: translateY(16px) scale(0.98);
}

/* Responsive */
@media (max-width: 768px) {
  .editor-root {
    padding: 16px;
  }
}
</style>
