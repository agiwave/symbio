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
        <button
          class="remove-image"
          type="button"
          title="移除图片"
          aria-label="移除图片"
          @click="removeImage(index)"
        >×</button>
      </div>
    </div>
    
    <div class="input-wrapper">
      <!-- 图片上传按钮 -->
      <button
        class="attach-btn"
        type="button"
        title="上传图片"
        aria-label="上传图片"
        @click="triggerImageUpload"
      >
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
        aria-label="选择图片"
        @change="handleImageSelect"
      />
      <textarea
        ref="textareaRef"
        v-model="modelValue"
        aria-label="消息输入"
        placeholder="输入消息... (可粘贴图片)"
        @keydown.enter.exact="handleKeydown"
        @paste="handlePaste"
        rows="1"
      ></textarea>
      <!-- 发送键 / 停止键：**同一个控件、同一个语义点**。
           运行中它就是停止，无论输入框里有没有字——呈现（图标 / 提示语 /
           可点性）全部由 `isLoading` 一处决定。输入文本参与呈现会让三者不一致：
           图标与提示语说「发送新消息」（用户以为在排队），而动作是把正在跑的一轮
           中止（见 `ModelChatPanel.handleSendOrAbort`）。 -->
      <button
        class="send-btn"
        :class="{ 'stop-btn': isLoading }"
        @click="$emit('submit')"
        :disabled="!isLoading && !modelValue.trim() && attachedImages.length === 0"
        :title="isLoading ? '停止' : '发送'"
        :aria-label="isLoading ? '停止' : '发送'"
      >
        <svg v-if="!isLoading" viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
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
import { useToast } from '@/composables/useToast'

/** 附件上限：前端先拦，避免大图 / 连贴把内存吃满后才由后端拒绝 */
const MAX_IMAGES = 10
const MAX_IMAGE_BYTES = 10 * 1024 * 1024

const { showToast } = useToast()

const props = defineProps<{
  isLoading: boolean
  /** 挂载即聚焦输入框（新建引导等「一进来就该能打字」的场景） */
  autofocus?: boolean
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
//
// 运行中 Enter **不提交**：它会把正在跑的一轮中止（父级的 `handleSendOrAbort`
// 在运行中只认中止），而「回车」是用户用来发送的键——用一个发送手势去中止一次
// 正在进行的生成，是这里最容易发生的误操作。中止有它自己的入口（那个红色方块
// 按钮）。因此运行中回车就是换行，用户可以先把下一句写下来。
function handleKeydown(e: KeyboardEvent) {
  // 输入法组合态：中文等 IME 下按 Enter 是**选词 / 上屏**，不是发送。
  // `isComposing` 是标准判据；`keyCode === 229` 覆盖旧 WebView 在组合期间
  // 只报 229 的情形。漏掉这一条会把用户正在打的字直接发出去。
  if (e.isComposing || e.keyCode === 229) return
  if (props.isLoading) return
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
  // 聚焦归输入框自己管：它才是 textarea 的所有者（此前由父级各写一遍
  // nextTick + `ref.textarea.focus()`，与 textarea 的暴露形状耦合在父级）。
  if (props.autofocus) void nextTick(() => textareaRef.value?.focus())
})

onUnmounted(() => {
  window.removeEventListener('paste', handleGlobalPaste)
})

/**
 * 读成 base64；三种终止事件都必须 settle。
 *
 * 此前只挂了 `onload`：读取失败（权限、文件被移走、OOM）时 Promise 永不 settle，
 * 图片静默丢失，外层的 try/catch 也接不住。
 */
function readFileAsBase64(file: File): Promise<string> {
  return new Promise<string>((resolve, reject) => {
    const reader = new FileReader()
    reader.onload = () => {
      const result = typeof reader.result === 'string' ? reader.result : ''
      const comma = result.indexOf(',')
      if (comma < 0) {
        reject(new Error('读取结果为空'))
        return
      }
      resolve(result.slice(comma + 1))
    }
    reader.onerror = () => reject(reader.error ?? new Error('读取失败'))
    reader.onabort = () => reject(new Error('读取已取消'))
    reader.readAsDataURL(file)
  })
}

async function processImageFiles(files: File[]) {
  const newImages = [...attachedImages.value]
  const failures: string[] = []
  for (const file of files) {
    if (newImages.length >= MAX_IMAGES) {
      failures.push(`${file.name}：最多附加 ${MAX_IMAGES} 张图片`)
      continue
    }
    if (file.size > MAX_IMAGE_BYTES) {
      failures.push(`${file.name}：超过 ${MAX_IMAGE_BYTES / 1024 / 1024}MB 上限`)
      continue
    }
    try {
      const base64 = await readFileAsBase64(file)
      const thumbnailUrl = URL.createObjectURL(file)
      newImages.push({
        base64,
        mimeType: file.type,
        fileName: file.name,
        thumbnailUrl
      })
    } catch (err) {
      logger.error('ChatInputArea', '处理图片失败', err)
      failures.push(`${file.name}：读取失败`)
    }
  }
  attachedImages.value = newImages
  // 一个坏文件不该吞掉整批：成功的仍然加入，失败的逐条说出来
  if (failures.length) showToast('error', failures.join('；'))
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
  background: var(--surface-sunken);
  border: 1px solid var(--border-default);
  border-radius: 0.75rem;
  padding: 0.5rem;
  transition: border-color 0.2s;
}

.input-wrapper:focus-within {
  border-color: var(--accent);
  background: var(--surface-overlay);
}

textarea {
  flex: 1;
  min-height: 1.5rem;
  max-height: 7.5rem;
  padding: 0.5rem;
  border: none;
  background: transparent;
  color: var(--text-primary);
  resize: none;
  font-size: 0.875rem;
  line-height: 1.5;
  outline: none;
  font-family: inherit;
}

textarea::placeholder {
  color: var(--text-muted);
}

.send-btn, .attach-btn {
  width: 2.25rem;
  height: 2.25rem;
  min-width: 2.25rem;
  display: flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: 0.5rem;
  cursor: pointer;
  transition: all 0.2s;
  padding: 0;
}

.send-btn { background: var(--accent); color: var(--text-on-accent); }
.send-btn:hover:not(:disabled) { opacity: 0.9; transform: scale(1.05); }
.send-btn:active:not(:disabled) { transform: scale(0.95); }
.send-btn:disabled { opacity: 0.4; cursor: not-allowed; }
/* 停止态：仅换背景，前景沿用 --text-on-accent（深/浅两态均由令牌给出对比度达标值） */
.send-btn.stop-btn { background: var(--danger-solid); }

.attach-btn { background: transparent; color: var(--text-muted); }
.attach-btn:hover { background: var(--surface-hover); color: var(--text-secondary); }

/* 键盘焦点必须可见：两个按钮在视觉上都不是输入框，Tab 到它们时要有落点 */
.attach-btn:focus-visible,
.send-btn:focus-visible,
.remove-image:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: 1px;
}

.images-preview {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
  padding: 0.5rem 0;
  margin-bottom: 0.5rem;
}

.image-item {
  position: relative;
  width: 5rem;
  height: 5rem;
  border-radius: 0.5rem;
  overflow: hidden;
  border: 1px solid var(--border-default);
}

.image-item img { width: 100%; height: 100%; object-fit: cover; }
.image-item .remove-image {
  position: absolute;
  top: var(--space-1); right: var(--space-1);
  width: 1.25rem; height: 1.25rem;
  border-radius: 50%;
  background: rgba(0, 0, 0, 0.6);
  color: white; border: none;
  cursor: pointer; font-size: 0.875rem;
  display: flex; align-items: center; justify-content: center;
  opacity: 0; transition: opacity 0.2s;
}

/* 移除按钮默认隐藏，但**聚焦时也必须出现**——否则键盘会停在一个看不见的按钮上 */
.image-item:hover .remove-image,
.image-item:focus-within .remove-image {
  opacity: 1;
}
</style>
