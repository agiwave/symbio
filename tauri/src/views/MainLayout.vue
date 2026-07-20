<template>
  <div class="main-layout">
    <nav class="side-nav">
      <div class="logo-area">
        <div class="logo" title="Symbio">S</div>
      </div>
      <div class="nav-items">
        <button
          class="nav-btn"
          :class="{ active: currentPage === 'session' }"
          title="会话"
          @click="goTo('/')"
        >
          <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
          </svg>
        </button>
        <button
          class="nav-btn"
          :class="{ active: currentPage === 'model-providers' }"
          title="Model Provider"
          @click="goTo('/model-providers')"
        >
          <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="4" y="6" width="16" height="12" rx="2" />
            <circle cx="9" cy="12" r="1.5" fill="currentColor" />
            <circle cx="15" cy="12" r="1.5" fill="currentColor" />
            <path d="M12 3v3" />
            <path d="M9 18v3" />
            <path d="M15 18v3" />
          </svg>
        </button>
        <button
          class="nav-btn"
          :class="{ active: currentPage === 'agent' }"
          title="Agent"
          @click="goTo('/agent')"
        >
          <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="8" r="4" />
            <path d="M6 21v-1a6 6 0 0 1 12 0v1" />
          </svg>
        </button>
        <button
          class="nav-btn"
          :class="{ active: currentPage === 'mcp' }"
          title="MCP Server"
          @click="goTo('/mcp')"
        >
          <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <rect x="3" y="3" width="7" height="7" rx="1" />
            <rect x="14" y="3" width="7" height="7" rx="1" />
            <rect x="3" y="14" width="7" height="7" rx="1" />
            <rect x="14" y="14" width="7" height="7" rx="1" />
            <line x1="10" y1="6.5" x2="14" y2="6.5" />
            <line x1="6.5" y1="10" x2="6.5" y2="14" />
            <line x1="17.5" y1="10" x2="17.5" y2="14" />
            <line x1="10" y1="17.5" x2="14" y2="17.5" />
          </svg>
        </button>
        <button
          class="nav-btn"
          :class="{ active: currentPage === 'skill' }"
          title="Skill"
          @click="goTo('/skill')"
        >
          <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2" />
          </svg>
        </button>
        <button
          class="nav-btn"
          :class="{ active: currentPage === 'settings' }"
          title="设置"
          @click="goTo('/settings')"
        >
          <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="3" />
            <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06A1.65 1.65 0 0 0 4.6 15a1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06A1.65 1.65 0 0 0 9 4.6a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09A1.65 1.65 0 0 0 15 4.6a1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06A1.65 1.65 0 0 0 -.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
          </svg>
        </button>
      </div>
      <!-- 底部：系统目录切换图标 (Homedir Switcher) -->
      <div class="nav-footer">
        <button
          class="nav-btn"
          :title="homedirTitle"
          @click="openHomedirSwitcher = true"
          aria-label="系统目录"
        >
          <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7z" />
            <circle cx="17" cy="13" r="2" />
          </svg>
        </button>
      </div>
    </nav>
    <main class="main-content">
      <RouterView />
    </main>
    <!-- 文件查看器全屏覆盖：覆盖整个 MainLayout -->
    <FileViewerOverlay />
    <!-- 系统目录切换对话框 -->
    <HomedirSwitcher
      v-model:open="openHomedirSwitcher"
      @reloaded="onHomedirReloaded"
    />
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { useRoute, useRouter, RouterView } from 'vue-router'
import { startSessionBusWatcher } from '@/services/sessionBusWatcher'
import { getHomedirInfo, type HomedirInfo } from '@/services/home'
import { logger } from '@/utils/logger'
import FileViewerOverlay from '@/components/fileViewer/FileViewerOverlay.vue'
import HomedirSwitcher from '@/components/common/HomedirSwitcher.vue'

const route = useRoute()
const router = useRouter()

const openHomedirSwitcher = ref(false)
const currentHomedir = ref<HomedirInfo>({ homedir: '', bootstrap_path: '' })

onMounted(async () => {
  // 启动全局会话事件监听（一次即可，跨页面共享）
  // 这一步必须在 SessionView 挂载之前，否则首屏会错过一些事件
  startSessionBusWatcher()

  // 异步加载 homedir 显示（不阻塞首屏）
  try {
    currentHomedir.value = await getHomedirInfo()
  } catch (err) {
    logger.warn('MainLayout', '加载 homedir 显示失败:', err)
  }
})

const currentPage = computed(() => {
  if (route.path.startsWith('/model-providers')) return 'model-providers'
  if (route.path.startsWith('/mcp')) return 'mcp'
  if (route.path.startsWith('/skill')) return 'skill'
  if (route.path.startsWith('/agent')) return 'agent'
  if (route.path.startsWith('/settings')) return 'settings'
  return 'session'
})

const homedirTitle = computed(() => {
  if (currentHomedir.value.homedir) {
    return `系统目录: ${currentHomedir.value.homedir}（点击切换）`
  }
  return '系统目录（点击切换）'
})

function goTo(path: string) {
  if (route.path !== path) {
    router.push(path)
  }
}

async function onHomedirReloaded() {
  // 切换成功后，更新本地显示并跳转到首页让用户看到刷新效果
  try {
    currentHomedir.value = await getHomedirInfo()
  } catch (err) {
    logger.warn('MainLayout', '刷新 homedir 显示失败:', err)
  }
  // 跳到首页（避免停留在某个"已失效"的页面）
  if (route.path !== '/') {
    router.push('/')
  }
}
</script>

<style scoped>
.main-layout {
  display: flex;
  width: 100%;
  height: 100vh;
  overflow: hidden;
}

.side-nav {
  width: var(--sidebar-width);
  background: var(--color-surface);
  border-right: 1px solid var(--color-border);
  display: flex;
  flex-direction: column;
  align-items: center;
  flex-shrink: 0;
  z-index: 10;
}

.logo-area {
  padding: 0.75rem 0;
  display: flex;
  align-items: center;
  justify-content: center;
}

.logo {
  width: 32px;
  height: 32px;
  display: flex;
  align-items: center;
  justify-content: center;
  font-weight: 700;
  font-size: 1rem;
  color: #fff;
  background: linear-gradient(135deg, var(--color-primary), var(--color-primary-dark));
  border-radius: 8px;
}

.nav-items {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.4rem;
  flex: 1;
  padding: 1rem 0;
  width: 100%;
}

.nav-footer {
  width: 100%;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0.5rem 0 0.75rem 0;
  border-top: 1px solid var(--color-border);
}

.nav-btn {
  width: 36px;
  height: 36px;
  border: none;
  background: transparent;
  border-radius: 8px;
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--color-text-secondary);
  opacity: 0.65;
  transition: all 0.15s;
}

.nav-btn:hover {
  background: rgba(0, 0, 0, 0.06);
  opacity: 1;
}

.nav-btn.active {
  background: rgba(102, 126, 234, 0.15);
  color: var(--color-primary);
  opacity: 1;
}

.main-content {
  flex: 1;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  display: flex;
}
</style>
