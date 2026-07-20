<!--
  AgentView — 智能体管理页（只读+删除）

  设计原则：
  - Agent 的 create 路由需要 cognition_units（认知单元）复杂结构，
    适合 seed 脚本批量创建，不适合 UI 表单
  - AgentView 只支持"列出/查看/删除"
-->
<template>
  <ResourceShell
    title="Agent"
    :loading="loading"
    :has-list-content="agents.length > 0"
    :hide-default-new="true"
  >
    <template #header-actions>
      <button
        class="icon-btn"
        title="刷新"
        :disabled="loading"
        @click="loadAll"
      >
        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M21 12a9 9 0 1 1-3-6.7L21 8" />
          <polyline points="21 3 21 8 16 8" />
        </svg>
      </button>
    </template>

    <template #meta v-if="agents.length > 0">
      <span class="running-pulse" />
      共 {{ agents.length }} 个智能体
    </template>

    <template #list>
      <div class="agent-list">
        <ResourceCard
          v-for="agent in agents"
          :key="agent.id"
          :title="agent.name || agent.id"
          :subtitle="agent.description || '（无描述）'"
          status="active"
          :is-active="selectedId === agent.id"
          @click="select(agent.id)"
        >
          <template #meta>
            <span class="tag tag-muted">{{ agent.id }}</span>
          </template>
        </ResourceCard>
      </div>
    </template>

    <template #empty>
      <p>暂无 Agent</p>
      <p class="hint">Agent 由 seed 脚本创建。请参考 <code>bin/seed_agents.rs</code></p>
    </template>

    <template #detail>
      <div v-if="selectedAgent" class="agent-detail">
        <header class="detail-header">
          <h2 class="detail-title">{{ selectedAgent.name || selectedAgent.id }}</h2>
        </header>
        <p class="detail-description">
          {{ selectedAgent.description || '（此智能体没有描述）' }}
        </p>
        <div class="detail-section">
          <label>ID</label>
          <code class="detail-code">{{ selectedAgent.id }}</code>
        </div>
        <div class="detail-section">
          <label>名称</label>
          <code class="detail-code">{{ selectedAgent.name }}</code>
        </div>
        <div class="detail-actions">
          <button
            class="btn btn-danger"
            :disabled="deletingId === selectedId"
            @click="handleDelete(selectedAgent)"
          >
            <span v-if="deletingId === selectedId">删除中…</span>
            <span v-else>删除智能体</span>
          </button>
        </div>
        <div class="detail-hint">
          <label class="muted">提示：智能体的创建需要 cognition_units（认知单元），建议使用 seed 脚本批量创建。</label>
        </div>
      </div>
      <div v-else class="no-selection">
        <p>← 选择一个 Agent 查看详情</p>
      </div>
    </template>

    <template #toast>
      <Transition name="toast">
        <div v-if="toast" :class="['toast', toast.type]" @click="toast = null">
          {{ toast.text }}
        </div>
      </Transition>
    </template>
  </ResourceShell>
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import { listAgents, deleteAgent } from '@/services/agents'
import { useResourceManager } from '@/composables/useResourceManager'
import type { AgentProfile } from '@/schemas/agents'
import ResourceShell from '../components/common/ResourceShell.vue'
import ResourceCard from '../components/common/ResourceCard.vue'

const {
  loading,
  selectedId,
  deletingId,
  toast,
  showToast,
  select,
  markDeleting,
} = useResourceManager({ logTag: 'AgentView' })

// === 状态 ===
const agents = ref<AgentProfile[]>([])

// === 计算属性 ===
const selectedAgent = computed(() => {
  if (!selectedId.value) return null
  return agents.value.find((a) => a.id === selectedId.value) ?? null
})

// === 方法 ===
async function loadAll() {
  loading.value = true
  try {
    agents.value = await listAgents()
    if (!selectedId.value && agents.value.length > 0) {
      select(agents.value[0].id)
    }
  } catch (err) {
    showToast('error', `加载失败: ${err}`)
  } finally {
    loading.value = false
  }
}

async function handleDelete(agent: AgentProfile) {
  if (!confirm(`确认删除 Agent 「${agent.name || agent.id}」？此操作不可恢复。`)) return
  markDeleting(agent.id)
  try {
    const result = await deleteAgent(agent.id)
    if (result.deleted) {
      showToast('success', `Agent ${agent.id} 已删除`)
      if (selectedId.value === agent.id) {
        selectedId.value = agents.value.find((a) => a.id !== agent.id)?.id ?? null
      }
      await loadAll()
    } else {
      showToast('info', `Agent ${agent.id} 不存在（幂等返回）`)
    }
  } catch (err) {
    showToast('error', `删除失败: ${err}`)
  } finally {
    markDeleting(null)
  }
}

onMounted(() => loadAll())
</script>

<style scoped>
.agent-list {
  flex: 1;
  overflow-y: auto;
  padding: 0.25rem 0;
}

.agent-detail {
  flex: 1;
  padding: 1.5rem 2rem;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: 1rem;
}

.detail-header {
  padding-bottom: 0.5rem;
  border-bottom: 1px solid var(--color-border, #e5e7eb);
}

.detail-title {
  font-size: 1.25rem;
  font-weight: 600;
  margin: 0;
  color: var(--color-text, #1f2937);
}

.detail-description {
  font-size: 0.95rem;
  color: var(--color-text, #1f2937);
  margin: 0;
  line-height: 1.5;
}

.detail-section {
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
}

.detail-section label {
  font-size: 0.75rem;
  font-weight: 600;
  color: var(--color-text-muted, #6b7280);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.detail-code {
  display: block;
  padding: 0.5rem 0.75rem;
  background: rgba(0, 0, 0, 0.04);
  border-radius: 6px;
  font-size: 0.8rem;
  font-family: 'Menlo', 'Monaco', monospace;
  word-break: break-all;
  color: var(--color-text, #1f2937);
}

.detail-actions {
  display: flex;
  gap: 0.5rem;
  padding-top: 0.5rem;
}

.btn {
  padding: 0.5rem 1rem;
  border: none;
  border-radius: 6px;
  font-size: 0.85rem;
  cursor: pointer;
  transition: all 0.15s;
}

.btn-danger {
  background: #ef4444;
  color: #fff;
}

.btn-danger:hover:not(:disabled) {
  background: #dc2626;
}

.btn-danger:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.detail-hint {
  padding-top: 0.5rem;
  border-top: 1px dashed var(--color-border, #e5e7eb);
}

.detail-hint label.muted {
  font-size: 0.75rem;
  color: var(--color-text-muted, #6b7280);
  font-style: italic;
}

.no-selection {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--color-text-muted, #6b7280);
  font-size: 0.9rem;
}

/* Toast */
.toast {
  position: absolute;
  bottom: 1.5rem;
  left: 50%;
  transform: translateX(-50%);
  padding: 0.6rem 1rem;
  border-radius: 6px;
  font-size: 0.85rem;
  color: #fff;
  background: #333;
  box-shadow: 0 4px 12px rgba(0, 0, 0, 0.15);
  cursor: pointer;
  z-index: 100;
}

.toast.success { background: #22c55e; }
.toast.error { background: #ef4444; }
.toast.info { background: #3b82f6; }

.toast-enter-active,
.toast-leave-active {
  transition: all 0.2s ease;
}
.toast-enter-from,
.toast-leave-to {
  opacity: 0;
  transform: translateX(-50%) translateY(10px);
}
</style>
