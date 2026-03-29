<template>
  <div class="result-viewer">
    <h3 class="viewer-title">运行结果</h3>
    
    <div v-if="result?.error" class="result-error">
      <span class="error-icon">❌</span>
      <span>{{ result.error }}</span>
    </div>
    
    <div v-else-if="result" class="result-content">
      <!-- 对象类型结果 -->
      <div v-if="isObject(result)" class="result-object">
        <div 
          v-for="(value, key) in result" 
          :key="key"
          class="result-field"
        >
          <span class="field-key">{{ formatKey(key) }}</span>
          <span class="field-value">{{ formatValue(value) }}</span>
        </div>
      </div>
      
      <!-- 数组类型结果 -->
      <div v-else-if="Array.isArray(result)" class="result-array">
        <div v-for="(item, index) in result" :key="index" class="array-item">
          {{ formatValue(item) }}
        </div>
      </div>
      
      <!-- 简单类型结果 -->
      <div v-else class="result-simple">
        {{ formatValue(result) }}
      </div>
      
      <!-- 原始 JSON 视图 -->
      <details class="raw-json">
        <summary>查看原始 JSON</summary>
        <pre>{{ JSON.stringify(result, null, 2) }}</pre>
      </details>
    </div>
  </div>
</template>

<script setup lang="ts">
import type { JsonSchema } from '../types'

const props = defineProps<{
  result: any
  schema?: JsonSchema | null
}>()

const isObject = (value: any): boolean => {
  return value !== null && typeof value === 'object' && !Array.isArray(value)
}

const formatKey = (key: string): string => {
  return key.charAt(0).toUpperCase() + key.slice(1).replace(/_/g, ' ')
}

const formatValue = (value: any): string => {
  if (value === null || value === undefined) {
    return '—'
  }
  if (typeof value === 'boolean') {
    return value ? '✓' : '✗'
  }
  if (typeof value === 'number') {
    return String(value)
  }
  if (typeof value === 'string') {
    return value
  }
  if (Array.isArray(value)) {
    return value.map(v => formatValue(v)).join(', ')
  }
  if (typeof value === 'object') {
    return JSON.stringify(value)
  }
  return String(value)
}
</script>

<style scoped>
.result-viewer {
  background: white;
  padding: 1.5rem;
  border-radius: 8px;
  box-shadow: 0 2px 8px rgba(0,0,0,0.05);
}

.viewer-title {
  font-size: 1.1rem;
  color: #333;
  margin-bottom: 1rem;
  padding-bottom: 0.75rem;
  border-bottom: 2px solid #e0e0e0;
}

.result-error {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding: 1rem;
  background: #f8d7da;
  color: #721c24;
  border-radius: 6px;
}

.error-icon {
  font-size: 1.2rem;
}

.result-content {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.result-object {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
}

.result-field {
  display: flex;
  padding: 0.75rem;
  background: #f8f9fa;
  border-radius: 6px;
  border-left: 3px solid #667eea;
}

.field-key {
  font-weight: 600;
  color: #555;
  min-width: 150px;
  text-transform: capitalize;
}

.field-value {
  color: #333;
  flex: 1;
  word-break: break-word;
}

.result-array {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.array-item {
  padding: 0.75rem;
  background: #f8f9fa;
  border-radius: 6px;
  border-left: 3px solid #28a745;
}

.result-simple {
  padding: 1rem;
  background: #f8f9fa;
  border-radius: 6px;
  font-size: 1.1rem;
  color: #333;
}

.raw-json {
  margin-top: 1rem;
  border-top: 1px solid #e0e0e0;
  padding-top: 1rem;
}

.raw-json summary {
  cursor: pointer;
  color: #667eea;
  font-size: 0.9rem;
  user-select: none;
}

.raw-json pre {
  margin-top: 0.75rem;
  padding: 1rem;
  background: #2d2d2d;
  color: #f8f8f2;
  border-radius: 6px;
  overflow-x: auto;
  font-size: 0.85rem;
  line-height: 1.5;
}
</style>
