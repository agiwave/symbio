<template>
  <div class="empty-workdir">
    <div class="empty-illustration">
      <div class="folder-icon">
        <svg viewBox="0 0 64 64" width="80" height="80" fill="none" stroke="currentColor" stroke-width="3" stroke-linejoin="round">
          <path d="M6 18 L6 50 L58 50 L58 22 L34 22 L28 16 L6 16 Z" />
        </svg>
      </div>
    </div>
    <h2 class="empty-title">未选择工作目录</h2>
    <p class="empty-desc">
      为当前会话绑定一个项目目录后，AI 才能在正确的上下文中回答，右栏资源浏览器才能加载。
    </p>
    <button class="primary-btn" @click="onPick" :disabled="picking">
      {{ picking ? '选择中…' : '选择工作目录' }}
    </button>
    <button v-if="store.lastUsedWorkdir" class="secondary-btn" @click="onUseLast">
      使用最近的工作目录
    </button>
    <p v-if="lastError" class="error-text">{{ lastError }}</p>
  </div>
</template>

<script setup lang="ts">
import { ref } from 'vue'
import { useSessionsStore } from '@/stores/sessions'
import { open } from '@tauri-apps/plugin-dialog'

const store = useSessionsStore()
const picking = ref(false)
const lastError = ref('')

async function onPick() {
  if (!store.activeId) {
    lastError.value = '请先选择或创建一个会话'
    return
  }
  picking.value = true
  lastError.value = ''
  try {
    const selected = await open({
      directory: true,
      multiple: false,
      title: '选择工作目录'
    })
    if (!selected) return
    const path = typeof selected === 'string' ? selected : Array.isArray(selected) ? selected[0] : null
    if (!path) return
    // 资源浏览器的重置和重载由 SessionExplorerPanel 监听 activeWorkdir 自动处理
    await store.setActiveWorkdir(path)
  } catch (e) {
    lastError.value = e instanceof Error ? e.message : String(e)
  } finally {
    picking.value = false
  }
}

async function onUseLast() {
  if (!store.lastUsedWorkdir) return
  await store.setActiveWorkdir(store.lastUsedWorkdir)
}
</script>

<style scoped>
.empty-workdir {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  text-align: center;
  padding: 3rem 1.5rem;
  height: 100%;
  color: var(--color-text-secondary);
}

.empty-illustration {
  margin-bottom: 1.5rem;
  color: var(--color-text-muted);
  opacity: 0.6;
}

.empty-title {
  font-size: 1.15rem;
  font-weight: 500;
  color: var(--color-text);
  margin-bottom: 0.5rem;
}

.empty-desc {
  max-width: 360px;
  font-size: 0.85rem;
  color: var(--color-text-muted);
  margin-bottom: 1.5rem;
  line-height: 1.6;
}

.primary-btn {
  padding: 0.6rem 1.2rem;
  background: var(--color-primary);
  color: #fff;
  border: none;
  border-radius: 8px;
  font-size: 0.9rem;
  cursor: pointer;
  margin-bottom: 0.5rem;
  transition: opacity 0.15s;
}

.primary-btn:hover { opacity: 0.9; }
.primary-btn:disabled { opacity: 0.5; cursor: not-allowed; }

.secondary-btn {
  padding: 0.5rem 1rem;
  background: transparent;
  border: 1px solid var(--color-border);
  border-radius: 8px;
  color: var(--color-text-secondary);
  font-size: 0.85rem;
  cursor: pointer;
}

.secondary-btn:hover {
  background: rgba(0, 0, 0, 0.04);
}

.error-text {
  margin-top: 1rem;
  color: #ef4444;
  font-size: 0.8rem;
}
</style>
