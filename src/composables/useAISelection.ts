import { ref, reactive, computed, nextTick, type Ref } from 'vue'

/**
 * AI 选区交互 composable
 * 用于在编辑器中选择文字后弹出 AI 对话框
 */

export interface SelectionInfo {
  text: string
  rect: DOMRect
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

  // 保存的选区信息
  let savedSelection: SelectionInfo | null = null
  let debounceTimer: ReturnType<typeof setTimeout> | null = null

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
  function openForSelection(text: string, rect: DOMRect) {
    savedSelection = { text, rect }
    selectedText.value = text
    calculatePosition(rect)
    
    visible.value = true
    messages.value = []
    input.value = ''
  }

  // 更新选区内容（对话框已打开时）
  function updateSelection(text: string, rect: DOMRect) {
    savedSelection = { text, rect }
    selectedText.value = text
    calculatePosition(rect)
    // 保持对话框打开和消息历史
  }

  // 关闭对话框
  function close() {
    visible.value = false
    selectedText.value = ''
    savedSelection = null
    messages.value = []
  }

  // 通过快捷键/按钮打开（无选区）
  function open() {
    savedSelection = null
    selectedText.value = ''
    messages.value = []
    // 默认显示在右上角
    position.top = 80
    position.left = window.innerWidth - DIALOG_WIDTH - MARGIN
    visible.value = true
  }

  // 处理 mouseup 事件 - 选区检测
  function handleMouseUp(e: MouseEvent, containerEl?: HTMLElement) {
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
            
            if (visible.value) {
              // 对话框已打开，更新选区内容
              updateSelection(text, rect)
            } else {
              // 对话框未打开，打开它
              openForSelection(text, rect)
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
  function handleCtrlK(e: KeyboardEvent) {
    if ((e.ctrlKey || e.metaKey) && e.key === 'k') {
      e.preventDefault()
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

    // 方法
    openForSelection,
    updateSelection,
    close,
    open,
    handleMouseUp,
    handleEscape,
    handleCtrlK,
  }
}

export type AISelectionReturn = ReturnType<typeof useAISelection>
