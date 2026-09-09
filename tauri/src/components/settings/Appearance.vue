<!--
  Appearance — 外观设置 editor（setting:appearance）

  主题 / 字体大小经 appearance store 即时生效并自动持久化，无需保存按钮。
-->
<template>
  <SettingsFormShell title="外观" description="主题与字体设置会立即生效并自动保存">
    <div class="setting-item">
      <div class="setting-info">
        <label>主题</label>
        <p class="setting-desc">选择应用的主题风格</p>
      </div>
      <div class="segmented">
        <button
          v-for="opt in themeOptions"
          :key="opt.value"
          type="button"
          class="seg-btn"
          :class="{ active: appearance.theme === opt.value }"
          @click="appearance.theme = opt.value"
        >
          {{ opt.label }}
        </button>
      </div>
    </div>

    <div class="setting-item">
      <div class="setting-info">
        <label>字体大小</label>
        <p class="setting-desc">调整界面文字大小</p>
      </div>
      <div class="segmented">
        <button
          v-for="opt in fontSizeOptions"
          :key="opt.value"
          type="button"
          class="seg-btn"
          :class="{ active: appearance.fontSize === opt.value }"
          @click="appearance.fontSize = opt.value"
        >
          {{ opt.label }}
        </button>
      </div>
    </div>

    <div class="setting-item">
      <div class="setting-info">
        <label>会话结束提示音</label>
        <p class="setting-desc">任何会话结束（完成 / 中止 / 失败）时播放提示音，不同结束类型音色不同</p>
      </div>
      <div class="segmented">
        <button
          v-for="(opt, i) in soundToggleOptions"
          :key="i"
          type="button"
          class="seg-btn"
          :class="{ active: sound.enabled === opt.value }"
          @click="sound.enabled = opt.value"
        >
          {{ opt.label }}
        </button>
      </div>
    </div>

    <div v-if="sound.enabled" class="sound-kinds">
      <div class="setting-item">
        <div class="setting-info">
          <label>正常结束</label>
          <p class="setting-desc">两音上行（明亮），会话正常完成时播放</p>
        </div>
        <div class="kind-controls">
          <button type="button" class="preview-btn" @click="preview('completed')">试听</button>
          <button
            type="button"
            class="seg-btn"
            :class="{ active: sound.kinds.completed }"
            @click="sound.kinds.completed = !sound.kinds.completed"
          >
            {{ sound.kinds.completed ? '开' : '关' }}
          </button>
        </div>
      </div>
      <div class="setting-item">
        <div class="setting-info">
          <label>中止</label>
          <p class="setting-desc">双音下行（平缓），用户主动停止会话时播放</p>
        </div>
        <div class="kind-controls">
          <button type="button" class="preview-btn" @click="preview('aborted')">试听</button>
          <button
            type="button"
            class="seg-btn"
            :class="{ active: sound.kinds.aborted }"
            @click="sound.kinds.aborted = !sound.kinds.aborted"
          >
            {{ sound.kinds.aborted ? '开' : '关' }}
          </button>
        </div>
      </div>
      <div class="setting-item">
        <div class="setting-info">
          <label>失败</label>
          <p class="setting-desc">低频双响（警示），会话异常结束时播放</p>
        </div>
        <div class="kind-controls">
          <button type="button" class="preview-btn" @click="preview('failed')">试听</button>
          <button
            type="button"
            class="seg-btn"
            :class="{ active: sound.kinds.failed }"
            @click="sound.kinds.failed = !sound.kinds.failed"
          >
            {{ sound.kinds.failed ? '开' : '关' }}
          </button>
        </div>
      </div>

      <div class="setting-item">
        <div class="setting-info">
          <label>音量</label>
          <p class="setting-desc">提示音播放音量</p>
        </div>
        <input
          v-model.number="sound.volume"
          class="volume-input"
          type="range"
          min="0"
          max="1"
          step="0.05"
        />
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
import { useAppearanceStore, type ThemeMode, type FontSize } from '@/stores/appearance'
import { useSoundSettingsStore } from '@/stores/soundSettings'
import { playCompletionChime } from '@/services/completionChime'
import SettingsFormShell from './SettingsFormShell.vue'

// 统一实体页会透传 capabilities/saving/testing/deleting 等编辑器级 props，
// 本表单不消费它们，禁止落根 DOM（清 fallthrough 污染）。
defineOptions({ inheritAttrs: false })

defineProps<{
  /** 当前设置分区实体项（统一实体协议注入） */
  item?: { id: string; name?: string } | null
}>()

const appearance = useAppearanceStore()
const sound = useSoundSettingsStore()

/** 试听对应类型的提示音（忽略单类型开关，总开关关闭时强制播放，便于在设置页确认效果） */
function preview(kind: 'completed' | 'aborted' | 'failed') {
  playCompletionChime(kind, undefined, { force: true })
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
