<!--
  HomedirEntry — 数据位置入口（工作台侧栏左下角）：按钮 + 切换对话框 + 连接显示

  从应用外壳抽出的自足组件：持有数据位置（本机 / 远端）的显示状态与切换
  对话框；切换成功后 emit `reloaded`，由宿主页面做数据侧重载（如刷新会话
  清单、回到首页）。谁嵌入谁拥有——不再依赖应用外壳。

  对用户只说「数据位置」，不暴露 homedir / Server 等实现术语。
-->
<template>
  <div class="nav-footer">
    <button
      class="nav-btn"
      :class="{ 'nav-btn--error': locationError }"
      :title="locationTitle"
      :aria-label="locationTitle"
      @click="open = true"
    >
      <svg viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
        <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7z" />
        <circle cx="17" cy="13" r="2" />
      </svg>
      <span class="nav-label" :title="locationLabel">{{ locationLabel }}</span>
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
  /** 数据位置切换成功（数据侧已重载连接目标）；宿主据此刷新自身数据 */
  (e: 'reloaded'): void
}>()

const { showToast } = useToast()
const open = ref(false)

onMounted(async () => {
  // 本地模式下异步加载当前数据位置（不阻塞首屏）；远端模式由状态直接展示
  if (currentLocation.value.kind === 'local') {
    try {
      const info = await getHomedirInfo()
      if (info.homedir) {
        currentLocation.value = { ...currentLocation.value, localPath: info.homedir }
      }
    } catch (err) {
      logger.warn('data-location', '加载当前位置失败:', err)
    }
  }

  // 启动期若远端不可达已回退本地，给出提示
  if (locationError.value) {
    showToast('error', locationError.value)
  }
})

// 远端不可达 / 连接异常时提示用户（便于从数据位置入口切回/修正）
watch(locationError, (msg) => {
  if (msg) showToast('error', msg)
})

/** 短标签：本机显示文件夹末段，远端显示主机地址；不再叠加「本地/远端」前缀 */
const locationLabel = computed(() => {
  const loc = currentLocation.value
  const raw = formatLocation(loc)
  if (loc.kind === 'remote') return raw
  const parts = raw.replace(/[\\/]+$/, '').split(/[\\/]/).filter(Boolean)
  return parts[parts.length - 1] || raw
})

const locationTitle = computed(() => {
  const loc = currentLocation.value
  const kind = loc.kind === 'remote' ? '远端服务' : '本机数据位置'
  const errorSuffix = '连接异常，点击检查'
  return locationError.value
    ? `${kind}：${formatLocation(loc)}（${errorSuffix}）`
    : `${kind}：${formatLocation(loc)}（点击切换）`
})

async function onReloaded() {
  // 切换成功后，从持久化同步当前位置显示
  const loc = loadSystemLocation()
  currentLocation.value = loc
  if (loc.kind === 'local') {
    try {
      const info = await getHomedirInfo()
      if (info.homedir) currentLocation.value = { ...loc, localPath: info.homedir }
    } catch (err) {
      logger.warn('data-location', '刷新当前位置失败:', err)
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
  padding: 0.25rem;
  border-top: 1px solid var(--border-default);
}

.nav-btn {
  position: relative;
  display: flex;
  align-items: center;
  gap: 0.5rem;
  width: 100%;
  min-width: 0;
}

.nav-btn--error {
  color: var(--danger-fg, #e5484d);
}

.nav-label {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  text-align: left;
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

@media (pointer: coarse) {
  .nav-footer {
    padding: 0.125rem;
  }
}
</style>
