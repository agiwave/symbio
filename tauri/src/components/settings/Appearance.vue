<!--
  Appearance — 外观设置 editor（setting:appearance）

  主题 / 字体大小经 appearance store 即时生效并自动持久化，无需保存按钮。

  ## 分区

  本页承载两类设置：**外观**（主题 / 字体）与**提示音**（开关 / 分类型 / 音量）。
  两者数据都住在前端 store、同属这个 editor，此前却完全平铺在一串 setting 行里
  ——用户找「声音」时以为走错了页面。现在用分节标题把它们分开，并在标题上说明
  各自的生效方式。
-->
<template>
  <SettingsFormShell title="外观与提示音" description="以下设置会立即生效并自动保存">
    <p class="section-title">外观</p>
    <div class="setting-item">
      <div class="setting-info">
        <span class="setting-label">主题</span>
        <p class="setting-desc">选择应用的主题风格</p>
      </div>
      <div class="segmented" role="group" aria-label="主题" @keydown="moveSegment">
        <button
          v-for="opt in themeOptions"
          :key="opt.value"
          type="button"
          class="seg-btn"
          :class="{ active: appearance.theme === opt.value }"
          :aria-pressed="appearance.theme === opt.value"
          @click="appearance.theme = opt.value"
        >
          {{ opt.label }}
        </button>
      </div>
    </div>

    <div class="setting-item">
      <div class="setting-info">
        <span class="setting-label">字体大小</span>
        <p class="setting-desc">调整界面文字大小</p>
      </div>
      <div class="segmented" role="group" aria-label="字体大小" @keydown="moveSegment">
        <button
          v-for="opt in fontSizeOptions"
          :key="opt.value"
          type="button"
          class="seg-btn"
          :class="{ active: appearance.fontSize === opt.value }"
          :aria-pressed="appearance.fontSize === opt.value"
          @click="appearance.fontSize = opt.value"
        >
          {{ opt.label }}
        </button>
      </div>
    </div>

    <p class="section-title">提示音</p>
    <div class="setting-item">
      <div class="setting-info">
        <span class="setting-label">会话结束提示音</span>
        <p class="setting-desc">任何会话结束（完成 / 中止 / 失败）时播放提示音，不同结束类型音色不同</p>
      </div>
      <div class="segmented" role="group" aria-label="会话结束提示音" @keydown="moveSegment">
        <button
          v-for="(opt, i) in soundToggleOptions"
          :key="i"
          type="button"
          class="seg-btn"
          :class="{ active: sound.enabled === opt.value }"
          :aria-pressed="sound.enabled === opt.value"
          @click="sound.enabled = opt.value"
        >
          {{ opt.label }}
        </button>
      </div>
    </div>

    <div v-if="sound.enabled" class="sound-kinds">
      <div class="setting-item">
        <div class="setting-info">
          <span class="setting-label">正常结束</span>
          <p class="setting-desc">两音上行（明亮），会话正常完成时播放</p>
        </div>
        <div class="kind-controls">
          <button type="button" class="preview-btn" @click="preview('completed')">试听</button>
          <button
            type="button"
            class="seg-btn"
            :class="{ active: sound.kinds.completed }"
            :aria-pressed="sound.kinds.completed"
            @click="sound.kinds.completed = !sound.kinds.completed"
          >
            {{ sound.kinds.completed ? '开' : '关' }}
          </button>
        </div>
      </div>
      <div class="setting-item">
        <div class="setting-info">
          <span class="setting-label">中止</span>
          <p class="setting-desc">双音下行（平缓），用户主动停止会话时播放</p>
        </div>
        <div class="kind-controls">
          <button type="button" class="preview-btn" @click="preview('aborted')">试听</button>
          <button
            type="button"
            class="seg-btn"
            :class="{ active: sound.kinds.aborted }"
            :aria-pressed="sound.kinds.aborted"
            @click="sound.kinds.aborted = !sound.kinds.aborted"
          >
            {{ sound.kinds.aborted ? '开' : '关' }}
          </button>
        </div>
      </div>
      <div class="setting-item">
        <div class="setting-info">
          <span class="setting-label">失败</span>
          <p class="setting-desc">低频双响（警示），会话异常结束时播放</p>
        </div>
        <div class="kind-controls">
          <button type="button" class="preview-btn" @click="preview('failed')">试听</button>
          <button
            type="button"
            class="seg-btn"
            :class="{ active: sound.kinds.failed }"
            :aria-pressed="sound.kinds.failed"
            @click="sound.kinds.failed = !sound.kinds.failed"
          >
            {{ sound.kinds.failed ? '开' : '关' }}
          </button>
        </div>
      </div>

      <div class="setting-item">
        <div class="setting-info">
          <label class="setting-label" for="sound-volume">音量</label>
          <p class="setting-desc">提示音播放音量</p>
        </div>
        <div class="volume-control">
          <input
            id="sound-volume"
            v-model.number="sound.volume"
            class="volume-input"
            aria-label="提示音音量"
            :aria-valuetext="`${volumePercent}%`"
            type="range"
            min="0"
            max="1"
            step="0.05"
          />
          <!-- 滑块没有刻度，拖动时回答不了「现在多少」——数值必须可见 -->
          <span class="volume-value" :class="{ muted: volumePercent === 0 }">
            {{ volumePercent }}%
          </span>
        </div>
      </div>
    </div>

    <div class="appearance-preview">
      <p class="preview-title">排版预览</p>
      <div class="preview-card">
        <h4>明晰的标题</h4>
        <p>这是一段用于预览字体大小与主题配色效果的示例文字，会随你在上面的选择实时变化。</p>
        <button class="preview-chip" type="button">代码片段</button>
      </div>
    </div>
  </SettingsFormShell>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import { useAppearanceStore, type ThemeMode, type FontSize } from '@/stores/appearance'
import { useSoundSettingsStore } from '@/stores/soundSettings'
import { playCompletionChime } from '@/services/completionChime'
import { useToast } from '@/composables/useToast'
import SettingsFormShell from './SettingsFormShell.vue'
import type { VdfsRendererProps } from '@/components/vdfs/rendererContract'

// 声明渲染器统一契约的**全量** props：页面恒传 node / data / testing /
// mechanism-actions…，声明了才不会被透传到根 DOM（不需要 inheritAttrs 兜底）。
// 本表单一律自取 store，不消费其中任何一项。
defineProps<VdfsRendererProps>()

const appearance = useAppearanceStore()
const sound = useSoundSettingsStore()
const { showToast } = useToast()

/** 音量百分比（四舍五入到整数；滑块步长 0.05，显示到整数足够） */
const volumePercent = computed(() => Math.round(sound.volume * 100))

/**
 * 试听对应类型的提示音（忽略单类型开关，总开关关闭时强制播放，便于在设置页确认效果）。
 *
 * 音量为 0 时**必须说出来**：此时试听一定无声，用户会以为试听坏了（或提示音功能
 * 坏了），而事实只是滑块在 0。
 */
function preview(kind: 'completed' | 'aborted' | 'failed') {
  if (sound.volume <= 0) {
    showToast('info', '当前音量为 0，调大音量后才能听到试听')
    return
  }
  playCompletionChime(kind, undefined, { force: true })
}

/**
 * 分段控件的方向键：左/右（上/下）在选项间移动并**就地生效**，Home/End 到两端。
 *
 * 分段控件是一组互斥选择，键盘用户不该被迫 Tab 三次才能改一个值；`role=group`
 * + 按钮的既有语义保持不变，这里只补方向键这一条通用交互。
 */
function moveSegment(e: KeyboardEvent) {
  const keys = ['ArrowLeft', 'ArrowRight', 'ArrowUp', 'ArrowDown', 'Home', 'End']
  if (!keys.includes(e.key)) return
  const group = e.currentTarget as HTMLElement | null
  const btns = Array.from(group?.querySelectorAll<HTMLButtonElement>('.seg-btn') ?? [])
  const idx = btns.indexOf(document.activeElement as HTMLButtonElement)
  if (idx < 0 || btns.length === 0) return
  e.preventDefault()
  let next = idx
  if (e.key === 'ArrowLeft' || e.key === 'ArrowUp') next = (idx - 1 + btns.length) % btns.length
  else if (e.key === 'ArrowRight' || e.key === 'ArrowDown') next = (idx + 1) % btns.length
  else if (e.key === 'Home') next = 0
  else next = btns.length - 1
  btns[next].focus()
  btns[next].click()
}

const soundToggleOptions: Array<{ value: boolean; label: string }> = [
  { value: true, label: '开启' },
  { value: false, label: '关闭' },
]

const themeOptions: Array<{ value: ThemeMode; label: string }> = [
  { value: 'light', label: '浅色' },
  { value: 'dark', label: '深色' },
  { value: 'auto', label: '跟随系统' },
]
const fontSizeOptions: Array<{ value: FontSize; label: string }> = [
  { value: 'small', label: '小' },
  { value: 'medium', label: '中' },
  { value: 'large', label: '大' },
]
</script>

<style scoped>
.appearance-preview {
  padding: 1rem;
  border-top: 1px solid var(--border-default);
}
.preview-title {
  font-size: var(--font-size-xs);
  color: var(--text-muted);
  margin-bottom: 0.75rem;
}
.preview-card {
  border: 1px solid var(--border-default);
  border-radius: var(--radius-lg);
  padding: 1rem 1.25rem;
  background: var(--surface-sunken);
}
.preview-card h4 {
  font-size: 1.125rem;
  margin-bottom: 0.4rem;
  color: var(--text-primary);
}
.preview-card p {
  font-size: var(--font-size-base);
  color: var(--text-secondary);
  margin-bottom: 0.75rem;
}
.preview-chip {
  padding: 0.3rem 0.75rem;
  background: var(--surface-panel);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  color: var(--accent);
  font-size: 0.8rem;
  cursor: default;
  font-family: var(--font-mono);
}
.sound-kinds {
  display: flex;
  flex-direction: column;
  border-top: 1px solid var(--border-default);
}

/* 分节标题：把「外观」与「提示音」两类设置分开 */
.section-title {
  margin: 0.75rem 0 0.25rem;
  padding: 0 1rem;
  font-size: var(--font-size-xs);
  font-weight: var(--font-weight-semibold);
  letter-spacing: 0.04em;
  color: var(--text-muted);
}
.section-title:first-child {
  margin-top: 0;
}

.volume-control {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  flex-shrink: 0;
}
.volume-value {
  min-width: 2.5rem;
  text-align: right;
  font-size: var(--font-size-xs);
  font-variant-numeric: tabular-nums;
  color: var(--text-secondary);
}
.volume-value.muted {
  color: var(--color-tag-warn-fg);
}
.kind-controls {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  flex-shrink: 0;
}
.preview-btn {
  padding: 0.3rem 0.7rem;
  font-size: 0.8rem;
  color: var(--accent);
  background: var(--surface-panel);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  cursor: pointer;
}
.preview-btn:hover {
  border-color: var(--accent);
}
.volume-input {
  width: 160px;
  accent-color: var(--accent);
  cursor: pointer;
}
</style>
