<template>
  <div class="main-layout">
    <!-- 应用外壳 = 三栏工作台（统一 Workbench 容器，应用外壳模式）：
         侧边栏 items 来自 `.vdfs` 虚拟根（挂载点清单，后端 order 排列、不分组，
         路由恒为 /vdfs/{mount}），
         工作区 = RouterView（VDFS 页嵌入其中，与本外壳共用同一台三栏工作台）。 -->
    <Workbench :rail-items="navItems" @rail-select="onNavSelect">
      <template #rail-header>
        <div class="logo-area">
          <div class="logo" title="Symbio">S</div>
          <span class="logo-text">Symbio</span>
        </div>
      </template>

      <!-- 底部：系统目录切换（Homedir Switcher，系统工具非实体，独立于实体导航） -->
      <template #rail-footer>
        <div class="nav-footer">
          <button
            class="nav-btn"
            :class="{ 'nav-btn--error': locationError }"
            :title="homedirTitle"
            @click="openHomedirSwitcher = true"
            aria-label="系统目录"
          >
            <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
              <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7z" />
              <circle cx="17" cy="13" r="2" />
            </svg>
            <span class="nav-label">系统目录</span>
            <span v-if="locationError" class="nav-dot" />
          </button>
        </div>
      </template>

      <template #content>
        <!-- :key 强制路由切换时重建组件：各实体页共用统一 WorkbenchView，
             若复用实例则 onMounted/订阅不会重新执行，列表会残留上一个类型的数据 -->
        <RouterView :key="route.path" />
      </template>
    </Workbench>
    <!-- 文件查看器全屏覆盖：覆盖整个 MainLayout -->
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
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter, RouterView } from 'vue-router'
import { startSessionBusWatcher } from '@/services/sessionBusWatcher'
import { getHomedirInfo, getWorkspacePath } from '@/services/home'
import { loadMounts, reloadMounts, useNavRailItems } from '@/composables/useNavRail'
import { useSessionsStore } from '@/stores/sessions'
import {
  loadSystemLocation,
  currentLocation,
  locationError,
  formatLocation,
} from '@/services/systemLocation'
import { useToast } from '@/composables/useToast'
import { logger } from '@/utils/logger'
import HomedirSwitcher from '@/components/common/HomedirSwitcher.vue'
import Workbench from '@/components/common/Workbench.vue'
import Toast from '@/components/common/Toast.vue'

const route = useRoute()
const router = useRouter()
const { showToast } = useToast()

const openHomedirSwitcher = ref(false)

/** 主导航项：`.vdfs` 虚拟根内容（挂载点清单，后端 order 顺序），不含任何特殊分组。
 *  机制内唯一实现（useNavRailItems）：`.vdfs` 根 → NavRail 项，
 *  路由约定见 navTargetOf（恒为 `/vdfs/{mount}`）——MainLayout 不再持有任何导航特判。 */
const { navItems, navTargetOf } = useNavRailItems()

function onNavSelect(mount: string) {
  goTo(navTargetOf(mount))
}

onMounted(async () => {
  // 启动全局会话事件监听（一次即可，跨页面共享）
  // 这一步必须在会话页（WorkbenchView session 实例）挂载之前，否则首屏会错过事件
  startSessionBusWatcher()

  // 恢复全局会话状态（原 SessionView 职责，随会话页纳入机制上移到应用外壳）：
  // lastWorkdir 供新建会话作默认工作区；sessions store 清单供聊天组件查元数据
  try {
    await getWorkspacePath()
  } catch (err) {
    logger.warn('MainLayout', '恢复全局工作区失败:', err)
  }
  void useSessionsStore().refreshList()

  // 拉取 `.vdfs` 虚拟根（挂载点清单）→ 动态生成左侧导航（幂等）。
  // 实体页（WorkbenchView）自持实体注册表的加载，外壳不再代劳。
  try {
    await loadMounts()
  } catch (err) {
    logger.warn('MainLayout', '加载 VDFS 挂载点清单失败:', err)
  }

  // 本地模式下异步加载 homedir 显示（不阻塞首屏）；远端模式由系统目录状态直接展示
  if (currentLocation.value.kind === 'local') {
    try {
      const info = await getHomedirInfo()
      if (info.homedir) {
        currentLocation.value = { ...currentLocation.value, localPath: info.homedir }
      }
    } catch (err) {
      logger.warn('MainLayout', '加载 homedir 显示失败:', err)
    }
  }

  // 启动期若远端不可达已回退本地，给出提示
  if (locationError.value) {
    showToast('error', locationError.value)
  }
})

// 远端不可达 / 连接异常时提示用户（便于从系统目录按钮切回/修正）
watch(locationError, (msg) => {
  if (msg) showToast('error', msg)
})

const homedirTitle = computed(() => {
  const loc = currentLocation.value
  const label = loc.kind === 'remote' ? `远端 ${formatLocation(loc)}` : `本地 ${formatLocation(loc)}`
  return locationError.value ? `${label}（连接异常，已回退本地）` : `${label}（点击切换）`
})

function goTo(path: string) {
  if (route.path !== path) {
    router.push(path)
  }
}

async function onHomedirReloaded() {
  // 切换成功后，从持久化同步当前连接目标显示
  const loc = loadSystemLocation()
  currentLocation.value = loc
  if (loc.kind === 'local') {
    try {
      const info = await getHomedirInfo()
      if (info.homedir) currentLocation.value = { ...loc, localPath: info.homedir }
    } catch (err) {
      logger.warn('MainLayout', '刷新 homedir 显示失败:', err)
    }
  }
  // 重新拉取导航与会话清单（远端模式下数据来自远端实例）
  try {
    await reloadMounts()
  } catch (err) {
    logger.warn('MainLayout', '切换后加载挂载点清单失败:', err)
  }
  void useSessionsStore().refreshList()
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

/* logo 区（NavRail header 插槽内容，父作用域样式） */
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

.nav-footer {
  width: 100%;
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  padding: 0.5rem;
  border-top: 1px solid var(--border-default);
}

.nav-btn--error {
  color: var(--danger-fg, #e5484d);
}

.nav-dot {
  position: absolute;
  top: 0.35rem;
  right: 0.5rem;
  width: 0.5rem;
  height: 0.5rem;
  border-radius: 50%;
  background: var(--danger-fg, #e5484d);
}

.nav-btn {
  position: relative;
}
</style>
