<template>
  <div class="session-view">
    <SessionListPanel class="col-left" />
    <ChatMainPanel class="col-middle" />
    <SessionExplorerPanel class="col-right" />
  </div>
</template>

<script setup lang="ts">
import { onMounted } from 'vue'
import { useSessionsStore } from '@/stores/sessions'
import { getWorkspacePath } from '@/services/home'
import SessionListPanel from '@/components/session/SessionListPanel.vue'
import ChatMainPanel from '@/components/session/ChatMainPanel.vue'
import SessionExplorerPanel from '@/components/session/SessionExplorerPanel.vue'

const store = useSessionsStore()

onMounted(async () => {
  // 1. 先从后端恢复全局工作目录（用户上次选过的工作区）。
  //    前端 getGlobalWorkdir 是内存值，这里把它从后端 `work/get_workspace` 灌回，
  //    使无 workdir 的旧会话能自动回填、不阻塞在选择工作目录引导。
  try {
    await getWorkspacePath()
  } catch (e) {
    console.warn('[SessionView] 恢复全局工作区失败', e)
  }

  // 2. 加载会话列表
  await store.refreshList()

  if (store.list.length === 0) {
    // 加载失败（error 非空）：不要误建空会话，否则会用无目录空会话反复触发选目录引导；
    // 停在"未选择会话"，由用户手动刷新。
    if (store.error) return
    await store.createSession()
  } else if (!store.activeId) {
    store.selectSession(store.list[0].id)
  } else {
    store.selectSession(store.activeId)
  }
})
</script>

<style scoped>
.session-view {
  display: flex;
  width: 100%;
  height: 100%;
  min-height: 0;
  background: var(--color-bg);
}

.col-left {
  flex: 0 0 16.25rem;
  min-width: 12.5rem;
  max-width: 22.5rem;
}

.col-middle {
  flex: 1 1 auto;
  min-width: 0;
}

.col-right {
  flex: 0 0 17.5rem;
  min-width: 12.5rem;
  max-width: 26.25rem;
}
</style>
