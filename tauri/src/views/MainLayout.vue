<template>
  <div class="main-layout">
    <!-- 应用外壳 = 三栏工作台（统一 Workbench 容器，应用外壳模式）：
         侧边栏 items 来自后端 providers 注册表（order 排列、不分组，
         会话→/、设置→/settings，其余→/entities/{kind}），工作区 = RouterView。 -->
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
      </template>

      <template #content>
        <!-- :key 强制路由切换时重建组件：各实体页共用统一 WorkbenchView，
             若复用实例则 onMounted/订阅不会重新执行，列表会残留上一个类型的数据 -->
        <RouterView :key="route.path" />
      </template>
    </Workbench>
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
import { loadProviders, useNavRailItems } from '@/composables/useEntityProviders'
import { useSessionsStore } from '@/stores/sessions'
import { getWorkspacePath } from '@/services/home'
import { logger } from '@/utils/logger'
import FileViewerOverlay from '@/components/fileViewer/FileViewerOverlay.vue'
import HomedirSwitcher from '@/components/common/HomedirSwitcher.vue'
import Workbench from '@/components/common/Workbench.vue'
import Toast from '@/components/common/Toast.vue'

const route = useRoute()
const router = useRouter()

const openHomedirSwitcher = ref(false)
const currentHomedir = ref<HomedirInfo>({ homedir: '', bootstrap_path: '' })

/** 主导航项：全部已注册 provider（后端 order 顺序），不含任何特殊分组。
 *  机制内唯一实现（useNavRailItems）：providers 注册表 → NavRail 项，
 *  路由约定见 navTargetOf——MainLayout 不再持有任何导航特判逻辑。 */
const { navItems, navTargetOf } = useNavRailItems()

function onNavSelect(kind: string) {
  goTo(navTargetOf(kind))
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

  // 拉取后端实体 provider 注册表（动态生成左侧导航；幂等）
  try {
    await loadProviders()
  } catch (err) {
    logger.warn('MainLayout', '加载实体 provider 注册表失败:', err)
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
</style>
