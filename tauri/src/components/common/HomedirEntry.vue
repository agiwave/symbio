<!--
  HomedirEntry — 系统目录入口（NavRail 左下角）：按钮 + 切换对话框 + 连接显示

  从应用外壳抽出的自足组件：持有系统目录（本地 / 远端）的显示状态与切换
  对话框；切换成功后 emit `reloaded`，由宿主页面做数据侧重载（如刷新会话
  清单、回到首页）。谁嵌入谁拥有——不再依赖应用外壳。
-->
<template>
  <div class="nav-footer">
    <button
      class="nav-btn"
      :class="{ 'nav-btn--error': locationError }"
      :title="homedirTitle"
      aria-label="系统目录"
      @click="open = true"
    >
      <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7z" />
        <circle cx="17" cy="13" r="2" />
      </svg>
      <span class="nav-label">系统目录</span>
      <span v-if="locationError" class="nav-dot" />
    </button>
  </div>

  <HomedirSwitcher v-model:open="open" @reloaded="onReloaded" />
</template>

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { getHomedirInfo } from '@/services/home'
import {
  loadSystemLocation,
  currentLocation,
  locationError,
  formatLocation,
} from '@/services/systemLocation'
import { useToast } from '@/composables/useToast'
import { logger } from '@/utils/logger'
import HomedirSwitcher from '@/components/common/HomedirSwitcher.vue'

const emit = defineEmits<{
  /** 系统目录切换成功（数据侧已重载连接目标）；宿主据此刷新自身数据 */
  (e: 'reloaded'): void
}>()

const { showToast } = useToast()
const open = ref(false)

onMounted(async () => {
  // 本地模式下异步加载 homedir 显示（不阻塞首屏）；远端模式由系统目录状态直接展示
  if (currentLocation.value.kind === 'local') {
    try {
      const info = await getHomedirInfo()
      if (info.homedir) {
        currentLocation.value = { ...currentLocation.value, localPath: info.homedir }
      }
    } catch (err) {
      logger.warn('HomedirEntry', '加载 homedir 显示失败:', err)
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

async function onReloaded() {
  // 切换成功后，从持久化同步当前连接目标显示
  const loc = loadSystemLocation()
  currentLocation.value = loc
  if (loc.kind === 'local') {
    try {
      const info = await getHomedirInfo()
      if (info.homedir) currentLocation.value = { ...loc, localPath: info.homedir }
    } catch (err) {
      logger.warn('HomedirEntry', '刷新 homedir 显示失败:', err)
    }
  }
  emit('reloaded')
}
</script>

<style scoped>
.nav-footer {
  width: 100%;
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
  padding: 0.5rem;
  border-top: 1px solid var(--border-default);
}

.nav-btn {
  position: relative;
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
</style>
