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
  /** 只读：禁用编辑 */
  readonly?: boolean
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string]
  'content-change': [value: string]
  'request-save': []
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
  width: 3rem;
  min-width: 3rem;
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
