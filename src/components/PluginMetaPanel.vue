<template>
  <div class="plugin-meta-panel">
    <h2 class="plugin-name">{{ meta?.name }}</h2>
    <p class="plugin-description">{{ meta?.description }}</p>

    <div class="meta-tags" v-if="hasInputSchema || hasOutputSchema">
      <span v-if="hasInputSchema" class="meta-tag tag-input">
        需要输入
      </span>
      <span v-if="hasOutputSchema" class="meta-tag tag-output">
        有输出
      </span>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import type { PluginMeta } from '../types'

const props = defineProps<{
  meta: PluginMeta | null
}>()

const hasInputSchema = computed(() => {
  return props.meta?.input &&
         typeof props.meta.input === 'object' &&
         Object.keys(props.meta.input.properties || {}).length > 0
})

const hasOutputSchema = computed(() => {
  return props.meta?.output &&
         typeof props.meta.output === 'object' &&
         Object.keys(props.meta.output.properties || {}).length > 0
})
</script>

<style scoped>
.plugin-meta-panel {
  background: white;
  padding: 1.5rem;
  border-radius: 8px;
  box-shadow: 0 2px 8px rgba(0,0,0,0.05);
}

.plugin-name {
  font-size: 1.5rem;
  color: #333;
  margin-bottom: 0.5rem;
}

.plugin-description {
  color: #666;
  line-height: 1.6;
  margin-bottom: 1rem;
}

.meta-tags {
  display: flex;
  gap: 0.5rem;
}

.meta-tag {
  display: inline-block;
  padding: 0.25rem 0.75rem;
  border-radius: 20px;
  font-size: 0.8rem;
  font-weight: 500;
}

.tag-input {
  background-color: #fff3cd;
  color: #856404;
}

.tag-output {
  background-color: #d4edda;
  color: #155724;
}
</style>
