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
          <div 
            v-if="!showToolbar"
            class="handle-trigger"
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
          
          <!-- 展开的工具条 -->
          <Transition name="expand">
            <div v-if="showToolbar" class="handle-toolbar">
              <!-- 拖拽按钮 - 第一个 -->
              <button 
                class="toolbar-btn drag-btn"
                :class="{ active: isDragging }"
                title="拖拽移动"
                @mousedown="handleDragMouseDown"
                @click.stop.prevent
              >
                <svg viewBox="0 0 24 24" fill="currentColor" width="16" height="16">
                  <circle cx="9" cy="5" r="1.5"/>
                  <circle cx="15" cy="5" r="1.5"/>
                  <circle cx="9" cy="12" r="1.5"/>
                  <circle cx="15" cy="12" r="1.5"/>
                  <circle cx="9" cy="19" r="1.5"/>
                  <circle cx="15" cy="19" r="1.5"/>
                </svg>
              </button>
              <div class="toolbar-divider"></div>
              <button class="toolbar-btn" @click.stop="addBlockBelow" title="添加块">
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
              <button class="toolbar-btn" @click.stop="turnInto('heading')" title="标题">
                <span class="btn-text">H</span>
              </button>
              <button class="toolbar-btn" @click.stop="turnInto('list')" title="列表">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <line x1="9" y1="6" x2="20" y2="6"/>
                  <line x1="9" y1="12" x2="20" y2="12"/>
                  <line x1="9" y1="18" x2="20" y2="18"/>
                  <circle cx="4" cy="6" r="1.5" fill="currentColor" stroke="none"/>
                  <circle cx="4" cy="12" r="1.5" fill="currentColor" stroke="none"/>
                  <circle cx="4" cy="18" r="1.5" fill="currentColor" stroke="none"/>
                </svg>
              </button>
              <button class="toolbar-btn" @click.stop="turnInto('code_block')" title="代码块">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <polyline points="16 18 22 12 16 6"/>
                  <polyline points="8 6 2 12 8 18"/>
                </svg>
              </button>
              <button class="toolbar-btn" @click.stop="turnInto('blockquote')" title="引用">
                <svg viewBox="0 0 24 24" fill="currentColor">
                  <path d="M6 17h3l2-4V7H5v6h3zm8 0h3l2-4V7h-6v6h3z"/>
                </svg>
              </button>
              <button class="toolbar-btn" @click.stop="turnInto('paragraph')" title="正文">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <line x1="3" y1="6" x2="21" y2="6"/>
                  <line x1="3" y1="12" x2="15" y2="12"/>
                  <line x1="3" y1="18" x2="18" y2="18"/>
                </svg>
              </button>
              <div class="toolbar-divider"></div>
              <button class="toolbar-btn" @click.stop="increaseLevel" title="提高级别">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <polyline points="18 15 12 9 6 15"/>
                </svg>
              </button>
              <button class="toolbar-btn" @click.stop="decreaseLevel" title="降低级别">
                <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                  <polyline points="6 9 12 15 18 9"/>
                </svg>
              </button>
              <div class="toolbar-divider"></div>
              <!-- AI 按钮已移至全局统一实现 -->
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
    
    <!-- 快捷键提示 -->
    <Transition name="fade">
      <div v-if="!blockHandle.visible" class="shortcut-hint">
        <kbd>/</kbd> 命令菜单 · 选中文本后自动显示 AI 助手
      </div>
    </Transition>
  </div>
</template>

<script setup lang="ts">
import { ref, reactive, computed, onMounted, onUnmounted, shallowRef, watch } from 'vue'
import { Editor, rootCtx, defaultValueCtx, editorViewCtx, commandsCtx, parserCtx } from '@milkdown/kit/core'
import { commonmark, paragraphSchema, headingSchema, bulletListSchema, blockquoteSchema, codeBlockSchema, setBlockTypeCommand, wrapInBlockTypeCommand } from '@milkdown/kit/preset/commonmark'
import { gfm } from '@milkdown/kit/preset/gfm'
import { history } from '@milkdown/kit/plugin/history'
import { listener, listenerCtx } from '@milkdown/kit/plugin/listener'
import { setAIContext } from '@/composables/useAIContext'

const props = defineProps<{
  modelValue: string
  /** 文件路径（可选，由父组件传入） */
  filePath?: string
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string]
  'content-change': [value: string]
  'request-save': []
}>()

// DOM refs
const editorRef = ref<HTMLElement | null>(null)

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

// 保存选区信息（用于更新全局 AI 上下文）
interface SelectionInfo {
  text: string
  rect: DOMRect
  startLine?: number
  endLine?: number
}
const savedSelection = shallowRef<SelectionInfo | null>(null)

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
        emit('content-change', markdown)
        editorCtx.value = ctx
      })

      // 监听光标位置变化，更新全局 AI 上下文
      ctx.get(listenerCtx).selectionUpdated((ctx) => {
        if (!editor.value || !props.filePath) return

        try {
          const view = ctx.get(editorViewCtx)
          const { state } = view
          const { from, to } = state.selection

          if (from !== to) {
            // 有文本选区
            const selectedTextFromDoc = state.doc.textBetween(from, to, '\n')
            const textBefore = state.doc.textBetween(0, from, '\n')

            const linesBefore = textBefore.split('\n').length
            const selectedLines = selectedTextFromDoc.split('\n').length

            setAIContext({
              filePath: props.filePath,
              fileContent: props.modelValue,
              selectedText: selectedTextFromDoc.trim() || undefined,
              startLine: linesBefore,
              endLine: linesBefore + selectedLines - 1,
            })

            // 同时更新本地 savedSelection（用于 UI 显示）
            const range = document.getSelection()?.getRangeAt(0)
            const rect = range?.getBoundingClientRect() || { left: 0, top: 0, width: 0, height: 0 } as DOMRect
            savedSelection.value = {
              text: selectedTextFromDoc.trim(),
              rect,
              startLine: linesBefore,
              endLine: linesBefore + selectedLines - 1,
            }
          } else {
            // 无选区，只更新光标位置
            const textBefore = state.doc.textBetween(0, from, '\n')
            const currentLine = textBefore.split('\n').length

            setAIContext({
              filePath: props.filePath,
              fileContent: props.modelValue,
              selectedText: undefined,
              startLine: currentLine,
              endLine: currentLine,
            })

            // 清除本地选区
            savedSelection.value = null
          }
        } catch (e) {
          // 忽略错误
        }
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
    // 拖拽过程中不更新 block handle 状态
    if (isDragging.value) return
    
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
        
        // 手柄显示在编辑器的 padding 区域内（左侧 48px padding）
        // 固定在编辑器左边界位置，不随内容块变化
        const handleLeft = editorRect.left + 8 // padding 区域内，留 8px 边距
        
        blockHandle.visible = true
        blockHandle.top = rect.top
        blockHandle.left = handleLeft
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
  
  // 监听编辑器 mouseup 事件 - 检测文字选择并打开/更新 AI 对话框
  const handleEditorMouseUp = (_e: MouseEvent) => {
    // 延迟检查，确保选区已经稳定
    setTimeout(() => {
      try {
        const selection = window.getSelection()
        if (selection && !selection.isCollapsed) {
          const text = selection.toString().trim()
          if (text.length > 0) {
            // 获取选区的位置信息
            const range = selection.getRangeAt(0)
            const rect = range.getBoundingClientRect()

            // 尝试从 ProseMirror 状态获取精确的行号/位置
            let startLine: number | undefined
            let endLine: number | undefined
            let selectedContent = text

            if (editor.value && editor.value.ctx) {
              try {
                const view = editor.value.ctx.get(editorViewCtx)
                const { state } = view
                const { from, to } = state.selection
                
                if (from !== to) {
                  const textBefore = state.doc.textBetween(0, from, '\n')
                  const selectedTextFromDoc = state.doc.textBetween(from, to, '\n')
                  
                  // 如果从文档中获取的文本和选中的文本一致，使用文档中的
                  if (selectedTextFromDoc.trim().length > 0) {
                    selectedContent = selectedTextFromDoc.trim()
                  }

                  // 计算行号
                  const linesBefore = textBefore.split('\n').length
                  startLine = linesBefore
                  const selectedLines = selectedContent.split('\n').length
                  endLine = linesBefore + selectedLines - 1

                  console.log('[MarkdownEditor] 从 ProseMirror 获取选区:', { 
                    from, to, 
                    startLine, 
                    endLine, 
                    contentLen: selectedContent.length 
                  })
                }
              } catch (pmError) {
                console.warn('[MarkdownEditor] 无法从 ProseMirror 获取选区:', pmError)
              }
            }

            // 如果从 ProseMirror 获取失败，回退到字符串查找
            if (startLine === undefined && props.modelValue) {
              const cleanSelected = selectedContent.replace(/\s+/g, ' ').trim()
              const cleanMarkdown = props.modelValue.replace(/\s+/g, ' ')
              const startIndex = cleanMarkdown.indexOf(cleanSelected)
              
              if (startIndex !== -1) {
                const beforeStart = props.modelValue.substring(0, startIndex)
                startLine = (beforeStart.match(/\n/g) || []).length + 1
                const endIndex = startIndex + selectedContent.length
                const beforeEnd = props.modelValue.substring(0, endIndex)
                endLine = (beforeEnd.match(/\n/g) || []).length + 1
              }
            }

            // 保存选区信息
            savedSelection.value = { text: selectedContent, rect, startLine, endLine }

            // 更新全局 AI 上下文
            setAIContext({
              filePath: props.filePath,
              fileContent: props.modelValue,
              selectedText: selectedContent,
              startLine,
              endLine,
            })
            // 不再打开内部对话框，让事件冒泡到父组件处理
          }
        } else {
          // 选区消失，清除上下文
          savedSelection.value = null
          setAIContext({
            filePath: props.filePath,
            fileContent: props.modelValue,
            selectedText: undefined,
            startLine: undefined,
            endLine: undefined,
          })
        }
      } catch (e) {
        // 忽略错误
      }
    }, 10)
  }
  
  editor.value.ctx.get(editorViewCtx).dom.addEventListener('mouseup', handleEditorMouseUp)
  
  // Store for cleanup
  ;(editor.value as any)._pollInterval = pollInterval
  ;(editor.value as any)._updateBlockHandle = updateBlockHandle
  ;(editor.value as any)._handleEditorMouseUp = handleEditorMouseUp
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
  // 拖拽过程中不关闭工具条
  if (isDragging.value) return
  if (showToolbar.value) {
    collapseTimer = setTimeout(() => {
      if (!isDragging.value) {
        showToolbar.value = false
      }
    }, 300)
  }
}

// 自定义拖拽功能 - 使用 mouse 事件而非原生拖拽

// 拖拽状态
const dragTargetPos = ref<number | null>(null)
const dragInsertBefore = ref(true) // true = 在目标块上方插入，false = 在目标块下方插入

function handleDragMouseDown(e: MouseEvent) {
  e.preventDefault()
  e.stopPropagation()
  
  const sourcePos = blockHandle.activePos
  if (sourcePos === undefined || sourcePos === null) return
  
  isDragging.value = true
  dragSourcePos.value = sourcePos
  dragTargetPos.value = null
  
  // 添加全局 mouse 事件监听
  document.addEventListener('mousemove', handleDragMouseMove)
  document.addEventListener('mouseup', handleDragMouseUp)
}

function handleDragMouseMove(e: MouseEvent) {
  if (!isDragging.value || !editor.value || !editorRef.value) return
  
  try {
    const view = editor.value.ctx.get(editorViewCtx)
    const editorRect = editorRef.value.getBoundingClientRect()
    
    // 检查鼠标是否在编辑器区域内
    if (e.clientX < editorRect.left || e.clientX > editorRect.right ||
        e.clientY < editorRect.top || e.clientY > editorRect.bottom) {
      dropIndicator.visible = false
      dragTargetPos.value = null
      return
    }
    
    // 使用 ProseMirror 的 posAtCoords 获取位置
    const posAtCoords = view.posAtCoords({ left: e.clientX, top: e.clientY })
    
    if (posAtCoords) {
      const $pos = view.state.doc.resolve(posAtCoords.pos)
      
      // 找到块级位置
      let depth = $pos.depth
      let node = $pos.node(depth)
      
      while (depth > 0) {
        node = $pos.node(depth)
        if (node.isBlock) break
        depth--
      }
      
      const blockPos = $pos.before(depth)
      const dom = view.nodeDOM(blockPos)
      
      if (dom && dom instanceof HTMLElement) {
        const domRect = dom.getBoundingClientRect()
        
        // 根据鼠标在块的上半部分还是下半部分决定插入位置
        const midY = domRect.top + domRect.height / 2
        const insertBefore = e.clientY < midY
        
        dragTargetPos.value = blockPos
        dragInsertBefore.value = insertBefore
        
        dropIndicator.visible = true
        dropIndicator.left = domRect.left
        dropIndicator.width = domRect.width
        
        // 根据 insertBefore 决定指示器位置
        if (insertBefore) {
          dropIndicator.top = domRect.top - 1
        } else {
          dropIndicator.top = domRect.bottom - 1
        }
      }
    } else {
      dropIndicator.visible = false
      dragTargetPos.value = null
    }
  } catch (err) {
    console.error('[Drag] Move error:', err)
  }
}

function handleDragMouseUp(_e: MouseEvent) {
  // 移除事件监听
  document.removeEventListener('mousemove', handleDragMouseMove)
  document.removeEventListener('mouseup', handleDragMouseUp)
  
  // 重置拖拽状态
  dropIndicator.visible = false
  
  if (!isDragging.value || dragSourcePos.value === null || !editor.value) {
    isDragging.value = false
    dragSourcePos.value = null
    dragTargetPos.value = null
    return
  }
  
  const sourcePos = dragSourcePos.value
  const targetPos = dragTargetPos.value
  
  // 没有有效目标位置
  if (targetPos === null) {
    isDragging.value = false
    dragSourcePos.value = null
    dragTargetPos.value = null
    return
  }
  
  // 不允许拖到自己的位置
  if (targetPos === sourcePos) {
    isDragging.value = false
    dragSourcePos.value = null
    dragTargetPos.value = null
    return
  }
  
  try {
    const view = editor.value.ctx.get(editorViewCtx)
    const state = view.state
    const sourceNode = state.doc.nodeAt(sourcePos)
    
    if (!sourceNode) {
      isDragging.value = false
      dragSourcePos.value = null
      dragTargetPos.value = null
      return
    }
    
    // 创建事务
    let tr = state.tr
    const nodeSize = sourceNode.nodeSize
    
    // 计算实际插入位置
    let insertPos = targetPos
    if (!dragInsertBefore.value) {
      // 插入到目标块下方
      const targetNode = state.doc.nodeAt(targetPos)
      if (targetNode) {
        insertPos = targetPos + targetNode.nodeSize
      }
    }
    
    // 处理位置调整：如果源在目标之前，删除后目标位置会后移
    if (sourcePos < insertPos) {
      // 先删除源节点，再插入
      tr = tr.delete(sourcePos, sourcePos + nodeSize)
      // 删除后，插入位置需要调整
      insertPos -= nodeSize
      tr = tr.insert(insertPos, sourceNode)
    } else {
      // 源在目标之后，先插入再删除
      tr = tr.insert(insertPos, sourceNode)
      // 插入后，源位置需要调整
      tr = tr.delete(sourcePos + nodeSize, sourcePos + nodeSize + nodeSize)
    }
    
    view.dispatch(tr)
    
  } catch (err) {
    console.error('[Drag] Drop error:', err)
  }
  
  // 重置状态
  isDragging.value = false
  dragSourcePos.value = null
  dragTargetPos.value = null
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
  
  // 使用 insert 命令添加新段落
  executeCommand('InsertParagraph', {})
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

function turnInto(type: string) {
  if (!editorCtx.value) return
  
  try {
    const commands = editorCtx.value.get(commandsCtx)
    const view = editorCtx.value.get(editorViewCtx)
    const { $from } = view.state.selection
    
    switch (type) {
      case 'heading': {
        // 获取当前标题级别，默认为 1
        const currentNode = $from.node($from.depth)
        let level = 1
        if (currentNode.type.name === 'heading') {
          level = currentNode.attrs.level || 1
        }
        const heading = headingSchema.type(editorCtx.value)
        commands.call(setBlockTypeCommand.key, {
          nodeType: heading,
          attrs: { level }
        })
        break
      }
      case 'list': {
        // 切换列表类型或创建列表
        const bulletList = bulletListSchema.type(editorCtx.value)
        commands.call(wrapInBlockTypeCommand.key, { nodeType: bulletList })
        break
      }
      case 'paragraph': {
        const paragraph = paragraphSchema.type(editorCtx.value)
        commands.call(setBlockTypeCommand.key, { nodeType: paragraph })
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

function increaseLevel() {
  if (!editorCtx.value) return
  
  try {
    const view = editorCtx.value.get(editorViewCtx)
    const { $from } = view.state.selection
    const currentNode = $from.node($from.depth)
    
    // 标题：降低数字（提高级别，如 H2 -> H1）
    if (currentNode.type.name === 'heading') {
      const currentLevel = currentNode.attrs.level || 1
      if (currentLevel > 1) {
        const heading = headingSchema.type(editorCtx.value)
        const commands = editorCtx.value.get(commandsCtx)
        commands.call(setBlockTypeCommand.key, {
          nodeType: heading,
          attrs: { level: currentLevel - 1 }
        })
      }
    }
  } catch (e) {
    console.error('Increase level error:', e)
  }
  
  showToolbar.value = false
}

function decreaseLevel() {
  if (!editorCtx.value) return
  
  try {
    const view = editorCtx.value.get(editorViewCtx)
    const { $from } = view.state.selection
    const currentNode = $from.node($from.depth)
    
    // 标题：增加数字（降低级别，如 H1 -> H2）
    if (currentNode.type.name === 'heading') {
      const currentLevel = currentNode.attrs.level || 1
      if (currentLevel < 6) {
        const heading = headingSchema.type(editorCtx.value)
        const commands = editorCtx.value.get(commandsCtx)
        commands.call(setBlockTypeCommand.key, {
          nodeType: heading,
          attrs: { level: currentLevel + 1 }
        })
      }
    }
    // 普通段落：转为标题 6
    else if (currentNode.type.name === 'paragraph') {
      const heading = headingSchema.type(editorCtx.value)
      const commands = editorCtx.value.get(commandsCtx)
      commands.call(setBlockTypeCommand.key, {
        nodeType: heading,
        attrs: { level: 6 }
      })
    }
  } catch (e) {
    console.error('Decrease level error:', e)
  }
  
  showToolbar.value = false
}

// Update content when modelValue changes externally
watch(() => props.modelValue, async (newValue, _oldValue) => {
  if (!editor.value || newValue === undefined) return
  
  try {
    const view = editor.value.ctx.get(editorViewCtx)
    const { state } = view
    
    // Get current markdown content
    const parser = editor.value.ctx.get(parserCtx)
    const currentDoc = state.doc
    
    // Parse new markdown content
    const newDoc = await parser(newValue || '')
    
    // Only update if content is different
    if (newDoc && !newDoc.eq(currentDoc)) {
      view.dispatch(
        state.tr.replaceWith(0, currentDoc.content.size, newDoc.content)
      )
    }
  } catch (e) {
    console.warn('[MarkdownEditor] Failed to update content:', e)
  }
})

// Keyboard shortcuts
function handleKeydown(e: KeyboardEvent) {
  // Ctrl+S 保存
  if ((e.ctrlKey || e.metaKey) && e.key === 's') {
    e.preventDefault()
    e.stopPropagation()
    emit('request-save')
    return
  }

  // Ctrl+K 和 Escape 不处理,让父组件处理
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
      const mouseUpFn = (editor.value as any)._handleEditorMouseUp
      if (mouseUpFn) {
        view.dom.removeEventListener('mouseup', mouseUpFn)
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

  // 初始化时设置全局上下文
  setAIContext({
    filePath: props.filePath,
    fileContent: props.modelValue,
  })
})

onUnmounted(() => {
  destroyEditor()
  document.removeEventListener('keydown', handleKeydown)
  if (expandTimer) clearTimeout(expandTimer)
  if (collapseTimer) clearTimeout(collapseTimer)
})
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

.handle-toolbar {
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

/* 拖拽按钮样式 */
.drag-btn {
  cursor: grab;
  transition: all 0.15s ease;
}

.drag-btn:active,
.drag-btn.active {
  cursor: grabbing;
  background: rgba(35, 131, 226, 0.2) !important;
  color: #2383e2;
}

.drag-btn:hover:not(.active) {
  background: rgba(55, 53, 47, 0.12) !important;
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

/* Responsive */
@media (max-width: 768px) {
  .editor-root {
    padding: 16px;
  }
}
</style>