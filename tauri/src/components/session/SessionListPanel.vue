<template>
  <aside class="session-list-panel">
    <header class="panel-header">
      <h3 class="panel-title">会话</h3>
      <div class="header-actions">
        <button class="icon-btn" :title="creating ? '创建中…' : '新建会话'" @click="onCreate">
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2">
            <line x1="12" y1="5" x2="12" y2="19" />
            <line x1="5" y1="12" x2="19" y2="12" />
          </svg>
        </button>
      </div>
    </header>

    <div class="list-meta" v-if="store.runningCount > 0">
      <span class="running-pulse" /> 运行中 {{ store.runningCount }} 个
    </div>

    <div class="session-list" role="listbox" aria-label="会话列表" v-if="store.list.length">
      <SessionCard
        v-for="s in store.list"
        :key="s.id"
        :session="s"
        :is-active="s.id === store.activeId"
        @click="store.selectSession(s.id)"
        @delete="onDelete(s.id)"
      />
    </div>

    <div class="empty-state" v-else>
      <p>暂无会话</p>
      <p class="hint">点击 + 创建新会话</p>
    </div>
  </aside>
</template>

<script setup lang="ts">
import { ref } from 'vue'
import { useSessionsStore } from '@/stores/sessions'
import { open } from '@tauri-apps/plugin-dialog'
import { useExplorerStore } from '@/stores/explorer'
import { logger } from '@/utils/logger'
import SessionCard from './SessionCard.vue'

const store = useSessionsStore()
const explorer = useExplorerStore()
const creating = ref(false)

async function pickWorkdir(): Promise<string | null> {
  try {
    const selected = await open({
      directory: true,
      multiple: false,
      title: '选择工作目录'
    })
    if (!selected) return null
    return typeof selected === 'string' ? selected : Array.isArray(selected) ? selected[0] || null : null
  } catch (e) {
    logger.error('SessionListPanel', '选择工作目录失败', e)
    return null
  }
}

async function onCreate() {
  if (creating.value) return
  creating.value = true
  try {
    let workdir = store.lastUsedWorkdir
    if (!workdir) {
      workdir = await pickWorkdir()
      if (!workdir) return
    }
    await store.createSession(workdir)
    explorer.reset()
  } finally {
    creating.value = false
  }
}

async function onDelete(id: string) {
  const s = store.list.find(it => it.id === id)
  if (!s) return
  if (s.is_working) {
    alert('请先停止当前运行的会话后再删除')
    return
  }
  if (!confirm('确定删除该会话？此操作不可恢复。')) return
  try {
    await store.deleteSession(id)
  } catch (e) {
    logger.error('SessionListPanel', '删除失败', e)
    alert('删除失败：' + (e instanceof Error ? e.message : e))
  }
}
</script>

<style scoped>
.session-list-panel {
  display: flex;
  flex-direction: column;
  width: 100%;
  height: 100%;
  background: var(--color-bg);
  border-right: 1px solid var(--color-border);
  overflow: hidden;
}

.panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.5rem 0.75rem;
  border-bottom: 1px solid var(--color-border);
  flex-shrink: 0;
}

.panel-title {
  font-size: 0.85rem;
  font-weight: 600;
  color: var(--color-text-secondary);
  margin: 0;
}

.icon-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 1.625rem;
  height: 1.625rem;
  border: none;
  background: transparent;
  border-radius: 0.375rem;
  cursor: pointer;
  color: var(--color-text-secondary);
  transition: all 0.15s;
}

.icon-btn:hover {
  background: rgba(0, 0, 0, 0.06);
  color: var(--color-text);
}

.list-meta {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  font-size: 0.7rem;
  color: var(--color-text-muted);
  padding: 0.4rem 0.75rem;
  border-bottom: 1px solid var(--color-border);
  background: rgba(34, 197, 94, 0.04);
  flex-shrink: 0;
}

.running-pulse {
  display: inline-block;
  width: 0.4375rem;
  height: 0.4375rem;
  background: #22c55e;
  border-radius: 50%;
  animation: pulse 1.4s ease-in-out infinite;
}

@keyframes pulse {
  0%, 100% { opacity: 1; transform: scale(1); }
  50% { opacity: 0.5; transform: scale(0.85); }
}

.session-list {
  flex: 1;
  overflow-y: auto;
  padding: 0.25rem 0;
}

.empty-state {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  color: var(--color-text-muted);
  font-size: 0.85rem;
  gap: 0.3rem;
  padding: 1rem;
  text-align: center;
}

.empty-state .hint {
  font-size: 0.75rem;
  opacity: 0.7;
}
</style>
