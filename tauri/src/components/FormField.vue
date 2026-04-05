<template>
  <div class="form-field">
    <label class="field-label">
      <span class="field-name">{{ displayName }}</span>
      <span v-if="required" class="required-mark">*</span>
    </label>
    
    <p v-if="schema.description" class="field-description">
      {{ schema.description }}
    </p>
    
    <!-- 字符串输入 -->
    <input 
      v-if="schema.type === 'string' && !schema.enum_values"
      type="text"
      :value="modelValue"
      @input="handleInput"
      class="field-input"
      :placeholder="schema.default"
    />
    
    <!-- 枚举选择 -->
    <select 
      v-if="schema.type === 'string' && schema.enum_values"
      :value="modelValue"
      @change="handleChange"
      class="field-select"
    >
      <option v-for="opt in schema.enum_values" :key="String(opt)" :value="opt">
        {{ opt }}
      </option>
    </select>
    
    <!-- 数字输入 -->
    <input 
      v-if="schema.type === 'number'"
      type="number"
      step="any"
      :value="modelValue"
      @input="handleNumberInput"
      class="field-input"
      :placeholder="String(schema.default ?? 0)"
    />
    
    <!-- 整数输入 -->
    <input 
      v-if="schema.type === 'integer'"
      type="number"
      :value="modelValue"
      @input="handleNumberInput"
      class="field-input"
      :placeholder="String(schema.default ?? 0)"
    />
    
    <!-- 布尔值 -->
    <label v-if="schema.type === 'boolean'" class="field-checkbox">
      <input 
        type="checkbox"
        :checked="modelValue"
        @change="handleCheckbox"
      />
      <span>{{ modelValue ? '是' : '否' }}</span>
    </label>
    
    <!-- 数组（简单文本输入） -->
    <textarea 
      v-if="schema.type === 'array'"
      :value="modelValue"
      @input="handleInput"
      class="field-textarea"
      placeholder="每行一个值..."
      rows="3"
    />
  </div>
</template>

<script setup lang="ts">
import type { SchemaProperty } from '../types'

interface Props {
  name: string
  schema: SchemaProperty
  required: boolean
  defaultValue?: any
  modelValue?: any
}

const props = defineProps<Props>()
const emit = defineEmits<{
  'update:modelValue': [value: any]
}>()

const displayName = props.name.charAt(0).toUpperCase() + props.name.slice(1)

const handleInput = (event: Event) => {
  const target = event.target as HTMLInputElement | HTMLTextAreaElement
  emit('update:modelValue', target.value)
}

const handleNumberInput = (event: Event) => {
  const target = event.target as HTMLInputElement
  const value = target.value === '' ? undefined : parseFloat(target.value)
  emit('update:modelValue', value)
}

const handleChange = (event: Event) => {
  const target = event.target as HTMLSelectElement
  emit('update:modelValue', target.value)
}

const handleCheckbox = (event: Event) => {
  const target = event.target as HTMLInputElement
  emit('update:modelValue', target.checked)
}
</script>

<style scoped>
.form-field {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.field-label {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-weight: 500;
  color: #444;
}

.field-name {
  text-transform: capitalize;
}

.required-mark {
  color: #dc3545;
}

.field-description {
  font-size: 0.85rem;
  color: #888;
  margin: -0.25rem 0 0 0;
}

.field-input,
.field-select,
.field-textarea {
  padding: 0.75rem;
  border: 1px solid #ddd;
  border-radius: 6px;
  font-size: 0.95rem;
  transition: border-color 0.2s, box-shadow 0.2s;
}

.field-input:focus,
.field-select:focus,
.field-textarea:focus {
  outline: none;
  border-color: #667eea;
  box-shadow: 0 0 0 3px rgba(102, 126, 234, 0.1);
}

.field-select {
  background: white;
  cursor: pointer;
}

.field-checkbox {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  cursor: pointer;
  user-select: none;
}

.field-checkbox input[type="checkbox"] {
  width: 1.2rem;
  height: 1.2rem;
  cursor: pointer;
}
</style>
