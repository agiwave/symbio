<template>
  <div class="code-editor">
    <textarea
      ref="textareaRef"
      v-model="content"
      class="code-textarea"
      :placeholder="'输入内容...'"
      @input="onInput"
      @keydown="handleKeydown"
      spellcheck="false"
    ></textarea>
  </div>
</template>

<script setup lang="ts">
import { ref, watch } from 'vue'

const props = defineProps<{
  modelValue: string
  filePath?: string
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string]
  'content-change': [value: string]
  'request-save': []
}>()

const textareaRef = ref<HTMLTextAreaElement | null>(null)
const content = ref(props.modelValue)

// 监听外部值变化
watch(() => props.modelValue, (val) => {
  content.value = val
})

function onInput() {
  emit('update:modelValue', content.value)
  emit('content-change', content.value)
}

function handleKeydown(e: KeyboardEvent) {
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
</script>

<style scoped>
.code-editor {
  width: 100%;
  height: 100%;
  display: flex;
  flex-direction: column;
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
