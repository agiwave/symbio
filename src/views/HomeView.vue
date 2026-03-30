<template>
  <div class="home-view">
    <!-- 顶层导航条 (始终显示) -->
    <nav class="nav-bar">
      <div class="nav-logo" @click="currentPage = 'workspace'">🌊</div>
      <div class="nav-items">
        <button 
          class="nav-btn" 
          :class="{ active: currentPage === 'workspace' }"
          title="工作区" 
          @click="currentPage = 'workspace'"
        >
          📁
        </button>
        <button 
          class="nav-btn" 
          :class="{ active: currentPage === 'agent' }"
          title="AI 交互" 
          @click="currentPage = 'agent'"
        >
          💬
        </button>
        <button 
          class="nav-btn" 
          :class="{ active: currentPage === 'settings' }"
          title="设置" 
          @click="currentPage = 'settings'"
        >
          ⚙️
        </button>
      </div>
    </nav>

    <!-- 主内容区 -->
    <div class="content-area">
      <!-- 工作区页面 -->
      <WorkspacePage v-show="currentPage === 'workspace'" />
      
      <!-- AI 交互页面 -->
      <AgentPage v-show="currentPage === 'agent'" />
      
      <!-- 设置页面 -->
      <SettingsPage v-show="currentPage === 'settings'" />
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref } from 'vue'
import WorkspacePage from '../components/WorkspacePage.vue'
import AgentPage from '../components/AgentPage.vue'
import SettingsPage from '../components/SettingsPage.vue'

// 当前页面
const currentPage = ref<'workspace' | 'agent' | 'settings'>('workspace')
</script>

<style scoped>
.home-view {
  display: flex;
  height: 100%;
  width: 100%;
  background: var(--color-bg);
}

/* 导航条 (始终显示) */
.nav-bar {
  width: var(--sidebar-width, 56px);
  background: #1a1a2e;
  display: flex;
  flex-direction: column;
  align-items: center;
  padding: 1rem 0;
  flex-shrink: 0;
  z-index: 10;
}

.nav-logo {
  font-size: 1.5rem;
  cursor: pointer;
  margin-bottom: 2rem;
}

.nav-items {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
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
}

.nav-btn:hover,
.nav-btn.active {
  background: rgba(255, 255, 255, 0.1);
  opacity: 1;
}

/* 主内容区 */
.content-area {
  flex: 1;
  display: flex;
  height: 100%;
  min-width: 0;
  overflow: hidden;
}
</style>