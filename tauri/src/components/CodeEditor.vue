<template>
  <div class="code-editor" ref="containerRef">
    <!-- 行号区域 -->
    <div class="line-numbers" ref="lineNumbersRef" @scroll="syncScroll">
      <div
        v-for="line in lineCount"
        :key="line"
        class="line-number"
      >
        {{ line }}
      </div>
    </div>
    <!-- 编辑区域 -->
    <textarea
      ref="textareaRef"
      v-model="content"
      class="code-textarea"
      :placeholder="'输入内容...'"
      :readonly="readonly"
      @input="onInput"
      @keydown="handleKeydown"
      @keyup="handleKeyUp"
      @mouseup="handleMouseUp"
      @select="handleSelect"
      @scroll="syncScroll"
      spellcheck="false"
    ></textarea>
  </div>
</template>

<script setup lang="ts">
import { ref, watch, computed } from 'vue'

const props = defineProps<{
  modelValue: string
  filePath?: string
  /** 只读：禁用编辑，但仍允许选区事件 */
  readonly?: boolean
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string]
  'content-change': [value: string]
  'request-save': []
  'selection-change': [data: { text: string; startLine: number; endLine: number } | null]
}>()

const textareaRef = ref<HTMLTextAreaElement | null>(null)
const containerRef = ref<HTMLElement | null>(null)
const lineNumbersRef = ref<HTMLElement | null>(null)
const content = ref(props.modelValue)

// 计算行数
const lineCount = computed(() => {
  return content.value.split('\n').length
})

// 监听外部值变化
watch(() => props.modelValue, (val) => {
  content.value = val
})

// 同步滚动
function syncScroll() {
  if (textareaRef.value && lineNumbersRef.value) {
    lineNumbersRef.value.scrollTop = textareaRef.value.scrollTop
  }
}

function onInput() {
  emit('update:modelValue', content.value)
  emit('content-change', content.value)
}

function handleKeydown(e: KeyboardEvent) {
  // 只读模式下不响应保存 / Tab 编辑快捷键
  if (props.readonly) {
    // Ctrl/Cmd+A 仍然允许（浏览器默认行为）
    return
  }

  // Ctrl+S / Cmd+S 保存
  if ((e.ctrlKey || e.metaKey) && e.key === 's') {
    e.preventDefault()
    emit('request-save')
  }

  // Tab 键支持
  if (e.key === 'Tab') {
    e.preventDefault()
    const textarea = textareaRef.value
    if (textarea) {
      const start = textarea.selectionStart
      const end = textarea.selectionEnd
      const value = textarea.value

      textarea.value = value.substring(0, start) + '  ' + value.substring(end)
      textarea.selectionStart = textarea.selectionEnd = start + 2

      onInput()
    }
  }
}

// 键盘释放时也检查选区（Shift+方向键 选中）
function handleKeyUp(_e: KeyboardEvent) {
  checkSelection()
}

// 原生 select 事件（覆盖键盘 + 拖动等多种选中方式）
function handleSelect() {
  checkSelection()
}

// 处理选区事件
function handleMouseUp(e: MouseEvent) {
  e.stopPropagation()
  checkSelection()
}

// 检查选区并通知父组件
function checkSelection() {
  setTimeout(() => {
    try {
      const textarea = textareaRef.value
      if (!textarea) return

      const start = textarea.selectionStart
      const end = textarea.selectionEnd

      if (start !== end) {
        const selectedText = textarea.value.substring(start, end)

        if (selectedText.trim().length > 0) {
          const textBefore = textarea.value.substring(0, start)
          const linesBefore = textBefore.split('\n').length
          const selectedLines = selectedText.split('\n').length
          const endLine = linesBefore + selectedLines - 1

          emit('selection-change', {
            text: selectedText.trim(),
            startLine: linesBefore,
            endLine: endLine
          })
          return
        }
      }

      emit('selection-change', null)
    } catch (e) {
      // 忽略错误
    }
  }, 10)
}

defineExpose({
  textarea: textareaRef,
  container: containerRef
})
</script>

<style scoped>
.code-editor {
  width: 100%;
  height: 100%;
  display: flex;
  flex-direction: row;
  overflow: hidden;
}

.line-numbers {
  width: 48px;
  min-width: 48px;
  background: #2d2d2d;
  color: #858585;
  font-family: 'Fira Code', 'Consolas', 'Monaco', monospace;
  font-size: 0.875rem;
  line-height: 1.6;
  padding: 1rem 0.5rem;
  text-align: right;
  overflow: hidden;
  user-select: none;
  border-right: 1px solid #3c3c3c;
}

.line-number {
  height: calc(0.875rem * 1.6);
}

.code-textarea {
  flex: 1;
  width: 100%;
  padding: 1rem;
  background: #1e1e1e;
  color: #d4d4d4;
  border: none;
  outline: none;
  font-family: 'Fira Code', 'Consolas', 'Monaco', monospace;
  font-size: 0.875rem;
  line-height: 1.6;
  resize: none;
  tab-size: 2;
  white-space: pre;
  overflow: auto;
}

.code-textarea::placeholder {
  color: #666;
}

.code-textarea:focus {
  background: #1a1a1a;
}
</style>
