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
import SessionListPanel from '@/components/session/SessionListPanel.vue'
import ChatMainPanel from '@/components/session/ChatMainPanel.vue'
import SessionExplorerPanel from '@/components/session/SessionExplorerPanel.vue'

const store = useSessionsStore()

onMounted(async () => {
  await store.refreshList()
  // 如果没有会话，自动创建一个
  if (store.list.length === 0) {
    await store.createSession()
  } else if (!store.activeId) {
    store.selectSession(store.list[0].id)
  } else {
    // 恢复 active 会话时，触发 watcher 加载 explorer
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
  flex: 0 0 260px;
  min-width: 200px;
  max-width: 360px;
}

.col-middle {
  flex: 1 1 auto;
  min-width: 0;
}

.col-right {
  flex: 0 0 280px;
  min-width: 200px;
  max-width: 420px;
}
</style>
