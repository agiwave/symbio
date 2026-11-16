<!--
  About — 关于 editor（setting:about，只读）
-->
<template>
  <SettingsFormShell title="关于">
    <div class="about-info">
      <img :src="logoUrl" alt="Symbio" class="app-logo" />
      <h1>Symbio</h1>
      <p class="version">版本 {{ version }}</p>
      <p class="description">在做中学：生信分析的互动学习平台</p>
      <!-- 此前这里有「文档 / GitHub / 反馈」三个 `href="#"`：点了什么也不会发生
           （或把应用跳到一个空 hash）。未接入系统浏览器打开能力之前不放无效入口。 -->
      <div class="links">
        <!-- 引导的重开入口：它只弹一次（且只在尚未配好时），但随时可以回来看 -->
        <button class="link-btn" type="button" @click="onboarding.openManually()">
          查看首次引导
        </button>
      </div>
    </div>
  </SettingsFormShell>
</template>

<script setup lang="ts">
import SettingsFormShell from './SettingsFormShell.vue'
import logoUrl from '../../assets/logo.svg'
import pkg from '../../../package.json'
import { useOnboardingStore } from '@/stores/onboarding'
import type { VdfsRendererProps } from '@/components/vdfs/rendererContract'

const onboarding = useOnboardingStore()

/** 版本号取自 package.json：此前写死 0.1.0，发版后与实际版本永久脱节 */
const version = pkg.version

// 声明渲染器统一契约的**全量** props：页面恒传 node / data / testing /
// mechanism-actions…，声明了才不会被透传到根 DOM（不需要 inheritAttrs 兜底）。
// 本面板只读，一律自取信息，不消费其中任何一项。
defineProps<VdfsRendererProps>()
</script>

<style scoped>
.about-info {
  text-align: center;
  padding: 3rem 0;
}
.app-logo {
  width: 5rem;
  height: 5rem;
  border-radius: var(--radius-lg);
  margin-bottom: 1rem;
}
.about-info h1 {
  font-size: 2rem;
  margin-bottom: 0.5rem;
  color: var(--text-primary);
}
.version {
  color: var(--text-muted);
  margin-bottom: 1rem;
}
.description {
  color: var(--text-secondary);
  margin-bottom: 2rem;
}
.links {
  display: flex;
  gap: 1rem;
  justify-content: center;
}
.link-btn {
  padding: 0.5rem 1.5rem;
  background: var(--surface-panel);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  color: var(--text-primary);
  text-decoration: none;
  transition: background var(--motion-fast) var(--motion-ease);
  font: inherit;
  cursor: pointer;
}
.link-btn:hover {
  background: var(--surface-hover);
}
</style>
