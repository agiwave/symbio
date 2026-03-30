<template>
  <div class="markdown-editor-container">
    <div ref="editorRef" class="editor-root"></div>
    
    <!-- 执行结果显示 -->
    <div v-if="executionResult" class="execution-result">
      <div class="result-header">
        <span :class="['status', executionResult.status]">
          {{ executionResult.status === 'success' ? '✅ 成功' : '❌ 失败' }}
        </span>
        <button class="close-btn" @click="executionResult = null">×</button>
      </div>
      <pre class="result-output">{{ executionResult.output }}</pre>
      <div v-if="executionResult.error" class="result-error">
        {{ executionResult.error }}
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted, onUnmounted, watch } from 'vue'
import { EditorView } from '@codemirror/view'
import { EditorState } from '@codemirror/state'
import { 
  extractCodeBlocks, 
  createEditorExtensions, 
  type CodeBlock 
} from '../composables/useMarkdownEditor'

const props = defineProps<{
  modelValue: string
  theme?: 'light' | 'dark'
}>()

const emit = defineEmits<{
  'update:modelValue': [value: string]
}>()

const editorRef = ref<HTMLElement | null>(null)
const editorView = ref<EditorView | null>(null)
const codeBlocks = ref<CodeBlock[]>([])
const executing = ref<string | null>(null)
const executionResult = ref<{
  status: 'success' | 'failed'
  output: string
  error?: string
} | null>(null)

function createEditor() {
  if (!editorRef.value) return

  const updateListener = EditorView.updateListener.of((update) => {
    if (update.docChanged) {
      const content = update.state.doc.toString()
      emit('update:modelValue', content)
      codeBlocks.value = extractCodeBlocks(content)
    }
  })

  const extensions = [
    ...createEditorExtensions(props.theme),
    updateListener,
    EditorView.lineWrapping,
  ]

  const state = EditorState.create({
    doc: props.modelValue,
    extensions,
  })

  editorView.value = new EditorView({
    state,
    parent: editorRef.value,
  })

  // 初始提取代码块
  codeBlocks.value = extractCodeBlocks(props.modelValue)
}

function destroyEditor() {
  if (editorView.value) {
    editorView.value.destroy()
    editorView.value = null
  }
}

// 监听外部值变化
watch(() => props.modelValue, (newValue) => {
  if (editorView.value) {
    const currentValue = editorView.value.state.doc.toString()
    if (newValue !== currentValue) {
      editorView.value.dispatch({
        changes: {
          from: 0,
          to: currentValue.length,
          insert: newValue,
        },
      })
    }
  }
})

onMounted(() => {
  createEditor()
})

onUnmounted(() => {
  destroyEditor()
})
</script>

<style scoped>
.markdown-editor-container {
  position: relative;
  height: 100%;
  display: flex;
  flex-direction: column;
}

.editor-root {
  flex: 1;
  overflow: auto;
  font-family: 'Monaco', 'Menlo', 'Consolas', monospace;
  font-size: 14px;
  line-height: 1.6;
}

.editor-root :deep(.cm-editor) {
  height: 100%;
}

.editor-root :deep(.cm-scroller) {
  font-family: inherit;
}

.editor-root :deep(.cm-content) {
  padding: 16px;
}

.editor-root :deep(.cm-line) {
  padding: 0 16px;
}

/* 执行结果 */
.execution-result {
  margin-top: 16px;
  border: 1px solid var(--color-border);
  border-radius: 8px;
  overflow: hidden;
  background: #fff;
}

.result-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 8px 12px;
  background: #f8f9fa;
  border-bottom: 1px solid var(--color-border);
}

.status {
  font-size: 13px;
  font-weight: 500;
}

.status.success {
  color: #28a745;
}

.status.failed {
  color: #dc3545;
}

.close-btn {
  width: 24px;
  height: 24px;
  border: none;
  background: transparent;
  cursor: pointer;
  font-size: 18px;
  color: #666;
}

.close-btn:hover {
  color: #333;
}

.result-output {
  padding: 12px;
  margin: 0;
  font-family: monospace;
  font-size: 13px;
  white-space: pre-wrap;
  word-break: break-all;
  max-height: 300px;
  overflow: auto;
}

.result-error {
  padding: 8px 12px;
  background: #fff5f5;
  color: #dc3545;
  font-size: 13px;
  border-top: 1px solid var(--color-border);
}
</style>