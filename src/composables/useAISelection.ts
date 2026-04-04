import { ref, reactive, computed, type Ref } from 'vue'
import { setAIContext } from './useAIContext'

/**
 * AI 选区交互 composable
 * 用于在编辑器中选择文字后弹出 AI 对话框
 */

export interface SelectionInfo {
  text: string
  rect: DOMRect
  // 行号信息（如果可用）
  startLine?: number
  endLine?: number
  // 文件路径
  filePath?: string
  // 完整文件内容（用于计算行号）
  fullContent?: string
}

export interface AISelectionState {
  visible: Ref<boolean>
  selectedText: Ref<string>
  position: {
    top: number
    left: number
  }
  messages: Ref<Array<{ role: 'user' | 'assistant'; content: string }>>
  input: Ref<string>
  loading: Ref<boolean>
}

export interface UseAISelectionOptions {
  /** 会话 ID，用于区分不同的 AI 会话 */
  sessionId: string
  /** 选区检测的延迟时间（毫秒） */
  debounceMs?: number
}

// 对话框尺寸常量
const DIALOG_WIDTH = 360
const DIALOG_MIN_HEIGHT = 200
const DIALOG_MAX_HEIGHT = 400
const MARGIN = 12

export function useAISelection(options: UseAISelectionOptions) {
  const { sessionId, debounceMs = 10 } = options

  // 状态
  const visible = ref(false)
  const selectedText = ref('')
  const position = reactive({
    top: 80,
    left: 0,
  })
  const messages = ref<Array<{ role: 'user' | 'assistant'; content: string }>>([])
  const input = ref('')
  const loading = ref(false)
  const dialogRef = ref<HTMLElement | null>(null)

  // 拖拽状态
  const isDragging = ref(false)
  const dragOffset = reactive({ x: 0, y: 0 })

  // 保存的选区信息（改为响应式 ref）
  const savedSelection = ref<SelectionInfo | null>(null)
  let debounceTimer: ReturnType<typeof setTimeout> | null = null
  
  // 开始拖拽
  function startDrag(e: MouseEvent) {
    isDragging.value = true
    dragOffset.x = e.clientX - position.left
    dragOffset.y = e.clientY - position.top
    
    document.addEventListener('mousemove', onDrag)
    document.addEventListener('mouseup', stopDrag)
  }
  
  // 拖拽中
  function onDrag(e: MouseEvent) {
    if (!isDragging.value) return
    
    const viewportWidth = window.innerWidth
    const viewportHeight = window.innerHeight
    
    // 计算新位置，限制在屏幕内
    let newLeft = e.clientX - dragOffset.x
    let newTop = e.clientY - dragOffset.y
    
    // 边界限制
    newLeft = Math.max(MARGIN, Math.min(newLeft, viewportWidth - DIALOG_WIDTH - MARGIN))
    newTop = Math.max(MARGIN, Math.min(newTop, viewportHeight - 100))
    
    position.left = newLeft
    position.top = newTop
  }
  
  // 停止拖拽
  function stopDrag() {
    isDragging.value = false
    document.removeEventListener('mousemove', onDrag)
    document.removeEventListener('mouseup', stopDrag)
  }

  // 计算对话框位置 - 跟随选区，避免超出屏幕
  function calculatePosition(rect: DOMRect) {
    const viewportWidth = window.innerWidth
    const viewportHeight = window.innerHeight
    
    // 估算对话框高度
    const estimatedHeight = DIALOG_MIN_HEIGHT + (selectedText.value.length > 50 ? 60 : 0)
    
    // 水平位置：优先显示在选区右侧，如果空间不够则显示在左侧
    let left = rect.right + MARGIN
    if (left + DIALOG_WIDTH > viewportWidth - MARGIN) {
      // 右侧空间不够，尝试显示在左侧
      left = rect.left - DIALOG_WIDTH - MARGIN
      if (left < MARGIN) {
        // 左侧也不够，显示在选区内或紧贴左侧
        left = Math.max(MARGIN, Math.min(rect.left, viewportWidth - DIALOG_WIDTH - MARGIN))
      }
    }
    
    // 垂直位置：优先显示在选区下方，如果空间不够则显示在上方
    let top = rect.bottom + MARGIN
    const spaceBelow = viewportHeight - rect.bottom - MARGIN
    const spaceAbove = rect.top - MARGIN
    
    if (spaceBelow < estimatedHeight && spaceAbove > spaceBelow) {
      // 下方空间不够且上方空间更大，显示在上方
      top = rect.top - estimatedHeight - MARGIN
      if (top < MARGIN) {
        top = MARGIN
      }
    } else if (top + estimatedHeight > viewportHeight - MARGIN) {
      // 下方空间不够，但上方也不够，尽量显示在屏幕内
      top = viewportHeight - estimatedHeight - MARGIN
      if (top < MARGIN) {
        top = MARGIN
      }
    }
    
    position.top = top
    position.left = left
  }

  // 打开对话框（用于选区触发）
  function openForSelection(text: string, rect: DOMRect, extra?: Partial<SelectionInfo>) {
    savedSelection.value = { text, rect, ...extra }
    selectedText.value = text
    calculatePosition(rect)

    visible.value = true
    messages.value = []
    input.value = ''

    // 更新全局 AI 上下文
    setAIContext({
      filePath: extra?.filePath,
      fileContent: extra?.fullContent,
      selectedText: text,
      startLine: extra?.startLine,
      endLine: extra?.endLine,
    })
  }

  // 更新选区内容（对话框已打开时）
  function updateSelection(text: string, rect: DOMRect, extra?: Partial<SelectionInfo>) {
    savedSelection.value = { text, rect, ...extra }
    selectedText.value = text
    calculatePosition(rect)
    // 保持对话框打开和消息历史

    // 更新全局 AI 上下文
    setAIContext({
      filePath: extra?.filePath,
      fileContent: extra?.fullContent,
      selectedText: text,
      startLine: extra?.startLine,
      endLine: extra?.endLine,
    })
  }

  // 关闭对话框
  function close() {
    visible.value = false
    selectedText.value = ''
    savedSelection.value = null
    messages.value = []
    // 重置全局 AI 上下文
    setAIContext({
      selectedText: undefined,
      startLine: undefined,
      endLine: undefined,
    })
  }

  // 通过快捷键/按钮打开（无选区）
  function open() {
    savedSelection.value = null
    selectedText.value = ''
    messages.value = []
    // 默认显示在右上角
    position.top = 80
    position.left = window.innerWidth - DIALOG_WIDTH - MARGIN
    visible.value = true
  }

  // 处理 mouseup 事件 - 选区检测
  function handleMouseUp(e: MouseEvent, containerEl?: HTMLElement, context?: { filePath?: string; fullContent?: string }) {
    // 如果正在加载，不处理
    if (loading.value) return

    // 清除之前的定时器
    if (debounceTimer) clearTimeout(debounceTimer)

    debounceTimer = setTimeout(() => {
      try {
        const selection = window.getSelection()

        if (selection && !selection.isCollapsed) {
          const text = selection.toString().trim()
          if (text.length > 0) {
            const range = selection.getRangeAt(0)
            const rect = range.getBoundingClientRect()

            // 计算行号（如果有完整内容）
            let startLine: number | undefined
            let endLine: number | undefined
            
            if (context?.fullContent && context?.filePath) {
              const lines = context.fullContent.split('\n')
              const selectedText = text
              
              // 在完整内容中查找选中内容的起始位置
              const startIndex = context.fullContent.indexOf(selectedText)
              if (startIndex !== -1) {
                // 计算起始行号
                const beforeStart = context.fullContent.substring(0, startIndex)
                startLine = (beforeStart.match(/\n/g) || []).length + 1
                
                // 计算结束行号
                const endIndex = startIndex + selectedText.length
                const beforeEnd = context.fullContent.substring(0, endIndex)
                endLine = (beforeEnd.match(/\n/g) || []).length + 1
              }
            }

            const extraInfo = {
              filePath: context?.filePath,
              fullContent: context?.fullContent,
              startLine,
              endLine
            }

            if (visible.value) {
              // 对话框已打开，更新选区内容
              updateSelection(text, rect, extraInfo)
            } else {
              // 对话框未打开，打开它
              openForSelection(text, rect, extraInfo)
            }
          }
        } else if (visible.value) {
          // 选区消失且对话框已打开，关闭对话框
          close()
        }
      } catch (e) {
        // 忽略错误
      }
    }, debounceMs)
  }

  // 处理 Escape 键
  function handleEscape(e: KeyboardEvent) {
    if (e.key === 'Escape' && visible.value) {
      e.preventDefault()
      close()
      return true
    }
    return false
  }

  // 处理 Ctrl+K 快捷键
  function handleCtrlK(e: KeyboardEvent, context?: { filePath?: string; fileContent?: string }) {
    if ((e.ctrlKey || e.metaKey) && e.key === 'k') {
      e.preventDefault()
      
      // 如果有文件上下文,先设置 AI 上下文
      if (context?.filePath) {
        setAIContext({
          filePath: context.filePath,
          fileContent: context.fileContent,
          selectedText: undefined,
          startLine: undefined,
          endLine: undefined,
        })
      }
      
      open()
      return true
    }
    return false
  }

  // 对话框样式
  const dialogStyle = computed(() => ({
    top: `${position.top}px`,
    left: `${position.left}px`,
  }))

  return {
    // 状态
    visible,
    selectedText,
    position,
    messages,
    input,
    loading,
    dialogRef,
    dialogStyle,
    sessionId,
    savedSelection,
    isDragging,

    // 方法
    openForSelection,
    updateSelection,
    close,
    open,
    handleMouseUp,
    handleEscape,
    handleCtrlK,
    startDrag,
  }
}

export type AISelectionReturn = ReturnType<typeof useAISelection>
