<template>
  <div class="diagnostic">
    <h2>诊断测试</h2>
    
    <div class="test-section">
      <h3>测试 1: 调用 invoke 命令</h3>
      <button @click="testInvoke" :disabled="testing">
        {{ testing ? '测试中...' : '运行测试' }}
      </button>
      <pre>{{ invokeResult }}</pre>
    </div>

    <div class="test-section">
      <h3>测试 2: 调用 meta 命令</h3>
      <button @click="testMeta" :disabled="testing">
        {{ testing ? '测试中...' : '运行测试' }}
      </button>
      <pre>{{ metaResult }}</pre>
    </div>

    <div class="test-section">
      <h3>测试 3: 插件列表</h3>
      <pre>{{ plugins }}</pre>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref } from 'vue'
import { invoke } from '@tauri-apps/api/core'

const testing = ref(false)
const invokeResult = ref<any>(null)
const metaResult = ref<any>(null)
const plugins = ref<any[]>([])
const error = ref<string | null>(null)

const testInvoke = async () => {
  testing.value = true
  error.value = null
  try {
    invokeResult.value = await invoke('invoke', { path: [], input: {} })
    const result = invokeResult.value as any
    if (result.plugins) {
      plugins.value = result.plugins
    }
  } catch (e) {
    error.value = String(e)
    invokeResult.value = { error: String(e) }
  } finally {
    testing.value = false
  }
}

const testMeta = async () => {
  testing.value = true
  error.value = null
  try {
    metaResult.value = await invoke('meta', { path: ['echo'] })
  } catch (e) {
    error.value = String(e)
    metaResult.value = { error: String(e) }
  } finally {
    testing.value = false
  }
}
</script>

<style scoped>
.diagnostic {
  padding: 2rem;
  font-family: monospace;
}

.test-section {
  margin: 1rem 0;
  padding: 1rem;
  background: #f5f5f5;
  border-radius: 8px;
}

.test-section h3 {
  margin-bottom: 0.5rem;
}

button {
  padding: 0.5rem 1rem;
  background: #667eea;
  color: white;
  border: none;
  border-radius: 4px;
  cursor: pointer;
  margin-bottom: 1rem;
}

button:disabled {
  opacity: 0.5;
}

pre {
  background: #fff;
  padding: 1rem;
  border-radius: 4px;
  overflow-x: auto;
  white-space: pre-wrap;
}
</style>
