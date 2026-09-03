<template>
  <div class="workdir-picker" :class="{ disabled: !canChange, 'no-workdir': !workdir }">
    <div
      class="picker-btn"
      :class="{ clickable: canChange }"
      :title="fullTitle"
      @click="onClick"
    >
      <span class="icon">📁</span>
      <span class="label">{{ displayLabel }}</span>
      <span v-if="canChange" class="change-hint">更换</span>
      <span v-else-if="hasMessages" class="lock-hint" :title="lockReason">🔒</span>
    </div>
  </div>
</template>

<script setup lang="ts">
/**
 * 工作目录选择器（用于 Model \u5bf9\u8bdd输入区下方）
 *
 * ## 行为
 *
 * - 显示当前会话的 workdir（basename），鼠标悬停显示完整路径
 * - 当 workdir 未设置时：显示「未选择目录」，点击直接打开目录选择
 * - 当 workdir 已设置 + 会话**没有消息历史**时：可点击「更换」打开目录选择
 * - 当 workdir 已设置 + 会话**有消息历史**时：禁用 + 🔒 锁标记
 *   - 原因：不同 workdir 的对话历史混在一起会让 Model 上下文困惑
 *
 * ## 配合
 *
 * - 与 ChatSettings 一起放在 `chat-controls` 下方
 * - 借助 store.sessionMessages[sessionId] 实时判断历史是否为空
 */
import { computed } from 'vue'
import { open } from '@tauri-apps/plugin-dialog'
import { useSessionsStore } from '@/stores/sessions'
import { logger } from '@/utils/logger'

const props = defineProps<{
  /** 当前会话 ID */
  sessionId: string
  /** 当前会话的 workdir（可选） */
  workdir: string | null
  /** 当前会话的消息条数（按 sort_index 排序后的总数） */
  messageCount: number
}>()

const store = useSessionsStore()

const hasMessages = computed(() => props.messageCount > 0)
const canChange = computed(() => !hasMessages.value) // 没消息才能换

const lockReason = computed(() =>
  '当前会话已有对话历史，不能更换工作目录（不同目录的上下文混在一起会干扰 AI）。\n如需在新目录中对话，请新建一个会话。'
)

function basename(p: string | null): string {
  if (!p) return ''
  return p.replace(/\\/g, '/').split('/').filter(Boolean).pop() || p
}

const displayLabel = computed(() => {
  if (!props.workdir) return '未选择目录'
  return basename(props.workdir)
})

const fullTitle = computed(() => {
  if (!props.workdir) {
    return canChange.value ? '点击选择工作目录' : '请先选择工作目录'
  }
  return props.workdir + (canChange.value ? '\n（点击更换）' : `\n${lockReason.value}`)
})

async function onClick() {
  if (!canChange.value) return
  try {
    const selected = await open({
      directory: true,
      multiple: false,
      title: props.workdir ? '更换工作目录' : '选择工作目录'
    })
    if (!selected) return
    const path = typeof selected === 'string'
      ? selected
      : Array.isArray(selected)
        ? (selected[0] as string)
        : null
    if (!path) return
    // 资源浏览器的重置和重载由 SessionExplorerPanel 监听 activeWorkdir 自动处理
    await store.setActiveWorkdir(path)
  } catch (e) {
    logger.error('WorkdirPicker', '选择工作目录失败', e)
  }
}
</script>

<style scoped>
.workdir-picker {
  display: inline-flex;
  align-items: center;
  position: relative;
}

.picker-btn {
  display: inline-flex;
  align-items: center;
  gap: 0.3rem;
  padding: 0.35rem 0.6rem;
  border-radius: 0.375rem;
  font-size: 0.75rem;
  color: var(--color-text-muted);
  user-select: none;
  transition: all 0.2s;
  max-width: 15rem;
  min-width: 0;
}

.picker-btn .icon { font-size: 0.875rem; flex-shrink: 0; }
.picker-btn .label {
  font-weight: 500;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  flex: 1;
  min-width: 0;
}

.picker-btn.clickable {
  cursor: pointer;
}

.picker-btn.clickable:hover {
  background: rgba(0, 0, 0, 0.04);
  color: var(--color-text-secondary);
}

.picker-btn.clickable:hover .change-hint {
  opacity: 1;
}

.change-hint {
  font-size: 0.7rem;
  padding: 0.05rem 0.3rem;
  border-radius: 0.1875rem;
  background: rgba(0, 0, 0, 0.05);
  color: var(--color-text-secondary);
  opacity: 0.6;
  transition: opacity 0.2s;
  flex-shrink: 0;
}

.lock-hint {
  font-size: 0.7rem;
  opacity: 0.6;
  flex-shrink: 0;
}

/* 没选 workdir 时更醒目 */
.workdir-picker.no-workdir .picker-btn {
  color: var(--color-primary);
  background: rgba(99, 102, 241, 0.06);
}

.workdir-picker.no-workdir .picker-btn:hover {
  background: rgba(99, 102, 241, 0.12);
}

/* 禁用态：变灰 + 不可点 */
.workdir-picker.disabled .picker-btn {
  cursor: not-allowed;
  color: var(--color-text-muted);
  opacity: 0.75;
}
</style>
