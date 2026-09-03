<template>
  <div class="main-layout">
    <nav class="side-nav">
      <div class="logo-area">
        <div class="logo" title="Symbio">S</div>
        <span class="logo-text">Symbio</span>
      </div>

      <div class="nav-items">
        <button
          class="nav-btn"
          :class="{ active: currentPage === 'session' }"
          aria-label="会话"
          title="会话"
          @click="goTo('/')"
        >
          <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z" />
          </svg>
          <span class="nav-label">会话</span>
        </button>

        <div class="nav-group-title">资源</div>

        <button
          class="nav-btn"
          :class="{ active: currentPage === 'agent' }"
          aria-label="Agent"
          title="Agent"
          @click="goTo('/agent')"
        >
          <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="8" r="4" />
            <path d="M6 21v-1a6 6 0 0 1 12 0v1" />
          </svg>
          <span class="nav-label">Agent</span>
        </button>
        <button
          class="nav-btn"
          :class="{ active: currentPage === 'skill' }"
          aria-label="Skill"
          title="Skill"
          @click="goTo('/skill')"
        >
          <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <polygon points="12 2 15.09 8.26 22 9.27 17 14.14 18.18 21.02 12 17.77 5.82 21.02 7 14.14 2 9.27 8.91 8.26 12 2" />
          </svg>
          <span class="nav-label">Skill</span>
        </button>
        <button
          class="nav-btn"
          :class="{ active: currentPage === 'mcp' }"
          aria-label="MCP Server"
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
          <span class="nav-label">MCP Server</span>
        </button>
        <button
          class="nav-btn"
          :class="{ active: currentPage === 'model-providers' }"
          aria-label="Model Provider"
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
          <span class="nav-label">Model Provider</span>
        </button>
      </div>

      <!-- 底部：设置 + 系统目录切换（Homedir Switcher） -->
      <div class="nav-footer">
        <button
          class="nav-btn"
          :class="{ active: currentPage === 'settings' }"
          aria-label="设置"
          title="设置"
          @click="goTo('/settings')"
        >
          <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="3" />
            <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-4 0v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1 0-4h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 4 0v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 0 4h-.09a1.65 1.65 0 0 0-1.51 1z" />
          </svg>
          <span class="nav-label">设置</span>
        </button>
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
          <span class="nav-label">系统目录</span>
        </button>
      </div>
    </nav>
    <main class="main-content">
      <RouterView />
    </main>
    <!-- 文件查看器全屏覆盖：覆盖整个 MainLayout -->
    <FileViewerOverlay />
    <!-- 全局浮动消息浮层（仅渲染一次，状态来自 useToast 单例）-->
    <Toast />
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
import Toast from '@/components/common/Toast.vue'

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
  background: var(--surface-panel);
  border-right: 1px solid var(--border-default);
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
  z-index: 10;
}

.logo-area {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 0.5rem;
  padding: 0.75rem 0;
  border-bottom: 1px solid var(--border-default);
}

.logo {
  width: 2rem;
  height: 2rem;
  display: flex;
  align-items: center;
  justify-content: center;
  font-weight: 700;
  font-size: 1rem;
  color: var(--text-on-accent);
  background: var(--accent);
  border-radius: var(--radius-md);
  flex-shrink: 0;
}

.logo-text {
  font-size: var(--font-size-md);
  font-weight: 600;
  color: var(--text-primary);
  white-space: nowrap;
  display: none;
}

.nav-items {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.25rem;
  flex: 1;
  padding: 0.5rem 0.375rem;
  overflow-y: auto;
}

/* 窄条模式下分组标题放不下文字，退化为一条 hairline 分隔线。
   align-self: stretch 保证在 align-items:center 的纵列里仍能横跨取到宽度。 */
.nav-group-title {
  height: 1px;
  margin: 0.5rem 0.75rem;
  padding: 0;
  background: var(--border-default);
  font-size: 0;
  line-height: 0;
  overflow: hidden;
  flex-shrink: 0;
  align-self: stretch;
}

.nav-footer {
  width: 100%;
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  padding: 0.5rem;
  border-top: 1px solid var(--border-default);
}

.nav-btn {
  width: 2.5rem;
  height: 2.5rem;
  border: none;
  background: transparent;
  border-radius: var(--radius-md);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  align-self: center;
  flex-shrink: 0;
  gap: 0.75rem;
  padding: 0;
  color: var(--text-secondary);
  font-size: var(--font-size-base);
  transition: background-color var(--motion-fast) var(--motion-ease),
    color var(--motion-fast) var(--motion-ease);
}

.nav-btn:hover {
  background: var(--surface-hover);
  color: var(--text-primary);
}

.nav-btn.active {
  background: var(--surface-selected);
  color: var(--accent);
}

/* 窄条模式隐藏文字标签（保留 DOM，便于日后悬浮展开）；
   可访问名称由 aria-label 提供，键盘焦点环由 base.css 的 :focus-visible 保证。 */
.nav-label {
  display: none;
}

.main-content {
  flex: 1;
  min-width: 0;
  min-height: 0;
  overflow: hidden;
  display: flex;
}
</style>
