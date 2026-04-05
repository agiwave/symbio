<template>
  <div class="dynamic-form">
    <form @submit.prevent="handleSubmit" class="form-content">
      <FormField 
        v-for="(propSchema, key) in schema.properties"
        :key="key"
        :name="key"
        :schema="propSchema"
        :required="isRequired(key)"
        :default-value="propSchema.default"
        v-model="formData[key]"
      />
      
      <div class="form-actions">
        <button type="submit" :disabled="loading">
          {{ loading ? '提交中...' : '提交' }}
        </button>
      </div>
    </form>
  </div>
</template>

<script setup lang="ts">
import { reactive, watch } from 'vue'
import FormField from './FormField.vue'
import type { JsonSchema } from '../types'

const props = defineProps<{
  schema: JsonSchema
  loading?: boolean
}>()

const emit = defineEmits<{
  'submit': [data: any]
}>()

const formData = reactive<Record<string, any>>({})

// 初始化默认值
const initDefaults = () => {
  if (props.schema.properties) {
    Object.entries(props.schema.properties).forEach(([key, prop]) => {
      if (prop.default !== undefined) {
        formData[key] = prop.default
      } else if (prop.type === 'boolean') {
        formData[key] = false
      } else if (prop.type === 'array') {
        formData[key] = []
      } else {
        formData[key] = undefined
      }
    })
  }
}

watch(() => props.schema, initDefaults, { immediate: true })

const isRequired = (key: string): boolean => {
  return props.schema.required?.includes(key) || false
}

const handleSubmit = () => {
  emit('submit', { ...formData })
}
</script>

<style scoped>
.dynamic-form {
  background: white;
  padding: 1.5rem;
  border-radius: 8px;
  box-shadow: 0 2px 8px rgba(0,0,0,0.05);
  margin-bottom: 1.5rem;
}

.form-content {
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.form-actions {
  margin-top: 1rem;
  padding-top: 1rem;
  border-top: 1px solid #e0e0e0;
}

.form-actions button {
  padding: 0.75rem 2rem;
  background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
  color: white;
  border: none;
  border-radius: 6px;
  cursor: pointer;
  font-size: 1rem;
  font-weight: 500;
  transition: transform 0.2s, opacity 0.2s;
}

.form-actions button:hover:not(:disabled) {
  transform: translateY(-2px);
}

.form-actions button:disabled {
  opacity: 0.6;
  cursor: not-allowed;
  transform: none;
}
</style>
