<template>
  <Teleport to="body">
    <Transition name="bubble">
      <div 
        v-if="visible"
        class="ai-bubble"
        :class="type"
        @click="handleClick"
        @mouseenter="pauseTimer"
        @mouseleave="resumeTimer"
      >
        <div class="bubble-icon">{{ icon }}</div>
        <div class="bubble-content">
          <div class="bubble-title">{{ title }}</div>
          <div v-if="message" class="bubble-message">{{ message }}</div>
        </div>
        <button class="bubble-close" @click.stop="$emit('close')">×</button>
        
        <div v-if="actions.length > 0" class="bubble-actions">
          <button 
            v-for="(action, index) in actions" 
            :key="index"
            class="action-btn"
            @click.stop="$emit('action', action.id)"
          >
            {{ action.label }}
          </button>
        </div>
      </div>
    </Transition>
  </Teleport>
</template>

<script setup lang="ts">
import { computed, watch, onUnmounted } from 'vue'

interface Action {
  id: string
  label: string
}

const props = withDefaults(defineProps<{
  visible: boolean
  type?: 'info' | 'warning' | 'success' | 'error'
  title: string
  message?: string
  actions?: Action[]
  autoDismiss?: number // seconds, 0 = no auto dismiss
}>(), {
  type: 'info',
  actions: () => [],
  autoDismiss: 3,
})

const emit = defineEmits<{
  close: []
  click: []
  action: [id: string]
}>()

const icon = computed(() => {
  switch (props.type) {
    case 'warning': return '⚠️'
    case 'success': return '✅'
    case 'error': return '❌'
    default: return '💡'
  }
})

let timer: ReturnType<typeof setTimeout> | null = null
let paused = false
let remaining = 0

function startTimer() {
  if (props.autoDismiss <= 0) return
  
  remaining = props.autoDismiss * 1000
  timer = setTimeout(() => {
    if (!paused) {
      emit('close')
    }
  }, remaining)
}

function clearTimer() {
  if (timer) {
    clearTimeout(timer)
    timer = null
  }
}

function pauseTimer() {
  paused = true
  clearTimer()
}

function resumeTimer() {
  paused = false
  if (remaining > 0) {
    timer = setTimeout(() => {
      emit('close')
    }, remaining)
  }
}

function handleClick() {
  emit('click')
}

watch(() => props.visible, (visible) => {
  if (visible) {
    startTimer()
  } else {
    clearTimer()
  }
})

onUnmounted(() => {
  clearTimer()
})
</script>

<style scoped>
.ai-bubble {
  position: fixed;
  right: 24px;
  bottom: 24px;
  max-width: 320px;
  background: var(--color-surface);
  border-radius: 12px;
  box-shadow: 0 4px 16px rgba(0, 0, 0, 0.15);
  padding: 1rem;
  display: flex;
  flex-wrap: wrap;
  gap: 0.75rem;
  cursor: pointer;
  z-index: 900;
  border-left: 4px solid var(--color-primary);
}

.ai-bubble.warning {
  border-left-color: #f59e0b;
}

.ai-bubble.success {
  border-left-color: #10b981;
}

.ai-bubble.error {
  border-left-color: #ef4444;
}

.bubble-icon {
  font-size: 1.5rem;
  flex-shrink: 0;
}

.bubble-content {
  flex: 1;
  min-width: 0;
}

.bubble-title {
  font-weight: 600;
  font-size: 0.875rem;
  color: var(--color-text);
}

.bubble-message {
  font-size: 0.75rem;
  color: var(--color-text-secondary);
  margin-top: 0.25rem;
}

.bubble-close {
  width: 20px;
  height: 20px;
  border: none;
  background: transparent;
  cursor: pointer;
  font-size: 1rem;
  color: var(--color-text-muted);
  border-radius: 4px;
  opacity: 0.6;
}

.bubble-close:hover {
  opacity: 1;
  background: #f0f0f0;
}

.bubble-actions {
  width: 100%;
  display: flex;
  gap: 0.5rem;
  margin-top: 0.5rem;
}

.action-btn {
  flex: 1;
  padding: 0.375rem 0.75rem;
  border: 1px solid var(--color-border);
  background: transparent;
  border-radius: 4px;
  cursor: pointer;
  font-size: 0.75rem;
  color: var(--color-text-secondary);
  transition: all 0.2s;
}

.action-btn:hover {
  background: var(--color-primary);
  border-color: var(--color-primary);
  color: white;
}

.bubble-enter-active {
  animation: slideIn 0.3s ease;
}

.bubble-leave-active {
  animation: slideOut 0.3s ease;
}

@keyframes slideIn {
  from {
    opacity: 0;
    transform: translateY(20px);
  }
  to {
    opacity: 1;
    transform: translateY(0);
  }
}

@keyframes slideOut {
  from {
    opacity: 1;
    transform: translateY(0);
  }
  to {
    opacity: 0;
    transform: translateY(20px);
  }
}
</style>
