<!--
  AppearanceSettingsForm — 外观设置 editor（setting:appearance）

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
import SettingsFormShell from './SettingsFormShell.vue'

defineProps<{
  /** 当前设置分区资源项（统一资源协议注入） */
  item?: { id: string; name?: string } | null
}>()

const appearance = useAppearanceStore()

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
</style>
