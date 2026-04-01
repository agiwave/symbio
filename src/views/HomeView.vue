<template>
  <div class="home-view">
    <!-- 顶层导航条 (始终显示) -->
    <nav class="nav-bar">
      <div class="nav-logo" @click.stop.prevent="handleNavClick('workspace')">
        <img :src="logoUrl" alt="Symbio" class="logo-img" />
      </div>
      <div class="nav-items">
        <button 
          class="nav-btn" 
          :class="{ active: currentPage === 'workspace' }"
          title="工作区" 
          @click.stop.prevent="handleNavClick('workspace')"
        >
          📁
        </button>
        <button 
          class="nav-btn" 
          :class="{ active: currentPage === 'agent' }"
          title="AI 交互" 
          @click.stop.prevent="handleNavClick('agent')"
        >
          💬
        </button>
        <button 
          class="nav-btn" 
          :class="{ active: currentPage === 'settings' }"
          title="设置" 
          @click.stop.prevent="handleNavClick('settings')"
        >
          ⚙️
        </button>
      </div>
    </nav>

    <!-- 主内容区 -->
    <div class="content-area">
      <!-- 工作区页面 -->
      <WorkspacePage v-if="currentPage === 'workspace'" />
      
      <!-- AI 交互页面 -->
      <AgentPage v-if="currentPage === 'agent'" />
      
      <!-- 设置页面 -->
      <SettingsPage v-if="currentPage === 'settings'" />
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, nextTick } from 'vue'
import WorkspacePage from '../components/WorkspacePage.vue'
import AgentPage from '../components/AgentPage.vue'
import SettingsPage from '../components/SettingsPage.vue'
import logoUrl from '../assets/logo.svg'

// 当前页面
const currentPage = ref<'workspace' | 'agent' | 'settings'>('workspace')

// Tauri WebView 修复：显式处理导航点击
function handleNavClick(page: 'workspace' | 'agent' | 'settings') {
  currentPage.value = page
  // 强制触发 Vue 更新
  nextTick(() => {
    // 确保 Vue 宄成响应式更新
  })
}
</script>

<style scoped>
.home-view {
  display: flex;
  height: 100%;
  width: 100%;
  background: var(--color-bg);
}

/* 导航条 */
.nav-bar {
  width: var(--sidebar-width, 56px);
  background: #1a1a2e;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  z-index: 100;  /* 提高 z-index */
  position: relative;  /* 保持 relative 但移除 position: absolute */
}

.nav-logo {
  cursor: pointer;
  margin-top: 1rem;
  /* 移除 position: absolute */
}

.nav-items {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  position: relative;  /* 添加 position */
  z-index: 1;  /* 确保在 nav-bar 上层 */
}

.logo-img {
  width: 36px;
  height: 36px;
  display: block;
}

.nav-btn {
  width: 36px;
  height: 36px;
  border: none;
  background: transparent;
  border-radius: 8px;
  cursor: pointer;
  font-size: 1.25rem;
  opacity: 0.6;
  transition: all 0.2s;
  position: relative;  /* 添加 position */
  z-index: 2;  /* 确保在 nav-items 上层 */
}

.nav-btn:hover,
.nav-btn.active {
  background: rgba(255, 255, 255, 0.1);
  opacity: 1;
}

/* 主内容区 */
.content-area {
  flex: 1;
  height: 100%;
  min-width: 0;
  overflow: hidden;
  position: relative;  /* 添加 position */
  z-index: 1;
}
</style>