<template>
  <div class="notion-editor" ref="containerRef">
    <!-- 编辑器容器 -->
    <div ref="editorRef" class="editor-root"></div>
    
    <!-- 自定义 Block Handle 容器 -->
    <Teleport to="body">
      <Transition name="fade">
        <div 
          v-if="blockHandle.visible" 
          class="custom-block-handle"
          :style="blockHandleStyle"
          @mouseenter="handleMouseEnter"
          @mouseleave="handleMouseLeave"
        >
          <!-- 拖拽触发区域 -->
          <div 
            class="handle-trigger"
            :class="{ dragging: isDragging }"
            draggable="true"
            @dragstart="handleDragStart"
            @dragend="handleDragEnd"
          >
            <svg viewBox="0 0 24 24" fill="currentColor" width="18" height="18">
              <circle cx="9" cy="6" r="1.5"/>
              <circle cx="15" cy="6" r="1.5"/>
              <circle cx="9" cy="12" r="1.5"/>
              <circle cx="15" cy="12" r="1.5"/>
              <circle cx="9" cy="18" r="1.5"/>
              <circle cx="15" cy="18" r="1.5"/>
            </svg>
          </div>
          
          <!-- 展开的操作栏 - 绝对定位 -->
          <Transition name="expand">
            <div v-if="showToolbar" class="handle-toolbar">
              <button class="toolbar-btn" @click.stop="addBlockBelow" title="在下方添加块">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <line x1="12" y1="5" x2="12" y2="19"/>
                  <line x1="5" y1="12" x2="19" y2="12"/>
                </svg>
              </button>
              <button class="toolbar-btn" @click.stop="deleteBlock" title="删除块">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <polyline points="3 6 5 6 21 6"/>
                  <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/>
                </svg>
              </button>
              <div class="toolbar-divider"></div>
              <button class="toolbar-btn" @click.stop="turnInto('heading', 1)" title="标题 1">
                <span class="btn-text">H1</span>
              </button>
              <button class="toolbar-btn" @click.stop="turnInto('heading', 2)" title="标题 2">
                <span class="btn-text">H2</span>
              </button>
              <button class="toolbar-btn" @click.stop="turnInto('heading', 3)" title="标题 3">
                <span class="btn-text">H3</span>
              </button>
              <div class="toolbar-divider"></div>
              <button class="toolbar-btn" @click.stop="turnInto('paragraph')" title="正文">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <line x1="3" y1="6" x2="21" y2="6"/>
                  <line x1="3" y1="12" x2="15" y2="12"/>
                  <line x1="3" y1="18" x2="18" y2="18"/>
                </svg>
              </button>
              <button class="toolbar-btn" @click.stop="turnInto('bullet_list')" title="无序列表">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <line x1="9" y1="6" x2="20" y2="6"/>
                  <line x1="9" y1="12" x2="20" y2="12"/>
                  <line x1="9" y1="18" x2="20" y2="18"/>
                  <circle cx="4" cy="6" r="1.5" fill="currentColor" stroke="none"/>
                  <circle cx="4" cy="12" r="1.5" fill="currentColor" stroke="none"/>
                  <circle cx="4" cy="18" r="1.5" fill="currentColor" stroke="none"/>
                </svg>
              </button>
              <button class="toolbar-btn" @click.stop="turnInto('ordered_list')" title="有序列表">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <line x1="10" y1="6" x2="21" y2="6"/>
                  <line x1="10" y1="12" x2="21" y2="12"/>
                  <line x1="10" y1="18" x2="21" y2="18"/>
                  <text x="3" y="8" font-size="8" fill="currentColor" stroke="none">1</text>
                  <text x="3" y="14" font-size="8" fill="currentColor" stroke="none">2</text>
                  <text x="3" y="20" font-size="8" fill="currentColor" stroke="none">3</text>
                </svg>
              </button>
              <button class="toolbar-btn" @click.stop="turnInto('blockquote')" title="引用">
                <svg viewBox="0 0 24 24" fill="currentColor">
                  <path d="M6 17h3l2-4V7H5v6h3zm8 0h3l2-4V7h-6v6h3z"/>
                </svg>
              </button>
              <button class="toolbar-btn" @click.stop="turnInto('code_block')" title="代码块">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <polyline points="16 18 22 12 16 6"/>
                  <polyline points="8 6 2 12 8 18"/>
                </svg>
              </button>
              <div class="toolbar-divider"></div>
              <button class="toolbar-btn ai-btn" @click.stop="openAI" title="AI 助手">
                <span>✨</span>
              </button>
            </div>
          </Transition>
        </div>
      </Transition>
    </Teleport>
    
    <!-- 拖拽放置指示器 -->
    <Teleport to="body">
      <div 
        v-if="dropIndicator.visible" 
        class="drop-indicator"
        :style="dropIndicatorStyle"
      ></div>
    </Teleport>
    
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
      <div v-if="!showAIDialog && !blockHandle.visible" class="shortcut-hint">
        <kbd>/</kbd> 命令菜单 · <kbd>Ctrl</kbd><kbd>K</kbd> AI 助手
      </div>
    </Transition>
  </div>
</template>

<script setup lang="ts">
import { ref, reactive, computed, onMounted, onUnmounted, nextTick, shallowRef, watch } from 'vue'
import { Editor, rootCtx, defaultValueCtx, editorViewCtx, commandsCtx, schemaCtx } from '@milkdown/kit/core'
import { commonmark, paragraphSchema, headingSchema, bulletListSchema, orderedListSchema, blockquoteSchema, codeBlockSchema, setBlockTypeCommand, wrapInBlockTypeCommand, addBlockTypeCommand, clearTextInCurrentBlockCommand } from '@milkdown/kit/preset/commonmark'
import { gfm } from '@milkdown/kit/preset/gfm'
import { history } from '@milkdown/kit/plugin/history'
import { listener, listenerCtx } from '@milkdown/kit/plugin/listener'
// BlockProvider removed - using direct editor view monitoring
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
const editor = shallowRef<Editor | null>(null)
const editorCtx = shallowRef<any>(null)

// Block handle state
const blockHandle = reactive({
  visible: false,
  top: 0,
  left: 0,
  activeNode: null as any,
  activePos: 0,
})

// Drop indicator for drag
const dropIndicator = reactive({
  visible: false,
  top: 0,
  left: 0,
  width: 0,
})

const showToolbar = ref(false)
const isDragging = ref(false)
const dragSourcePos = ref<number | null>(null)
let expandTimer: ReturnType<typeof setTimeout> | null = null
let collapseTimer: ReturnType<typeof setTimeout> | null = null

const blockHandleStyle = computed(() => ({
  top: `${blockHandle.top}px`,
  left: `${blockHandle.left}px`,
}))

const dropIndicatorStyle = computed(() => ({
  top: `${dropIndicator.top}px`,
  left: `${dropIndicator.left}px`,
  width: `${dropIndicator.width}px`,
}))

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

  editor.value = await Editor.make()
    .config((ctx) => {
      ctx.set(rootCtx, editorRef.value)
      ctx.set(defaultValueCtx, defaultContent)
      
      ctx.get(listenerCtx).markdownUpdated((ctx, markdown) => {
        emit('update:modelValue', markdown)
        editorCtx.value = ctx
      })
    })
    .use(commonmark)
    .use(gfm)
    .use(history)
    .use(listener)
    .create()
  
  // Store context for command execution
  editorCtx.value = editor.value.ctx
  
  // Watch for block updates using editor view
  const updateBlockHandle = () => {
    if (!editor.value || !editorRef.value) return
    
    try {
      const view = editor.value.ctx.get(editorViewCtx)
      const { selection, doc } = view.state
      const { $from } = selection
      
      // Find the parent block node
      let depth = $from.depth
      let node = $from.node(depth)
      
      // Skip text nodes, find the actual block
      while (depth > 0 && node.isText) {
        depth--
        node = $from.node(depth)
      }
      
      // Check if document has content
      if (doc.content.size <= 2) {
        blockHandle.visible = false
        showToolbar.value = false
        return
      }
      
      // Get the position and element
      const pos = $from.before(depth)
      const dom = view.nodeDOM(pos)
      
      if (dom && dom instanceof HTMLElement) {
        const rect = dom.getBoundingClientRect()
        const editorRect = editorRef.value.getBoundingClientRect()
        
        blockHandle.visible = true
        blockHandle.top = rect.top
        blockHandle.left = editorRect.left - 28
        blockHandle.activeNode = { node, pos, el: dom }
        blockHandle.activePos = pos
      } else {
        blockHandle.visible = false
        showToolbar.value = false
      }
    } catch (e) {
      // Silently ignore errors during updates
    }
  }
  
  // Update on selection changes
  const pollInterval = setInterval(updateBlockHandle, 100)
  
  // Also update on editor events
  editor.value.ctx.get(editorViewCtx).dom.addEventListener('click', updateBlockHandle)
  editor.value.ctx.get(editorViewCtx).dom.addEventListener('keyup', updateBlockHandle)
  
  // Store for cleanup
  ;(editor.value as any)._pollInterval = pollInterval
  ;(editor.value as any)._updateBlockHandle = updateBlockHandle
}

// Block handle interactions - 监听整个容器
function handleMouseEnter() {
  if (collapseTimer) {
    clearTimeout(collapseTimer)
    collapseTimer = null
  }
  if (!showToolbar.value && !isDragging.value) {
    if (expandTimer) clearTimeout(expandTimer)
    expandTimer = setTimeout(() => {
      showToolbar.value = true
    }, 100)
  }
}

function handleMouseLeave() {
  if (expandTimer) {
    clearTimeout(expandTimer)
    expandTimer = null
  }
  if (showToolbar.value && !isDragging.value) {
    collapseTimer = setTimeout(() => {
      showToolbar.value = false
    }, 300)
  }
}

// 拖拽功能
function handleDragStart(e: DragEvent) {
  isDragging.value = true
  showToolbar.value = false
  dragSourcePos.value = blockHandle.activePos
  
  if (e.dataTransfer) {
    e.dataTransfer.effectAllowed = 'move'
    e.dataTransfer.setData('text/plain', String(blockHandle.activePos))
  }
  
  // 添加全局拖放监听
  const editorDom = editor.value?.ctx.get(editorViewCtx).dom
  if (editorDom) {
    editorDom.addEventListener('dragover', handleDragOver)
    editorDom.addEventListener('drop', handleDrop)
  }
}

function handleDragEnd(_e: DragEvent) {
  isDragging.value = false
  dragSourcePos.value = null
  dropIndicator.visible = false
  
  // 移除全局拖放监听
  const editorDom = editor.value?.ctx.get(editorViewCtx).dom
  if (editorDom) {
    editorDom.removeEventListener('dragover', handleDragOver)
    editorDom.removeEventListener('drop', handleDrop)
  }
}

function handleDragOver(e: DragEvent) {
  e.preventDefault()
  if (!e.dataTransfer) return
  
  e.dataTransfer.dropEffect = 'move'
  
  // 计算放置位置
  if (!editor.value || !editorRef.value) return
  
  try {
    const view = editor.value.ctx.get(editorViewCtx)
    const editorDom = view.dom
    const rect = editorDom.getBoundingClientRect()
    
    // 计算鼠标在编辑器中的位置
    const x = e.clientX - rect.left
    const y = e.clientY - rect.top
    
    // 使用 ProseMirror 的 posAtCoords 获取位置
    const posAtCoords = view.posAtCoords({ left: e.clientX, top: e.clientY })
    
    if (posAtCoords) {
      const $pos = view.state.doc.resolve(posAtCoords.pos)
      
      // 找到块级位置
      let depth = $pos.depth
      while (depth > 0) {
        const node = $pos.node(depth)
        if (node.isBlock && node.isTextblock) {
          break
        }
        depth--
      }
      
      const blockPos = $pos.before(depth)
      const dom = view.nodeDOM(blockPos)
      
      if (dom && dom instanceof HTMLElement) {
        const domRect = dom.getBoundingClientRect()
        const editorRect = editorRef.value.getBoundingClientRect()
        
        dropIndicator.visible = true
        dropIndicator.top = domRect.bottom - 2
        dropIndicator.left = editorRect.left
        dropIndicator.width = editorRect.width
      }
    }
  } catch (err) {
    // 忽略错误
  }
}

function handleDrop(e: DragEvent) {
  e.preventDefault()
  
  if (dragSourcePos.value === null || !editor.value) return
  
  try {
    const view = editor.value.ctx.get(editorViewCtx)
    const state = view.state
    
    // 获取目标位置
    const posAtCoords = view.posAtCoords({ left: e.clientX, top: e.clientY })
    if (!posAtCoords) return
    
    const $pos = state.doc.resolve(posAtCoords.pos)
    let depth = $pos.depth
    while (depth > 0) {
      const node = $pos.node(depth)
      if (node.isBlock && node.isTextblock) {
        break
      }
      depth--
    }
    const targetPos = $pos.before(depth)
    
    // 不允许拖到自己的位置
    if (targetPos === dragSourcePos.value) {
      dropIndicator.visible = false
      return
    }
    
    // 执行移动
    const sourcePos = dragSourcePos.value
    const sourceNode = state.doc.nodeAt(sourcePos)
    
    if (!sourceNode) return
    
    // 创建事务
    let tr = state.tr
    
    // 先删除源节点
    tr = tr.delete(sourcePos, sourcePos + sourceNode.nodeSize)
    
    // 调整目标位置（如果源在目标前面，删除后位置会改变）
    const adjustedTarget = sourcePos < targetPos ? targetPos - sourceNode.nodeSize : targetPos
    
    // 插入到新位置
    tr = tr.insert(adjustedTarget, sourceNode)
    
    view.dispatch(tr)
    
    // 更新选区到新位置
    const newPos = adjustedTarget
    view.dispatch(view.state.tr.setSelection(
      new (view.state.selection.constructor as any)(
        view.state.doc.resolve(newPos + 1),
        view.state.doc.resolve(newPos + sourceNode.nodeSize - 1)
      )
    ))
  } catch (err) {
    console.error('Drop error:', err)
  }
  
  dropIndicator.visible = false
}

// Block operations
function executeCommand(commandKey: string, args?: any) {
  if (!editorCtx.value) return
  
  try {
    const commands = editorCtx.value.get(commandsCtx)
    commands.call(commandKey, args)
  } catch (e) {
    console.error('Command error:', e)
  }
}

function addBlockBelow() {
  if (!editorCtx.value) return
  
  const schema = editorCtx.value.get(schemaCtx)
  const paragraph = paragraphSchema.type(editorCtx.value)
  
  executeCommand(addBlockTypeCommand.key, { nodeType: paragraph })
  showToolbar.value = false
}

function deleteBlock() {
  if (!editorCtx.value) return
  
  const view = editorCtx.value.get(editorViewCtx)
  const { state } = view
  const { $from, $to } = state.selection
  
  // Delete the current block
  const tr = state.tr.delete($from.before(), $to.after())
  view.dispatch(tr)
  
  showToolbar.value = false
}

function turnInto(type: string, level?: number) {
  if (!editorCtx.value) return
  
  const schema = editorCtx.value.get(schemaCtx)
  
  try {
    const commands = editorCtx.value.get(commandsCtx)
    
    // Clear any markdown prefix first
    commands.call(clearTextInCurrentBlockCommand.key)
    
    switch (type) {
      case 'heading': {
        const heading = headingSchema.type(editorCtx.value)
        commands.call(setBlockTypeCommand.key, {
          nodeType: heading,
          attrs: { level }
        })
        break
      }
      case 'paragraph': {
        const paragraph = paragraphSchema.type(editorCtx.value)
        commands.call(setBlockTypeCommand.key, { nodeType: paragraph })
        break
      }
      case 'bullet_list': {
        const bulletList = bulletListSchema.type(editorCtx.value)
        const listItem = schema.nodes['list_item']
        commands.call(wrapInBlockTypeCommand.key, { nodeType: bulletList })
        break
      }
      case 'ordered_list': {
        const orderedList = orderedListSchema.type(editorCtx.value)
        commands.call(wrapInBlockTypeCommand.key, { nodeType: orderedList })
        break
      }
      case 'blockquote': {
        const blockquote = blockquoteSchema.type(editorCtx.value)
        commands.call(wrapInBlockTypeCommand.key, { nodeType: blockquote })
        break
      }
      case 'code_block': {
        const codeBlock = codeBlockSchema.type(editorCtx.value)
        commands.call(setBlockTypeCommand.key, { nodeType: codeBlock })
        break
      }
    }
  } catch (e) {
    console.error('Turn into error:', e)
  }
  
  showToolbar.value = false
}

function openAI() {
  showToolbar.value = false
  showAIDialog.value = true
  nextTick(() => aiInputRef.value?.focus())
}

// Update content when modelValue changes externally
watch(() => props.modelValue, (newValue) => {
  if (editor.value && newValue !== undefined) {
    // Only update if significantly different to avoid cursor jump
    // This is a simplified approach
  }
})

// AI Dialog
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
    showAIDialog.value = true
    nextTick(() => aiInputRef.value?.focus())
  }
}

// Destroy editor
async function destroyEditor() {
  if (editor.value) {
    // Clear poll interval
    if ((editor.value as any)._pollInterval) {
      clearInterval((editor.value as any)._pollInterval)
    }
    
    // Remove event listeners
    try {
      const view = editor.value.ctx.get(editorViewCtx)
      const updateFn = (editor.value as any)._updateBlockHandle
      if (updateFn) {
        view.dom.removeEventListener('click', updateFn)
        view.dom.removeEventListener('keyup', updateFn)
      }
    } catch (e) {
      // Ignore errors during cleanup
    }
    
    try {
      await editor.value.destroy()
    } catch (e) {
      console.error('Destroy error:', e)
    }
    editor.value = null
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
  if (expandTimer) clearTimeout(expandTimer)
  if (collapseTimer) clearTimeout(collapseTimer)
})

defineExpose({ openAI: () => { showAIDialog.value = true } })
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

/* Milkdown Editor Styles - Notion-like */
.editor-root :deep(.milkdown) {
  font-family: -apple-system, BlinkMacMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif;
  font-size: 16px;
  line-height: 1.6;
  color: #37352f;
  outline: none;
  min-height: 100%;
}

.editor-root :deep(.milkdown .ProseMirror) {
  outline: none;
  min-height: 100%;
}

/* Headings */
.editor-root :deep(.milkdown h1) {
  font-size: 2.25rem;
  font-weight: 700;
  margin: 0 0 0.5rem;
  line-height: 1.2;
  letter-spacing: -0.03em;
  color: #37352f;
}

.editor-root :deep(.milkdown h2) {
  font-size: 1.5rem;
  font-weight: 600;
  margin: 1rem 0 0.375rem;
  line-height: 1.3;
}

.editor-root :deep(.milkdown h3) {
  font-size: 1.25rem;
  font-weight: 600;
  margin: 0.75rem 0 0.25rem;
}

/* Paragraph */
.editor-root :deep(.milkdown p) {
  margin: 0.25rem 0;
}

/* Code */
.editor-root :deep(.milkdown code) {
  background: rgba(135, 131, 120, 0.15);
  color: #eb5757;
  padding: 0.2em 0.4em;
  border-radius: 3px;
  font-family: 'SF Mono', 'Fira Code', Consolas, monospace;
  font-size: 85%;
}

.editor-root :deep(.milkdown pre) {
  background: #f7f6f3;
  border-radius: 4px;
  padding: 16px;
  margin: 8px 0;
  overflow-x: auto;
}

.editor-root :deep(.milkdown pre code) {
  background: transparent;
  color: inherit;
  padding: 0;
  font-size: 14px;
}

/* Blockquote */
.editor-root :deep(.milkdown blockquote) {
  border-left: 3px solid #37352f;
  padding-left: 16px;
  margin: 8px 0;
  color: #37352f;
}

/* Lists */
.editor-root :deep(.milkdown ul),
.editor-root :deep(.milkdown ol) {
  margin: 4px 0;
  padding-left: 24px;
}

.editor-root :deep(.milkdown li) {
  margin: 2px 0;
}

.editor-root :deep(.milkdown li p) {
  margin: 0;
}

/* Tables */
.editor-root :deep(.milkdown table) {
  border-collapse: collapse;
  width: 100%;
  margin: 8px 0;
}

.editor-root :deep(.milkdown th),
.editor-root :deep(.milkdown td) {
  border: 1px solid #e0e0e0;
  padding: 8px 12px;
  text-align: left;
}

.editor-root :deep(.milkdown th) {
  background: #f7f6f3;
  font-weight: 600;
}

/* Links */
.editor-root :deep(.milkdown a) {
  color: #2383e2;
  text-decoration: underline;
  text-underline-offset: 2px;
}

.editor-root :deep(.milkdown a:hover) {
  color: #0077d4;
}

/* HR */
.editor-root :deep(.milkdown hr) {
  border: none;
  border-top: 1px solid #e0e0e0;
  margin: 16px 0;
}

/* Images */
.editor-root :deep(.milkdown img) {
  max-width: 100%;
  border-radius: 4px;
  margin: 8px 0;
}

/* Selection highlight */
.editor-root :deep(.milkdown ::selection) {
  background: rgba(35, 131, 226, 0.28);
}

/* Custom Block Handle */
.custom-block-handle {
  position: fixed;
  z-index: 100;
  user-select: none;
}

.handle-trigger {
  width: 24px;
  height: 24px;
  display: flex;
  align-items: center;
  justify-content: center;
  cursor: grab;
  color: #9b9a97;
  border-radius: 4px;
  transition: all 0.15s ease;
  background: transparent;
}

.handle-trigger:hover,
.custom-block-handle:hover .handle-trigger {
  background: rgba(55, 53, 47, 0.08);
  color: #37352f;
}

.handle-trigger:active,
.handle-trigger.dragging {
  cursor: grabbing;
  background: rgba(55, 53, 47, 0.12);
}

.handle-toolbar {
  position: absolute;
  top: 0;
  left: 28px;
  display: flex;
  align-items: center;
  gap: 2px;
  background: #fff;
  border: 1px solid #e5e5e5;
  border-radius: 6px;
  padding: 4px 6px;
  box-shadow: 0 2px 8px rgba(0, 0, 0, 0.1);
  white-space: nowrap;
}

/* Drop Indicator */
.drop-indicator {
  position: fixed;
  height: 2px;
  background: #2383e2;
  border-radius: 1px;
  pointer-events: none;
  z-index: 101;
}

.toolbar-btn {
  width: 28px;
  height: 28px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: none;
  border-radius: 4px;
  cursor: pointer;
  color: #37352f;
  transition: all 0.1s ease;
}

.toolbar-btn:hover {
  background: rgba(55, 53, 47, 0.08);
}

.toolbar-btn:active {
  background: rgba(55, 53, 47, 0.12);
}

.toolbar-btn svg {
  width: 16px;
  height: 16px;
}

.btn-text {
  font-size: 12px;
  font-weight: 600;
}

.toolbar-divider {
  width: 1px;
  height: 16px;
  background: #e5e5e5;
  margin: 0 4px;
}

.ai-btn {
  background: linear-gradient(135deg, #7c3aed, #2563eb);
  color: #fff;
  border-radius: 4px;
}

.ai-btn:hover {
  opacity: 0.9;
  transform: scale(1.05);
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

.expand-enter-active,
.expand-leave-active {
  transition: all 0.15s ease;
}

.expand-enter-from,
.expand-leave-to {
  opacity: 0;
  transform: translateX(-8px);
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