<!--
  AboutPanel — 关于 editor（setting:about，只读）
-->
<template>
  <SettingsFormShell title="关于">
    <div class="about-info">
      <img :src="logoUrl" alt="Symbio" class="app-logo" />
      <h1>Symbio</h1>
      <p class="version">版本 0.1.0</p>
      <p class="description">在做中学：生信分析的互动学习平台</p>
      <div class="links">
        <a href="#" class="link-btn">文档</a>
        <a href="#" class="link-btn">GitHub</a>
        <a href="#" class="link-btn">反馈</a>
      </div>
    </div>
  </SettingsFormShell>
</template>

<script setup lang="ts">
import SettingsFormShell from './SettingsFormShell.vue'
import logoUrl from '../../assets/logo.svg'

defineProps<{
  /** 当前设置分区资源项（统一资源协议注入） */
  item?: { id: string; name?: string } | null
}>()
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
}
.link-btn:hover {
  background: var(--surface-hover);
}
</style>
