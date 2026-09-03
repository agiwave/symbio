<template>
  <div class="main-layout">
    <nav class="side-nav">
      <div class="logo-area">
        <div class="logo" title="Symbio">S</div>
        <span class="logo-text">Symbio</span>
      </div>

      <div class="nav-items">
        <!-- 主导航：全部 provider 按后端注册表 order 顺序并排渲染，不分成"资源/会话/设置"多段。
             会话→/、设置→/settings，其余→/resources/{kind}。路由目标由 kind 决定，UI 无特殊位置。 -->
        <button
          v-for="p in providers"
          :key="p.kind"
          class="nav-btn"
          :class="{ active: isNavActive(p.kind) }"
          :aria-label="p.label"
          :title="p.label"
          @click="goTo(navTarget(p.kind))"
        >
          <component :is="navIconFor(p.kind)" v-if="navIconFor(p.kind)" />
          <svg v-else viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z" />
            <polyline points="13 2 13 9 20 9" />
          </svg>
          <span class="nav-label">{{ p.label }}</span>
        </button>
      </div>

      <!-- 底部：系统目录切换（Homedir Switcher，系统工具非资源，独立于资源导航） -->
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
          <span class="nav-label">系统目录</span>
        </button>
      </div>
    </nav>
    <main class="main-content">
      <!-- :key 强制路由切换时重建组件：四个资源页共用 ResourceManagerView，
           若复用实例则 onMounted/订阅不会重新执行，列表会残留上一个类型的数据 -->
      <RouterView :key="route.path" />
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
import { loadProviders, useResourceProviders } from '@/composables/useResourceProviders'
import { getResourceIcon } from '@/registry/resourceTypes'
import { logger } from '@/utils/logger'
import FileViewerOverlay from '@/components/fileViewer/FileViewerOverlay.vue'
import HomedirSwitcher from '@/components/common/HomedirSwitcher.vue'
import Toast from '@/components/common/Toast.vue'

const route = useRoute()
const router = useRouter()

const openHomedirSwitcher = ref(false)
const currentHomedir = ref<HomedirInfo>({ homedir: '', bootstrap_path: '' })

/** 主导航项：全部已注册 provider（后端 order 顺序），不含任何特殊分组。 */
const { providers } = useResourceProviders()

/** 单 kind 的导航路由目标：会话/设置走其专用入口，其余进统一资源页 */
function navTarget(kind: string): string {
  switch (kind) {
    case 'session':
      return '/'
    case 'setting':
      return '/settings'
    default:
      return `/resources/${kind}`
  }
}

/** 单 kind 导航项高亮 */
function isNavActive(kind: string): boolean {
  return route.path === navTarget(kind)
}

/** 类型图标：前端 icon 注册表（未注册回 null，走默认图标） */
function navIconFor(kind: string) {
  return getResourceIcon(kind) ?? null
}

onMounted(async () => {
  // 启动全局会话事件监听（一次即可，跨页面共享）
  // 这一步必须在 SessionView 挂载之前，否则首屏会错过一些事件
  startSessionBusWatcher()

  // 拉取后端资源 provider 注册表（动态生成左侧导航；幂等）
  try {
    await loadProviders()
  } catch (err) {
    logger.warn('MainLayout', '加载资源 provider 注册表失败:', err)
  }

  // 异步加载 homedir 显示（不阻塞首屏）
  try {
    currentHomedir.value = await getHomedirInfo()
  } catch (err) {
    logger.warn('MainLayout', '加载 homedir 显示失败:', err)
  }
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
