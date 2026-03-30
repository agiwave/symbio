<template>
  <div class="code-block-executor">
    <div class="code-header">
      <span class="language-badge">{{ language }}</span>
      <div class="actions">
        <button 
          v-if="executable" 
          class="run-btn" 
          @click="executeCode"
          :disabled="isExecuting"
        >
          {{ isExecuting ? '执行中...' : '▶ 运行' }}
        </button>
        <button class="copy-btn" @click="copyCode">📋 复制</button>
      </div>
    </div>
    
    <pre class="code-content"><code>{{ code }}</code></pre>
    
    <div v-if="result" class="execution-result" :class="{ error: !result.success }">
      <div class="result-header">
        <span>{{ result.success ? '✓ 执行成功' : '✗ 执行失败' }}</span>
        <span class="duration">{{ result.duration_ms }}ms</span>
      </div>
      <div v-if="result.stdout" class="stdout">
        <pre>{{ result.stdout }}</pre>
      </div>
      <div v-if="result.stderr" class="stderr">
        <pre>{{ result.stderr }}</pre>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed } from 'vue'
import { executeCodeBlock, isExecutableLanguage } from '../services/executor'

const props = defineProps<{
  code: string
  language: string
}>()

const isExecuting = ref(false)
const result = ref<{
  success: boolean
  stdout: string
  stderr: string
  duration_ms: number
} | null>(null)

const executable = computed(() => isExecutableLanguage(props.language))

async function executeCode() {
  if (isExecuting.value) return
  
  isExecuting.value = true
  result.value = null
  
  try {
    const res = await executeCodeBlock(props.code, props.language)
    result.value = {
      success: res.exit_code === 0 && !res.timed_out,
      stdout: res.stdout,
      stderr: res.stderr,
      duration_ms: res.duration_ms,
    }
  } catch (err) {
    result.value = {
      success: false,
      stdout: '',
      stderr: String(err),
      duration_ms: 0,
    }
  } finally {
    isExecuting.value = false
  }
}

function copyCode() {
  navigator.clipboard.writeText(props.code)
}
</script>

<style scoped>
.code-block-executor {
  margin: 1rem 0;
  border: 1px solid var(--color-border);
  border-radius: 8px;
  overflow: hidden;
}

.code-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.5rem 1rem;
  background: var(--color-surface);
  border-bottom: 1px solid var(--color-border);
}

.language-badge {
  font-size: 0.75rem;
  padding: 0.25rem 0.5rem;
  background: #6c757d;
  color: white;
  border-radius: 4px;
  text-transform: uppercase;
}

.actions {
  display: flex;
  gap: 0.5rem;
}

.run-btn, .copy-btn {
  padding: 0.25rem 0.75rem;
  border: none;
  border-radius: 4px;
  cursor: pointer;
  font-size: 0.75rem;
  transition: opacity 0.2s;
}

.run-btn {
  background: var(--color-primary);
  color: white;
}

.run-btn:hover:not(:disabled) {
  opacity: 0.9;
}

.run-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.copy-btn {
  background: transparent;
  border: 1px solid var(--color-border);
  color: var(--color-text-secondary);
}

.code-content {
  margin: 0;
  padding: 1rem;
  background: #1e1e1e;
  color: #d4d4d4;
  overflow-x: auto;
  font-family: 'Fira Code', 'Consolas', monospace;
  font-size: 0.875rem;
  line-height: 1.5;
}

.execution-result {
  border-top: 1px solid var(--color-border);
  padding: 0.75rem 1rem;
  background: #f8f9fa;
}

.execution-result.error {
  background: #fff5f5;
}

.result-header {
  display: flex;
  justify-content: space-between;
  margin-bottom: 0.5rem;
  font-size: 0.875rem;
  font-weight: 500;
}

.duration {
  color: var(--color-text-muted);
}

.stdout pre, .stderr pre {
  margin: 0;
  padding: 0.5rem;
  background: #1e1e1e;
  color: #d4d4d4;
  border-radius: 4px;
  font-family: 'Fira Code', 'Consolas', monospace;
  font-size: 0.75rem;
  white-space: pre-wrap;
  word-break: break-all;
}

.stderr pre {
  color: #f87171;
}
</style>
