<!--
  Appearance — 外观设置 editor（setting:appearance）

  主题 / 字体大小经 appearance store 即时生效并自动持久化（首屏快照），但权威值
  来自 ui_prefs 的配置文档：读取时对齐，修改后写回（D-521）。

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
          @click="setTheme(opt.value)"
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
          @click="setFontSize(opt.value)"
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
          @click="setEnabled(opt.value)"
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
            @click="toggleKind('completed')"
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
            @click="toggleKind('aborted')"
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
            @click="toggleKind('failed')"
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
            :value="sound.volume"
            @input="onVolumeInput"
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
import { computed, watch } from 'vue'
import { useAppearanceStore, type ThemeMode, type FontSize } from '@/stores/appearance'
import { useSoundSettingsStore } from '@/stores/soundSettings'
import { playCompletionChime } from '@/services/completionChime'
import { useToast } from '@/composables/useToast'
import SettingsFormShell from './SettingsFormShell.vue'
import type { VdfsRendererProps } from '@/components/vdfs/rendererContract'
import type { CompletionKind } from '@/schemas/vdfs'

// 声明渲染器统一契约的**全量** props：页面恒传 node / data / testing /
// mechanism-actions…，声明了才不会被透传到根 DOM（不需要 inheritAttrs 兜底）。
// `data` 是 ui_prefs 下发的 JSON 正文（后端权威值，D-521）；store 保留为
// 首屏同步快照与离线降级——main.ts 在挂载前同步 apply()，这条路径不能改成异步读。
const props = defineProps<VdfsRendererProps>()
const emit = defineEmits<{ (e: 'save', payload: string): void }>()

const appearance = useAppearanceStore()
const sound = useSoundSettingsStore()
const { showToast } = useToast()

/** 后端配置正文的形状（字段与 ui_prefs 的 UiPrefsConfig 一一对应） */
interface BackendPrefs {
  theme?: unknown
  font_size?: unknown
  sound_enabled?: unknown
  sound_completed?: unknown
  sound_aborted?: unknown
  sound_failed?: unknown
  volume?: unknown
}

/** 正文是 JSON 文本；解析不出来时保持首屏快照（插件未装配 / 后端读取失败的降级） */
function parseBackend(raw: unknown): BackendPrefs | null {
  if (typeof raw !== 'string' || !raw.trim()) return null
  try {
    const parsed: unknown = JSON.parse(raw)
    return parsed && typeof parsed === 'object' && !Array.isArray(parsed)
      ? (parsed as BackendPrefs)
      : null
  } catch {
    return null
  }
}

function isTheme(v: unknown): v is ThemeMode {
  return v === 'light' || v === 'dark' || v === 'auto'
}
function isFontSize(v: unknown): v is FontSize {
  return v === 'small' || v === 'medium' || v === 'large'
}

// 后端权威值到达时覆盖首屏快照；store 的 watch 会同步持久化并即时应用主题。
// 这里**不** emit save——否则「读回 → 写回」会形成回环。
watch(
  () => props.data,
  (raw) => {
    const p = parseBackend(raw)
    if (!p) return
    if (isTheme(p.theme)) appearance.theme = p.theme
    if (isFontSize(p.font_size)) appearance.fontSize = p.font_size
    if (typeof p.sound_enabled === 'boolean') sound.enabled = p.sound_enabled
    if (typeof p.sound_completed === 'boolean') sound.kinds.completed = p.sound_completed
    if (typeof p.sound_aborted === 'boolean') sound.kinds.aborted = p.sound_aborted
    if (typeof p.sound_failed === 'boolean') sound.kinds.failed = p.sound_failed
    if (typeof p.volume === 'number' && Number.isFinite(p.volume)) {
      sound.volume = Math.min(1, Math.max(0, p.volume))
    }
  },
  { immediate: true },
)

/** 把当前偏好写成完整配置 JSON，交工作台写回 ui_prefs */
function persist(): void {
  emit(
    'save',
    JSON.stringify({
      theme: appearance.theme,
      font_size: appearance.fontSize,
      sound_enabled: sound.enabled,
      sound_completed: sound.kinds.completed,
      sound_aborted: sound.kinds.aborted,
      sound_failed: sound.kinds.failed,
      volume: sound.volume,
    }),
  )
}

function setTheme(v: ThemeMode): void {
  appearance.theme = v
  persist()
}
function setFontSize(v: FontSize): void {
  appearance.fontSize = v
  persist()
}
function setEnabled(v: boolean): void {
  sound.enabled = v
  persist()
}
function toggleKind(kind: CompletionKind): void {
  sound.kinds[kind] = !sound.kinds[kind]
  persist()
}
function onVolumeInput(e: Event): void {
  const v = Number((e.target as HTMLInputElement).value)
  if (Number.isFinite(v)) {
    sound.volume = Math.min(1, Math.max(0, v))
    persist()
  }
}

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
