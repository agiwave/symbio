<!--
  McpView — 基于 ResourceShell 的两栏管理页
-->
<template>
  <ResourceShell
    title="MCP Server"
    :loading="loading"
    :has-list-content="servers.length > 0"
    @new="enterCreateMode"
  >
    <template #meta v-if="serverCount > 0">
      <span class="running-pulse" />
      共 {{ serverCount }} 个 Server（{{ enabledCount }} 启用）
    </template>

    <template #list>
      <div class="server-list">
        <McpServerCard
          v-for="item in servers"
          :key="item.name"
          :name="item.name"
          :server="item.server"
          :is-active="selectedName === item.name"
          @click="select(item.name)"
        />
      </div>
    </template>

    <template #empty>
      <p>暂无 MCP Server</p>
      <p class="hint">点击右上角 + 创建新 Server</p>
    </template>

    <template #detail>
      <McpServerSettings
        :name="selectedName"
        :server="selectedServer"
        :saving="saving"
        :testing="testing"
        :deleting="deletingId === selectedName"
        @save="handleSave"
        @test="handleTest"
        @delete="requestDelete"
      />
    </template>

    <!-- BUG-FR10：测试连接结果详情卡片 -->
    <template #toast>
      <!-- 测试结果详情面板（持久显示直到下次测试） -->
      <Transition name="toast">
        <div v-if="lastTestResult" :class="['test-result', lastTestResult.ok ? 'success' : 'error']">
          <div class="test-result-header">
            <span class="test-result-icon">{{ lastTestResult.ok ? '✓' : '✗' }}</span>
            <span class="test-result-title">
              {{ lastTestResult.ok ? '连接成功' : '连接失败' }}
            </span>
            <button class="test-result-close" @click="lastTestResult = null">×</button>
          </div>
          <div v-if="lastTestResult.ok" class="test-result-body">
            <div v-if="lastTestResult.server_name || lastTestResult.server_version" class="test-result-row">
              <span class="row-label">Server</span>
              <span class="row-value">
                {{ lastTestResult.server_name ?? '未知' }}
                <span v-if="lastTestResult.server_version" class="version">
                  v{{ lastTestResult.server_version }}
                </span>
              </span>
            </div>
            <div class="test-result-row">
              <span class="row-label">协议版本</span>
              <span class="row-value mono">{{ lastTestResult.protocol_version }}</span>
            </div>
            <div class="test-result-row">
              <span class="row-label">发现工具</span>
              <span class="row-value">{{ lastTestResult.tool_count }} 个</span>
            </div>
            <div class="test-result-row">
              <span class="row-label">耗时</span>
              <span class="row-value">{{ lastTestResult.elapsed_ms }} ms</span>
            </div>
            <div v-if="lastTestResult.instructions" class="test-result-instructions">
              <span class="row-label">使用说明</span>
              <p class="instructions-text">{{ lastTestResult.instructions }}</p>
            </div>
          </div>
          <div v-else class="test-result-body">
            <p class="error-text">{{ lastTestResult.error ?? '未知错误' }}</p>
          </div>
        </div>
      </Transition>
      <Transition name="toast">
        <div v-if="toast" :class="['toast', toast.type]" @click="toast = null">
          {{ toast.text }}
        </div>
      </Transition>
    </template>
  </ResourceShell>

  <!-- BUG-FR5：删除确认 modal（替代浏览器 confirm） -->
  <ConfirmDialog
    v-model:visible="confirmDeleteVisible"
    title="确认删除"
    :message="confirmDeleteMessage"
    confirm-text="删除"
    cancel-text="取消"
    danger
    icon="🗑"
    icon-kind="danger"
    :loading="deletingId !== null"
    @confirm="confirmDelete"
    @cancel="confirmDeleteVisible = false"
  />
</template>

<script setup lang="ts">
import { computed, onMounted, ref } from 'vue'
import type { McpServersTest } from '@/schemas/mcp_servers'
import {
  listMcpServers,
  setMcpServer,
  deleteMcpServer,
  testMcpServer,
  flattenMcpServers
} from '@/services/mcpServers'
import { useResourceManager } from '@/composables/useResourceManager'
import type { McpConfig, McpServerConfig } from '@/schemas/mcp_config'
import McpServerSettings from '../components/settings/McpServerSettings.vue'
import McpServerCard from '../components/settings/McpServerCard.vue'
import ResourceShell from '../components/common/ResourceShell.vue'
import ConfirmDialog from '../components/common/ConfirmDialog.vue'

const {
  loading,
  saving,
  testing,
  creating,
  selectedId: selectedName,
  deletingId,
  toast,
  showToast,
  enterCreateMode,
  select,
  markDeleting,
} = useResourceManager({ logTag: 'McpView' })

// === 状态 ===
const servers = ref<Array<{ name: string; server: McpServerConfig }>>([])
const lastTestResult = ref<McpServersTest.Response | null>(null)

// === 计算属性 ===
const serverCount = computed(() => servers.value.length)
const enabledCount = computed(() => servers.value.filter((s) => s.server.enabled).length)

const selectedServer = computed<McpServerConfig | null>(() => {
  if (!selectedName.value) return null
  return servers.value.find((s) => s.name === selectedName.value)?.server ?? null
})

// BUG-FR5：删除确认 modal
const confirmDeleteVisible = ref(false)
const confirmDeleteName = ref<string | null>(null)
const confirmDeleteMessage = computed(() => {
  if (!confirmDeleteName.value) return ''
  return `确认删除 MCP Server「${confirmDeleteName.value}」？\n此操作不可恢复。`
})

// === 操作 ===
async function loadAll() {
  loading.value = true
  try {
    const cfg: McpConfig = await listMcpServers()
    servers.value = flattenMcpServers(cfg)

    if (!selectedName.value && !creating.value) {
      selectedName.value = servers.value[0]?.name ?? null
    }
  } catch (err) {
    showToast('error', `加载失败: ${err}`)
  } finally {
    loading.value = false
  }
}

async function handleSave(payload: { name: string; server: McpServerConfig }) {
  const { name, server } = payload
  if (!name) {
    showToast('error', '请填写 Server 名称')
    return
  }
  // BUG-FR6：客户端字段校验（失焦时已校验，保存时再次确认）
  const t = server.type ?? 'stdio'
  if (t === 'stdio' && !server.command?.trim()) {
    showToast('error', '请填写启动命令')
    return
  }
  if ((t === 'http' || t === 'sse') && !server.url?.trim()) {
    showToast('error', '请填写 URL')
    return
  }
  saving.value = true
  try {
    if (creating.value) {
      if (servers.value.some((x) => x.name === name)) {
        showToast('error', `名称 "${name}" 已存在，请换一个`)
        return
      }
    }
    const cfg = await setMcpServer(name, server)
    showToast('success', `Server ${name} 已保存`)
    creating.value = false
    servers.value = flattenMcpServers(cfg)
    selectedName.value = name
  } catch (err) {
    showToast('error', `保存失败: ${err}`)
  } finally {
    saving.value = false
  }
}

/** BUG-FR7：测试 MCP server 连接（不修改任何状态） */
async function handleTest(payload: { name: string; server: McpServerConfig }) {
  if (!payload.name) {
    showToast('error', '请先填写名称')
    return
  }
  // 必须先保存才能测试（否则 server 还没在 config 里）
  // 简化：要求 server 已存在
  const exists = servers.value.some((s) => s.name === payload.name)
  if (!exists) {
    showToast('error', '请先保存该 Server 再测试')
    return
  }
  testing.value = true
  try {
    const result = await testMcpServer(payload.name)
    lastTestResult.value = result
    if (result.ok) {
      showToast(
        'success',
        `✓ ${result.name} 连接成功（${result.tool_count} 个工具，${result.elapsed_ms}ms）`
      )
    } else {
      showToast('error', `连接失败: ${result.error ?? '未知错误'}`)
    }
  } catch (err) {
    lastTestResult.value = {
      name: payload.name,
      ok: false,
      tool_count: 0,
      protocol_version: 'unknown',
      server_name: null,
      server_version: null,
      instructions: null,
      error: String(err),
      elapsed_ms: 0
    }
    showToast('error', `测试异常: ${err}`)
  } finally {
    testing.value = false
  }
}

/** 触发删除：先打开确认 modal */
function requestDelete(name: string) {
  confirmDeleteName.value = name
  confirmDeleteVisible.value = true
}

/** 确认删除：实际执行 */
async function confirmDelete() {
  const name = confirmDeleteName.value
  if (!name) return
  markDeleting(name)
  try {
    const cfg = await deleteMcpServer(name)
    showToast('success', `Server ${name} 已删除`)
    if (selectedName.value === name) {
      selectedName.value = servers.value.find((x) => x.name !== name)?.name ?? null
    }
    servers.value = flattenMcpServers(cfg)
  } catch (err) {
    showToast('error', `删除失败: ${err}`)
  } finally {
    markDeleting(null)
    confirmDeleteName.value = null
    confirmDeleteVisible.value = false
  }
}

onMounted(() => loadAll())
</script>

<style scoped>
.server-list {
  flex: 1;
  overflow-y: auto;
  padding: 0.25rem 0;
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

/* Test Connection Result Card */
.test-result {
  position: absolute;
  bottom: 1.5rem;
  right: 1.5rem;
  width: 320px;
  padding: 1.2rem;
  border-radius: 8px;
  background: var(--color-surface, #ffffff);
  box-shadow: 0 10px 30px rgba(0, 0, 0, 0.08), 0 1px 3px rgba(0, 0, 0, 0.02);
  border: 1px solid var(--color-border);
  z-index: 101;
  transition: all 0.3s cubic-bezier(0.16, 1, 0.3, 1);
  display: flex;
  flex-direction: column;
  gap: 0.85rem;
}

.test-result.success {
  border-left: 4px solid #22c55e;
}

.test-result.error {
  border-left: 4px solid #ef4444;
}

.test-result-header {
  display: flex;
  align-items: center;
  gap: 0.6rem;
  font-weight: 600;
  font-size: 0.9rem;
}

.test-result.success .test-result-icon {
  color: #22c55e;
  font-weight: bold;
}

.test-result.error .test-result-icon {
  color: #ef4444;
  font-weight: bold;
}

.test-result-title {
  flex: 1;
  color: var(--color-text);
  font-size: 0.9rem;
  font-weight: 600;
}

.test-result-close {
  background: none;
  border: none;
  color: var(--color-text-muted);
  cursor: pointer;
  font-size: 1.25rem;
  padding: 0;
  line-height: 1;
  transition: color 0.15s ease;
}

.test-result-close:hover {
  color: var(--color-text);
}

.test-result-body {
  display: flex;
  flex-direction: column;
  gap: 0.6rem;
  font-size: 0.8rem;
}

.test-result-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  border-bottom: 1px dashed var(--color-border-subtle, rgba(0, 0, 0, 0.04));
  padding-bottom: 0.3rem;
}

.test-result-row:last-child {
  border-bottom: none;
}

.row-label {
  color: var(--color-text-secondary);
}

.row-value {
  color: var(--color-text);
  font-weight: 500;
}

.row-value.mono {
  font-family: var(--font-mono, 'JetBrains Mono', Consolas, monospace);
  font-size: 0.75rem;
}

.row-value .version {
  font-size: 0.7rem;
  color: var(--color-text-muted);
  background: rgba(0, 0, 0, 0.05);
  padding: 0.05rem 0.25rem;
  border-radius: 4px;
  margin-left: 0.25rem;
}

.test-result-instructions {
  display: flex;
  flex-direction: column;
  gap: 0.3rem;
  margin-top: 0.25rem;
  padding: 0.6rem;
  background: rgba(0, 0, 0, 0.02);
  border-radius: 4px;
  max-height: 90px;
  overflow-y: auto;
  border: 1px solid var(--color-border-subtle, rgba(0, 0, 0, 0.04));
}

.test-result-instructions .row-label {
  font-size: 0.75rem;
  font-weight: 600;
  margin-bottom: 0.1rem;
}

.instructions-text {
  margin: 0;
  font-size: 0.75rem;
  color: var(--color-text-secondary);
  line-height: 1.45;
  white-space: pre-wrap;
}

.error-text {
  margin: 0;
  color: #ef4444;
  font-size: 0.8rem;
  line-height: 1.45;
  word-break: break-all;
  max-height: 120px;
  overflow-y: auto;
}
</style>
