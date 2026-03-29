<template>
  <div class="plugin-browser">
    <div class="browser-header">
      <h3>插件列表</h3>
    </div>

    <div class="plugin-tree">
      <template v-if="loading">
        <div class="loading">加载中...</div>
      </template>
      <template v-else-if="error">
        <div class="error">{{ error }}</div>
      </template>
      <template v-else>
        <div
          v-for="plugin in plugins"
          :key="plugin.name"
          class="plugin-item"
          :class="{ selected: isSelected(plugin.name) }"
          @click="handleSelect([plugin.name])"
        >
          <span class="plugin-icon">📄</span>
          <span class="plugin-name">{{ plugin.name }}</span>
        </div>
      </template>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, onMounted } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import type { PluginMeta } from '../types'

interface PluginInfo {
  name: string
  meta: PluginMeta
}

const props = defineProps<{
  modelValue?: string[]
}>()

const emit = defineEmits<{
  'update:modelValue': [path: string[]]
  'select': [path: string[]]
}>()

const plugins = ref<PluginInfo[]>([])
const loading = ref(true)
const error = ref<string | null>(null)

const isSelected = (name: string) => {
  if (!props.modelValue || props.modelValue.length === 0) return false
  return props.modelValue.length === 1 && props.modelValue[0] === name
}

const handleSelect = (path: string[]) => {
  emit('update:modelValue', path)
  emit('select', path)
}

onMounted(async () => {
  try {
    // 调用 agent 的 invoke 方法获取插件列表
    const result = await invoke<any>('invoke', {
      path: [],
      input: {}
    })
    
    // result.plugins 是插件名称数组
    const pluginNames: string[] = result.plugins || []
    
    // 获取每个插件的元数据
    for (const name of pluginNames) {
      try {
        const metaResponse = await invoke<any>('meta', { path: [name] })
        plugins.value.push({
          name,
          meta: metaResponse.meta
        })
      } catch (e) {
        // 插件元数据获取失败，跳过
        console.log(`获取插件 ${name} 元数据失败:`, e)
      }
    }
  } catch (err) {
    error.value = String(err)
    console.error('加载插件列表失败:', err)
  } finally {
    loading.value = false
  }
})
</script>

<style scoped>
.plugin-browser {
  height: 100%;
  display: flex;
  flex-direction: column;
}

.browser-header {
  padding: 1rem;
  border-bottom: 1px solid #e0e0e0;
  background: #f9f9f9;
}

.browser-header h3 {
  font-size: 1rem;
  color: #555;
}

.plugin-tree {
  padding: 0.5rem;
  overflow-y: auto;
  flex: 1;
}

.loading, .error {
  padding: 1rem;
  text-align: center;
  color: #999;
}

.error {
  color: #dc3545;
}

.plugin-item {
  display: flex;
  align-items: center;
  padding: 0.5rem 0.75rem;
  border-radius: 6px;
  cursor: pointer;
  transition: background-color 0.2s;
  margin: 0.25rem 0;
}

.plugin-item:hover {
  background-color: #f0f0f0;
}

.plugin-item.selected {
  background-color: #e3e7fc;
  color: #667eea;
  font-weight: 500;
}

.plugin-icon {
  margin-right: 0.5rem;
  font-size: 1.1rem;
}

.plugin-name {
  flex: 1;
  font-size: 0.9rem;
}
</style>
