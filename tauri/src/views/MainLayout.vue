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
          :class="{ active: isAllResourcesActive }"
          aria-label="全部资源"
          title="全部资源"
          @click="goTo('/resources/all')"
        >
          <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
          </svg>
          <span class="nav-label">全部资源</span>
        </button>
        <!-- 资源导航项由后端 resources/providers 注册表（nav='resources'）动态生成 -->
        <button
          v-for="p in resourceNav"
          :key="p.kind"
          class="nav-btn"
          :class="{ active: isProviderNavActive(p.kind) }"
          :aria-label="p.label"
          :title="p.label"
          @click="goTo(`/resources/${p.kind}`)"
        >
          <component :is="navIconFor(p.kind)" v-if="navIconFor(p.kind)" />
          <svg v-else viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M13 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V9z" />
            <polyline points="13 2 13 9 20 9" />
          </svg>
          <span class="nav-label">{{ p.label }}</span>
        </button>
      </div>

      <!-- 底部：设置入口（ProviderInfo.nav='settings' 动态生成，不再硬编码）
            + 系统目录切换（Homedir Switcher） -->
      <div class="nav-footer">
        <button
          v-for="p in settingsNav"
          :key="p.kind"
          class="nav-btn"
          :class="{ active: currentPage === 'settings' }"
          :aria-label="p.label"
          :title="p.label"
          @click="goTo('/settings')"
        >
          <component :is="navIconFor(p.kind)" v-if="navIconFor(p.kind)" />
          <svg v-else viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <circle cx="12" cy="12" r="3" />
            <path d="" />
          </svg>
          <span class="nav-label">{{ p.label }}</span>
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

/** 左侧导航项：全部按 ProviderInfo.nav 分组——资源区（nav='resources'）与设置区
 * （nav='settings'）均由注册表动态生成，前端不再写死"设置"入口 */
const { resourceNav, settingsNav } = useResourceProviders()

/** 当前单类型资源路径 */
const resTypes = computed(() => (route.params.types as string) || 'all')
/** "全部资源"高亮：all 或逗号分隔（多类型混合） */
const isAllResourcesActive = computed(
  () => resTypes.value === 'all' || resTypes.value.includes(',')
)
/** 单类型资源导航项高亮：路由命中该 kind */
function isProviderNavActive(kind: string): boolean {
  return (
    route.path === `/resources/${kind}` ||
    route.path.startsWith(`/resources/${kind},`) ||
    resTypes.value === kind
  )
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

const currentPage = computed(() => {
  if (route.path.startsWith('/resources')) return 'resources'
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
