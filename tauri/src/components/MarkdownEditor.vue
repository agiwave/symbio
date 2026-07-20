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
          <!-- 未展开时的触发按钮 -->
          <div v-if="!showToolbar" class="handle-trigger">
            <svg viewBox="0 0 24 24" fill="currentColor" width="18" height="18">
              <circle cx="9" cy="6" r="1.5"/><circle cx="15" cy="6" r="1.5"/><circle cx="9" cy="12" r="1.5"/><circle cx="15" cy="12" r="1.5"/><circle cx="9" cy="18" r="1.5"/><circle cx="15" cy="18" r="1.5"/>
            </svg>
          </div>
          
          <!-- 展开的工具条 -->
          <Transition name="expand">
            <div v-if="showToolbar" class="handle-toolbar">
              <button class="toolbar-btn drag-btn" :class="{ active: isDragging }" title="拖拽移动" @mousedown="handleDragMouseDown" @click.stop.prevent>
                <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16">
                  <circle cx="9" cy="5" r="1.5"/><circle cx="15" cy="5" r="1.5"/><circle cx="9" cy="12" r="1.5"/><circle cx="15" cy="12" r="1.5"/><circle cx="9" cy="19" r="1.5"/><circle cx="15" cy="19" r="1.5"/>
                </svg>
              </button>
              <div class="toolbar-divider"></div>
              <button class="toolbar-btn" @click.stop="addBlockBelow" title="添加块">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><line x1="12" y1="5" x2="12" y2="19"/><line x1="5" y1="12" x2="19" y2="12"/></svg>
              </button>
              <button class="toolbar-btn" @click.stop="deleteBlock" title="删除块">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="3 6 5 6 21 6"/><path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2"/></svg>
              </button>
              <div class="toolbar-divider"></div>
              <button class="toolbar-btn" @click.stop="turnInto('heading')" title="标题"><span class="btn-text">H</span></button>
              <button class="toolbar-btn" @click.stop="turnInto('list')" title="列表">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><line x1="9" y1="6" x2="20" y2="6"/><line x1="9" y1="12" x2="20" y2="12"/><line x1="9" y1="18" x2="20" y2="18"/><circle cx="4" cy="6" r="1.5" fill="currentColor" stroke="none"/><circle cx="4" cy="12" r="1.5" fill="currentColor" stroke="none"/><circle cx="4" cy="18" r="1.5" fill="currentColor" stroke="none"/></svg>
              </button>
              <button class="toolbar-btn" @click.stop="turnInto('code_block')" title="代码块">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="16 18 22 12 16 6"/><polyline points="8 6 2 12 8 18"/></svg>
              </button>
              <button class="toolbar-btn" @click.stop="turnInto('blockquote')" title="引用">
                <svg viewBox="0 0 24 24" fill="currentColor"><path d="M6 17h3l2-4V7H5v6h3zm8 0h3l2-4V7h-6v6h3z"/></svg>
              </button>
              <button class="toolbar-btn" @click.stop="turnInto('paragraph')" title="正文">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><line x1="3" y1="6" x2="21" y2="6"/><line x1="3" y1="12" x2="15" y2="12"/><line x1="3" y1="18" x2="18" y2="18"/></svg>
              </button>
              <div class="toolbar-divider"></div>
              <button class="toolbar-btn" @click.stop="increaseLevel" title="提高级别"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="18 15 12 9 6 15"/></svg></button>
              <button class="toolbar-btn" @click.stop="decreaseLevel" title="降低级别"><svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2"><polyline points="6 9 12 15 18 9"/></svg></button>
            </div>
          </Transition>
        </div>
      </Transition>
    </Teleport>
    
    <!-- 拖拽放置指示器 -->
    <Teleport to="body">
      <div v-if="dropIndicator.visible" class="drop-indicator" :style="dropIndicatorStyle"></div>
    </Teleport>
    
    <!-- 快捷键提示 -->
    <Transition name="fade">
      <div v-if="!blockHandle.visible" class="shortcut-hint"><kbd>/</kbd> 命令菜单 · 选中文本后自动显示 Model \u52a9\u624b</div>
    </Transition>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, shallowRef, watch, toRef } from 'vue'
import { Editor, rootCtx, defaultValueCtx, editorViewCtx, commandsCtx, parserCtx, CmdKey } from '@milkdown/kit/core'
import { commonmark, paragraphSchema, headingSchema, bulletListSchema, blockquoteSchema, codeBlockSchema, setBlockTypeCommand, wrapInBlockTypeCommand } from '@milkdown/kit/preset/commonmark'
import { gfm } from '@milkdown/kit/preset/gfm'
import { logger } from '@/utils/logger'
import { history } from '@milkdown/kit/plugin/history'
import { listener, listenerCtx } from '@milkdown/kit/plugin/listener'
import { $view } from '@milkdown/utils'
import { diagram, diagramSchema } from '@milkdown/plugin-diagram'
import { math } from '@milkdown/plugin-math'
import { prism } from '@milkdown/plugin-prism'
import { setModelContext } from '@/composables/useModelContext'

// Composables
import { useEditorDrag } from '@/composables/useMilkdownDrag'
import { useBlockHandle } from '@/composables/useBlockHandle'
import { useEditorSelection } from '@/composables/useEditorSelection'

// Styles
import 'katex/dist/katex.min.css'
import 'prismjs/themes/prism-tomorrow.css'

const props = defineProps<{
  modelValue: string
  filePath?: string
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string]
  'content-change': [value: string]
  'request-save': []
}>()

const editorRef = ref<HTMLElement | null>(null)
const editor = shallowRef<Editor | null>(null)
const editorCtx = shallowRef<any>(null)
const lastMarkdown = ref(props.modelValue)

// --- Composable: Drag ---
const activePos = ref(0)
const { isDragging, dropIndicator, handleDragMouseDown } = useEditorDrag(
  editor, editorRef, activePos,
  () => { /* update state if needed */ }
)

// --- Composable: Block Handle ---
const { blockHandle, showToolbar, updateBlockHandle, handleMouseEnter, handleMouseLeave, clearTimers } = useBlockHandle(
  editor, editorRef, isDragging
)

// Watch blockHandle.activePos to sync with drag composable
watch(() => blockHandle.activePos, (val) => { activePos.value = val })

// --- Composable: Selection ---
const { updateModelContextFromSelection, handleEditorMouseUp } = useEditorSelection(
  editor, toRef(props, 'filePath'), toRef(props, 'modelValue')
)

// --- Styles Computed ---
const blockHandleStyle = computed(() => ({ top: `${blockHandle.top}px`, left: `${blockHandle.left}px` }))
const dropIndicatorStyle = computed(() => ({
  top: `${dropIndicator.value.top}px`, left: `${dropIndicator.value.left}px`, width: `${dropIndicator.value.width}px`
}))

// Custom diagram view (Mermaid)
const diagramView = $view(diagramSchema.node, () => {
  return (node, _view, _getPos) => {
    const dom = document.createElement('div')
    dom.classList.add('mermaid-container')
    const render = async () => {
      const code = node.attrs.value
      if (!code.trim()) { dom.innerHTML = '<div class="mermaid-empty">Empty</div>'; return; }
      try {
        const id = `mermaid-${Math.random().toString(36).substr(2, 9)}`
        const { default: mermaid } = await import('mermaid')
        const { svg } = await mermaid.render(id, code)
        dom.innerHTML = svg
      } catch (e: any) { dom.innerHTML = `<pre class="mermaid-error">${e.message}</pre>` }
    }
    render()
    return {
      dom,
      update: (updatedNode) => {
        if (updatedNode.type.name !== 'diagram') return false
        if (updatedNode.attrs.value === node.attrs.value) return true
        node = updatedNode
        render()
        return true
      },
      destroy: () => { dom.innerHTML = '' }
    }
  }
})

async function initEditor() {
  if (!editorRef.value) return
  const { default: mermaid } = await import('mermaid')
  mermaid.initialize({ startOnLoad: false, theme: 'default' })
  
  editor.value = await Editor.make()
    .config((ctx) => {
      ctx.set(rootCtx, editorRef.value)
      ctx.set(defaultValueCtx, props.modelValue || '# 开始创作')
      ctx.get(listenerCtx).markdownUpdated((ctx, markdown) => {
        if (markdown === lastMarkdown.value) return
        lastMarkdown.value = markdown
        emit('update:modelValue', markdown)
        emit('content-change', markdown)
        editorCtx.value = ctx
      })
      ctx.get(listenerCtx).selectionUpdated(updateModelContextFromSelection)
    })
    .use(commonmark).use(gfm).use(history).use(listener).use(diagram).use(diagramView).use(prism).use(math).create()
  
  editorCtx.value = editor.value.ctx
  const view = editor.value.ctx.get(editorViewCtx)
  const pollInterval = setInterval(updateBlockHandle, 100)
  view.dom.addEventListener('click', updateBlockHandle)
  view.dom.addEventListener('keyup', updateBlockHandle)
  view.dom.addEventListener('mouseup', handleEditorMouseUp)
  
  ;(editor.value as any)._cleanup = () => {
    clearInterval(pollInterval)
    view.dom.removeEventListener('click', updateBlockHandle)
    view.dom.removeEventListener('keyup', updateBlockHandle)
    view.dom.removeEventListener('mouseup', handleEditorMouseUp)
  }
}

// Command execution
function executeCommand(commandKey: string | CmdKey<any>, args?: any) {
  if (!editorCtx.value) return
  try { editorCtx.value.get(commandsCtx).call(commandKey, args) } catch (e) { logger.error('MarkdownEditor', 'command call failed', e) }
}

function addBlockBelow() { executeCommand('InsertParagraph', {}); showToolbar.value = false }
function deleteBlock() {
  const view = editorCtx.value.get(editorViewCtx)
  const { state } = view
  view.dispatch(state.tr.delete(state.selection.$from.before(), state.selection.$to.after()))
  showToolbar.value = false
}

function turnInto(type: string) {
  if (!editorCtx.value) return
  const commands = editorCtx.value.get(commandsCtx)
  const view = editorCtx.value.get(editorViewCtx)
  const { $from } = view.state.selection
  switch (type) {
    case 'heading': {
      let level = 1
      if ($from.node($from.depth).type.name === 'heading') level = $from.node($from.depth).attrs.level || 1
      commands.call(setBlockTypeCommand.key, { nodeType: headingSchema.type(editorCtx.value), attrs: { level } })
      break
    }
    case 'list': commands.call(wrapInBlockTypeCommand.key, { nodeType: bulletListSchema.type(editorCtx.value) }); break
    case 'paragraph': commands.call(setBlockTypeCommand.key, { nodeType: paragraphSchema.type(editorCtx.value) }); break
    case 'blockquote': commands.call(wrapInBlockTypeCommand.key, { nodeType: blockquoteSchema.type(editorCtx.value) }); break
    case 'code_block': commands.call(setBlockTypeCommand.key, { nodeType: codeBlockSchema.type(editorCtx.value) }); break
  }
  showToolbar.value = false
}

function increaseLevel() {
  const node = editorCtx.value.get(editorViewCtx).state.selection.$from.node()
  if (node.type.name === 'heading' && node.attrs.level > 1) {
    executeCommand(setBlockTypeCommand.key, { nodeType: headingSchema.type(editorCtx.value), attrs: { level: node.attrs.level - 1 } })
  }
  showToolbar.value = false
}

function decreaseLevel() {
  const node = editorCtx.value.get(editorViewCtx).state.selection.$from.node()
  if (node.type.name === 'heading' && node.attrs.level < 6) {
    executeCommand(setBlockTypeCommand.key, { nodeType: headingSchema.type(editorCtx.value), attrs: { level: node.attrs.level + 1 } })
  } else if (node.type.name === 'paragraph') {
    executeCommand(setBlockTypeCommand.key, { nodeType: headingSchema.type(editorCtx.value), attrs: { level: 6 } })
  }
  showToolbar.value = false
}

watch(() => props.modelValue, async (val) => {
  if (!editor.value || val === undefined || val === lastMarkdown.value) return
  const view = editor.value.ctx.get(editorViewCtx)
  const newDoc = await editor.value.ctx.get(parserCtx)(val || '')
  if (newDoc && !newDoc.eq(view.state.doc)) {
    lastMarkdown.value = val
    view.dispatch(view.state.tr.replaceWith(0, view.state.doc.content.size, newDoc.content))
  }
})

function handleKeydown(e: KeyboardEvent) {
  if ((e.ctrlKey || e.metaKey) && e.key === 's') { e.preventDefault(); e.stopPropagation(); emit('request-save') }
}

onMounted(() => {
  initEditor()
  document.addEventListener('keydown', handleKeydown)
  setModelContext({ filePath: props.filePath, fileContent: props.modelValue })
})

onUnmounted(async () => {
  if (editor.value) {
    (editor.value as any)._cleanup?.()
    await editor.value.destroy()
  }
  document.removeEventListener('keydown', handleKeydown)
  clearTimers()
})
</script>

<style scoped>
.notion-editor { position: relative; height: 100%; width: 100%; background: #fff; display: flex; flex-direction: column; }
.editor-root { flex: 1; overflow-y: auto; padding: 32px 48px 32px 64px; min-height: 0; counter-reset: line; position: relative; }
.editor-root::before { content: ''; position: absolute; left: 0; top: 32px; width: 48px; bottom: 0; background: #f7f7f5; border-right: 1px solid #e8e8e6; pointer-events: none; }
.editor-root :deep(.milkdown) { font-family: -apple-system, BlinkMacMacSystemFont, 'Segoe UI', Helvetica, Arial, sans-serif; font-size: 16px; line-height: 1.6; color: #37352f; outline: none; min-height: 100%; }
.editor-root :deep(.ProseMirror > *) { position: relative; counter-increment: line; }
.editor-root :deep(.ProseMirror > *)::before { content: counter(line); position: absolute; left: -56px; top: 0; width: 40px; text-align: right; font-family: 'Fira Code', monospace; font-size: 12px; color: #b0b0ab; pointer-events: none; }
.editor-root :deep(.milkdown h1) { font-size: 2.25rem; font-weight: 700; margin: 0 0 0.5rem; }
.editor-root :deep(.milkdown h2) { font-size: 1.5rem; font-weight: 600; margin: 1rem 0 0.375rem; }
.editor-root :deep(.milkdown code) { background: rgba(135, 131, 120, 0.15); color: #eb5757; padding: 0.2em 0.4em; border-radius: 3px; font-family: monospace; font-size: 85%; }
.editor-root :deep(.milkdown pre) { background: #f7f6f3; border-radius: 4px; padding: 16px; margin: 8px 0; overflow-x: auto; }
.editor-root :deep(.milkdown blockquote) { border-left: 3px solid #37352f; padding-left: 16px; margin: 8px 0; }
.custom-block-handle { position: fixed; z-index: 100; user-select: none; }
.handle-trigger { width: 24px; height: 24px; display: flex; align-items: center; justify-content: center; color: #9b9a97; border-radius: 4px; transition: all 0.15s ease; }
.handle-trigger:hover, .custom-block-handle:hover .handle-trigger { background: rgba(55, 53, 47, 0.08); color: #37352f; }
.handle-toolbar { display: flex; align-items: center; gap: 2px; background: #fff; border: 1px solid #e5e5e5; border-radius: 6px; padding: 4px 6px; box-shadow: 0 2px 8px rgba(0,0,0,0.1); white-space: nowrap; }
.drag-btn { cursor: grab; }
.drag-btn.active { cursor: grabbing; background: rgba(35, 131, 226, 0.2) !important; color: #2383e2; }
.drop-indicator { position: fixed; height: 2px; background: #2383e2; border-radius: 1px; pointer-events: none; z-index: 101; }
.toolbar-btn { width: 28px; height: 28px; display: flex; align-items: center; justify-content: center; background: transparent; border: none; border-radius: 4px; cursor: pointer; color: #37352f; }
.toolbar-btn:hover { background: rgba(55, 53, 47, 0.08); }
.toolbar-divider { width: 1px; height: 16px; background: #e5e5e5; margin: 0 4px; }
.shortcut-hint { position: fixed; bottom: 16px; right: 16px; background: #1f1f1f; color: #fff; padding: 6px 12px; border-radius: 6px; font-size: 12px; z-index: 100; }
.fade-enter-active, .fade-leave-active { transition: opacity 0.15s ease; }
.fade-enter-from, .fade-leave-to { opacity: 0; }
.expand-enter-active, .expand-leave-active { transition: all 0.15s ease; }
.expand-enter-from, .expand-leave-to { opacity: 0; transform: translateX(-8px); }
</style>
