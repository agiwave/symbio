<template>
  <Teleport to="body">
    <Transition name="fade">
      <div 
        v-if="visible"
        class="floating-input-overlay"
        @click.self="$emit('close')"
      >
        <div 
          class="floating-input-container"
          :style="positionStyle"
        >
          <div class="input-header">
            <span class="input-title">🤖 Model \u52a9\u624b</span>
            <button class="close-btn" @click="$emit('close')">×</button>
          </div>
          
          <div v-if="context" class="context-preview">
            <span class="context-label">上下文:</span>
            <span class="context-text">{{ contextPreview }}</span>
          </div>
          
          <textarea
            ref="inputRef"
            v-model="inputText"
            class="input-field"
            :placeholder="placeholder"
            @keydown.enter.exact.prevent="submit"
            @keydown.esc="$emit('close')"
          ></textarea>
          
          <div class="input-actions">
            <span class="shortcut-hint">Enter 发送 · Esc 关闭</span>
            <button 
              class="submit-btn" 
              @click="submit"
              :disabled="!inputText.trim()"
            >
              发送
            </button>
          </div>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<script setup lang="ts">
import { ref, computed, watch, nextTick } from 'vue'

const props = defineProps<{
  visible: boolean
  position?: { x: number; y: number }
  context?: string
  placeholder?: string
}>()

const emit = defineEmits<{
  close: []
  submit: [text: string, context?: string]
}>()

const inputRef = ref<HTMLTextAreaElement | null>(null)
const inputText = ref('')

const positionStyle = computed(() => {
  if (props.position) {
    return {
      left: `${Math.min(props.position.x, window.innerWidth - 400)}px`,
      top: `${Math.min(props.position.y, window.innerHeight - 200)}px`,
    }
  }
  return {
    left: '50%',
    top: '50%',
    transform: 'translate(-50%, -50%)',
  }
})

const contextPreview = computed(() => {
  if (!props.context) return ''
  return props.context.length > 50 
    ? props.context.slice(0, 50) + '...' 
    : props.context
})

function submit() {
  const text = inputText.value.trim()
  if (!text) return
  
  emit('submit', text, props.context)
  inputText.value = ''
  emit('close')
}

// 聚焦输入框
watch(() => props.visible, (visible) => {
  if (visible) {
    nextTick(() => {
      inputRef.value?.focus()
    })
  }
})
</script>

<style scoped>
.floating-input-overlay {
  position: fixed;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  background: rgba(0, 0, 0, 0.3);
  display: flex;
  z-index: 1000;
}

.floating-input-container {
  position: absolute;
  width: 400px;
  background: var(--color-surface);
  border-radius: 12px;
  box-shadow: 0 8px 32px rgba(0, 0, 0, 0.2);
  overflow: hidden;
}

.input-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.75rem 1rem;
  background: var(--color-primary);
  color: white;
}

.input-title {
  font-weight: 500;
  font-size: 0.875rem;
}

.close-btn {
  width: 24px;
  height: 24px;
  border: none;
  background: transparent;
  color: white;
  cursor: pointer;
  font-size: 1.25rem;
  border-radius: 4px;
  opacity: 0.8;
}

.close-btn:hover {
  opacity: 1;
  background: rgba(255, 255, 255, 0.1);
}

.context-preview {
  padding: 0.5rem 1rem;
  background: #f8f9fa;
  border-bottom: 1px solid var(--color-border);
  font-size: 0.75rem;
}

.context-label {
  color: var(--color-text-muted);
  margin-right: 0.5rem;
}

.context-text {
  color: var(--color-text-secondary);
}

.input-field {
  width: 100%;
  min-height: 80px;
  padding: 1rem;
  border: none;
  background: transparent;
  font-size: 0.875rem;
  line-height: 1.5;
  resize: none;
  outline: none;
}

.input-actions {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.75rem 1rem;
  border-top: 1px solid var(--color-border);
}

.shortcut-hint {
  font-size: 0.75rem;
  color: var(--color-text-muted);
}

.submit-btn {
  padding: 0.5rem 1.5rem;
  background: var(--color-primary);
  color: white;
  border: none;
  border-radius: 6px;
  cursor: pointer;
  font-size: 0.875rem;
  font-weight: 500;
  transition: opacity 0.2s;
}

.submit-btn:hover:not(:disabled) {
  opacity: 0.9;
}

.submit-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.2s ease;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}
</style>
