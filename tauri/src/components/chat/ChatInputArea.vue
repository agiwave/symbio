<template>
  <div class="chat-input-area">
    <!-- 图片预览区域 -->
    <div v-if="attachedImages.length > 0" class="images-preview">
      <div
        v-for="(img, index) in attachedImages"
        :key="index"
        class="image-item"
      >
        <img :src="img.thumbnailUrl" :alt="img.fileName || '图片'" />
        <button class="remove-image" @click="removeImage(index)" title="移除图片">×</button>
      </div>
    </div>
    
    <div class="input-wrapper">
      <!-- 图片上传按钮 -->
      <button class="attach-btn" @click="triggerImageUpload" title="上传图片">
        <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <rect x="3" y="3" width="18" height="18" rx="2" ry="2"></rect>
          <circle cx="8.5" cy="8.5" r="1.5"></circle>
          <polyline points="21 15 16 10 5 21"></polyline>
        </svg>
      </button>
      <input
        ref="imageInputRef"
        type="file"
        accept="image/*"
        multiple
        style="display: none"
        @change="handleImageSelect"
      />
      <textarea
        ref="textareaRef"
        v-model="modelValue"
        placeholder="输入消息... (可粘贴图片)"
        @keydown.enter.exact="handleKeydown"
        @paste="handlePaste"
        rows="1"
      ></textarea>
      <button
        class="send-btn"
        :class="{ 'stop-btn': isLoading && !modelValue.trim() }"
        @click="$emit('submit')"
        :disabled="!isLoading && !modelValue.trim() && attachedImages.length === 0"
        :title="isLoading ? (modelValue.trim() ? '发送新消息' : '停止') : '发送'"
      >
        <svg v-if="!isLoading || modelValue.trim()" viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
          <line x1="22" y1="2" x2="11" y2="13"></line>
          <polygon points="22 2 15 22 11 13 2 9 22 2"></polygon>
        </svg>
        <svg v-else viewBox="0 0 24 24" width="20" height="20" fill="currentColor">
          <rect x="6" y="6" width="12" height="12" rx="2"></rect>
        </svg>
      </button>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, watch, nextTick, onMounted, onUnmounted } from 'vue'
import type { ImageAttachment } from '@/types'
import { logger } from '@/utils/logger'

const props = defineProps<{
  isLoading: boolean
}>()

const modelValue = defineModel<string>({ default: '' })
const attachedImages = defineModel<ImageAttachment[]>('attachedImages', { default: () => [] })

const emit = defineEmits<{
  'submit': []
}>()

const textareaRef = ref<HTMLTextAreaElement | null>(null)
const imageInputRef = ref<HTMLInputElement | null>(null)

// 暴露给父组件，用于重置高度
function resetHeight() {
  if (textareaRef.value) {
    textareaRef.value.style.height = 'auto'
  }
}

defineExpose({ resetHeight, textarea: textareaRef })

// 键盘事件
function handleKeydown(e: KeyboardEvent) {
  if (!e.shiftKey) {
    e.preventDefault()
    emit('submit')
  }
}

// 图片逻辑
function triggerImageUpload() {
  imageInputRef.value?.click()
}

async function handleImageSelect(event: Event) {
  const input = event.target as HTMLInputElement
  if (input.files) {
    await processImageFiles(Array.from(input.files))
  }
  input.value = ''
}

async function handlePaste(event: ClipboardEvent) {
  const files = event.clipboardData?.files
  const items = event.clipboardData?.items
  
  const imageFiles: File[] = []
  
  // 1. Try reading from files first (e.g. copied from Windows File Explorer)
  if (files && files.length > 0) {
    for (let i = 0; i < files.length; i++) {
      const file = files[i]
      if (file.type.startsWith('image/')) {
        imageFiles.push(file)
      }
    }
  }
  
  // 2. Fallback to items (e.g. screenshot clipboards or browser "Copy Image")
  if (imageFiles.length === 0 && items) {
    for (const item of items) {
      if (item.type.startsWith('image/')) {
        const file = item.getAsFile()
        if (file) {
          imageFiles.push(file)
        }
      }
    }
  }

  if (imageFiles.length > 0) {
    event.preventDefault()
    await processImageFiles(imageFiles)
  }
}

// Global paste listener: captures pasted images anywhere in the page, as long as another text field is not active.
function handleGlobalPaste(event: ClipboardEvent) {
  const activeEl = document.activeElement
  if (
    activeEl &&
    activeEl !== textareaRef.value &&
    (activeEl.tagName === 'INPUT' || activeEl.tagName === 'TEXTAREA' || (activeEl as HTMLElement).isContentEditable)
  ) {
    return
  }
  handlePaste(event)
}

onMounted(() => {
  window.addEventListener('paste', handleGlobalPaste)
})

onUnmounted(() => {
  window.removeEventListener('paste', handleGlobalPaste)
})

async function processImageFiles(files: File[]) {
  const newImages = [...attachedImages.value]
  for (const file of files) {
    try {
      const reader = new FileReader()
      const base64 = await new Promise<string>((resolve) => {
        reader.onload = () => resolve((reader.result as string).split(',')[1])
        reader.readAsDataURL(file)
      })
      const thumbnailUrl = URL.createObjectURL(file)
      newImages.push({
        base64,
        mimeType: file.type,
        fileName: file.name,
        thumbnailUrl
      })
    } catch (err) {
      logger.error('ChatInputArea', '处理图片失败', err)
    }
  }
  attachedImages.value = newImages
}

function removeImage(index: number) {
  const newImages = [...attachedImages.value]
  const img = newImages[index]
  if (img.thumbnailUrl) URL.revokeObjectURL(img.thumbnailUrl)
  newImages.splice(index, 1)
  attachedImages.value = newImages
}

// 自动高度
watch(modelValue, () => {
  if (textareaRef.value) {
    textareaRef.value.style.height = 'auto'
    nextTick(() => {
      if (textareaRef.value) {
        textareaRef.value.style.height = `${textareaRef.value.scrollHeight}px`
      }
    })
  }
})
</script>

<style scoped>
.chat-input-area {
  display: flex;
  flex-direction: column;
}

.input-wrapper {
  display: flex;
  align-items: flex-end;
  gap: 0.5rem;
  background: #f5f5f5;
  border: 1px solid var(--color-border);
  border-radius: 12px;
  padding: 0.5rem;
  transition: border-color 0.2s;
}

.input-wrapper:focus-within {
  border-color: var(--color-primary);
  background: #fff;
}

textarea {
  flex: 1;
  min-height: 24px;
  max-height: 120px;
  padding: 0.5rem;
  border: none;
  background: transparent;
  resize: none;
  font-size: 0.875rem;
  line-height: 1.5;
  outline: none;
  font-family: inherit;
}

.send-btn, .attach-btn {
  width: 36px;
  height: 36px;
  min-width: 36px;
  display: flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: 8px;
  cursor: pointer;
  transition: all 0.2s;
  padding: 0;
}

.send-btn { background: var(--color-primary); color: white; }
.send-btn:hover:not(:disabled) { opacity: 0.9; transform: scale(1.05); }
.send-btn:active:not(:disabled) { transform: scale(0.95); }
.send-btn:disabled { opacity: 0.4; cursor: not-allowed; }
.send-btn.stop-btn { background: #dc3545; }

.attach-btn { background: transparent; color: var(--color-text-muted); }
.attach-btn:hover { background: rgba(0, 0, 0, 0.05); color: var(--color-text-secondary); }

.images-preview {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
  padding: 0.5rem 0;
  margin-bottom: 0.5rem;
}

.image-item {
  position: relative;
  width: 80px;
  height: 80px;
  border-radius: 8px;
  overflow: hidden;
  border: 1px solid var(--color-border);
}

.image-item img { width: 100%; height: 100%; object-fit: cover; }
.image-item .remove-image {
  position: absolute;
  top: 4px; right: 4px;
  width: 20px; height: 20px;
  border-radius: 50%;
  background: rgba(0, 0, 0, 0.6);
  color: white; border: none;
  cursor: pointer; font-size: 14px;
  display: flex; align-items: center; justify-content: center;
  opacity: 0; transition: opacity 0.2s;
}

.image-item:hover .remove-image { opacity: 1; }
</style>
