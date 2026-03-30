<template>
  <div class="markdown-editor-container">
    <!-- 工具栏 -->
    <div class="editor-toolbar">
      <div class="toolbar-left">
        <button 
          v-for="block in executableBlocks" 
          :key="block.id"
          class="run-block-btn"
          @click="executeBlock(block)"
          :disabled="isExecuting"
        >
          ▶ {{ block.language }} ({{ block.code.slice(0, 20) }}...)
        </button>
      </div>
      <div class="toolbar-right">
        <span v-if="executableBlocks.length === 0" class="hint">
          添加可执行代码块 (python/r/bash)
        </span>
      </div>
    </div>
    
    <!-- 编辑器 -->
    <div ref="editorRef" class="editor-root"></div>
    
    <!-- 执行结果 -->
    <div v-if="executionResult" class="execution-result">
      <div class="result-header">
        <span :class="['status', executionResult.status]">
          {{ executionResult.status === 'success' ? '✓ 成功' : '✗ 失败' }}
        </span>
        <span class="duration">{{ executionResult.duration_ms }}ms</span>
        <button class="close-btn" @click="executionResult = null">×</button>
      </div>
      <pre v-if="executionResult.stdout" class="result-output">{{ executionResult.stdout }}</pre>
      <pre v-if="executionResult.stderr" class="result-error">{{ executionResult.stderr }}</pre>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import { EditorView } from '@codemirror/view'
import { EditorState } from '@codemirror/state'
import { 
  extractCodeBlocks, 
  createEditorExtensions, 
  isExecutableLanguage,
  type CodeBlock 
} from '../composables/useMarkdownEditor'
import { executeCodeBlock, type ExecutionResult } from '../services/executor'

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
const isExecuting = ref(false)
const executionResult = ref<{
  status: 'success' | 'failed'
  stdout: string
  stderr: string
  duration_ms: number
} | null>(null)

const executableBlocks = computed(() => 
  codeBlocks.value.filter(block => isExecutableLanguage(block.language))
)

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

  codeBlocks.value = extractCodeBlocks(props.modelValue)
}

function destroyEditor() {
  if (editorView.value) {
    editorView.value.destroy()
    editorView.value = null
  }
}

async function executeBlock(block: CodeBlock) {
  if (isExecuting.value) return
  
  isExecuting.value = true
  executionResult.value = null
  
  try {
    const result = await executeCodeBlock(block.code, block.language)
    executionResult.value = {
      status: result.exit_code === 0 && !result.timed_out ? 'success' : 'failed',
      stdout: result.stdout,
      stderr: result.stderr,
      duration_ms: result.duration_ms,
    }
  } catch (err) {
    executionResult.value = {
      status: 'failed',
      stdout: '',
      stderr: String(err),
      duration_ms: 0,
    }
  } finally {
    isExecuting.value = false
  }
}

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

.editor-toolbar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.5rem 1rem;
  background: var(--color-surface);
  border-bottom: 1px solid var(--color-border);
  min-height: 40px;
}

.toolbar-left {
  display: flex;
  gap: 0.5rem;
  flex-wrap: wrap;
}

.run-block-btn {
  padding: 0.25rem 0.75rem;
  border: none;
  background: var(--color-primary);
  color: white;
  border-radius: 4px;
  cursor: pointer;
  font-size: 0.75rem;
  transition: opacity 0.2s;
}

.run-block-btn:hover:not(:disabled) {
  opacity: 0.9;
}

.run-block-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.hint {
  font-size: 0.75rem;
  color: var(--color-text-muted);
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

.execution-result {
  margin-top: 0;
  border-top: 1px solid var(--color-border);
  background: var(--color-surface);
  max-height: 300px;
  overflow: auto;
}

.result-header {
  display: flex;
  align-items: center;
  gap: 1rem;
  padding: 0.5rem 1rem;
  background: #f8f9fa;
  border-bottom: 1px solid var(--color-border);
}

.status {
  font-size: 0.875rem;
  font-weight: 500;
}

.status.success {
  color: #28a745;
}

.status.failed {
  color: #dc3545;
}

.duration {
  font-size: 0.75rem;
  color: var(--color-text-muted);
}

.close-btn {
  margin-left: auto;
  width: 24px;
  height: 24px;
  border: none;
  background: transparent;
  cursor: pointer;
  font-size: 18px;
  color: #666;
}

.result-output, .result-error {
  padding: 0.75rem 1rem;
  margin: 0;
  font-family: 'Fira Code', 'Consolas', monospace;
  font-size: 0.75rem;
  white-space: pre-wrap;
  word-break: break-all;
}

.result-output {
  background: #1e1e1e;
  color: #d4d4d4;
}

.result-error {
  background: #1e1e1e;
  color: #f87171;
  border-top: 1px solid var(--color-border);
}
</style>
