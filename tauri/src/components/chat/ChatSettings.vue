<template>
  <div class="chat-settings">
    <!-- Agent 按钮 -->
    <div class="setting-btn" @click.stop="toggleMenu('agent')" :title="currentAgentInfo?.description || '选择认知人格'">
      <span class="icon">🎭</span>
      <span class="label">{{ currentAgentInfo?.name || 'Loading...' }}</span>
      <svg class="arrow" :class="{ open: activeMenu === 'agent' }" viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2">
        <polyline points="6 9 12 15 18 9"></polyline>
      </svg>
    </div>
    
    <!-- Agent 下拉菜单 -->
    <Transition name="dropdown">
      <div v-if="activeMenu === 'agent'" class="menu" @click.stop>
        <!-- 加载状态 -->
        <div v-if="loadingAgents" class="loading-state">
          <svg class="spinner" viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2">
            <circle class="spin" cx="12" cy="12" r="10" stroke-dasharray="100"></circle>
          </svg>
          <span>加载中...</span>
        </div>
        
        <template v-else>
          <div
            v-for="agent in agents"
            :key="agent.id"
            class="option"
            :class="{ active: agentId === agent.id }"
            @click.stop="selectAgent(agent.id)"
          >
          <span class="icon">👤</span>
          <div class="text">
            <div class="label">{{ agent.name }}</div>
            <div class="desc">{{ agent.description }}</div>
          </div>
          <svg v-if="agentId === agent.id" class="check" viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2">
            <polyline points="20 6 9 17 4 12"></polyline>
          </svg>
        </div>
        </template>
        <div class="divider" style="margin: 4px 0;"></div>
        <div class="option create-btn" @click.stop="openCreateAgentModal">
          <span class="icon">➕</span>
          <div class="text">
            <div class="label" style="color: var(--color-primary)">创建新人格</div>
            <div class="desc">定义专属认知模型</div>
          </div>
        </div>
      </div>
    </Transition>
    
    <!-- Model Provider 按钮 -->
    <div class="setting-btn" @click.stop="toggleMenu('model')" :title="currentProviderInfo?.title || '选择 Model'">
      <span class="icon">🧠</span>
      <span class="label">{{ currentProviderInfo?.short || 'Model' }}</span>
      <svg class="arrow" :class="{ open: activeMenu === 'model' }" viewBox="0 0 24 24" width="12" height="12" fill="none" stroke="currentColor" stroke-width="2">
        <polyline points="6 9 12 15 18 9"></polyline>
      </svg>
    </div>

    <!-- Model Provider 下拉菜单 -->
    <Transition name="dropdown">
      <div v-if="activeMenu === 'model'" class="menu" @click.stop>
        <div v-if="loadingProviders" class="loading-state">
          <svg class="spinner" viewBox="0 0 24 24" width="20" height="20" fill="none" stroke="currentColor" stroke-width="2">
            <circle class="spin" cx="12" cy="12" r="10" stroke-dasharray="100"></circle>
          </svg>
          <span>加载中...</span>
        </div>

        <template v-else>
          <div v-if="availableEnabledProviders.length === 0" class="empty-hint">
            暂无可用 Model，请前往「设置 → Model Provider」添加
          </div>
          <div
            v-for="p in availableEnabledProviders"
            :key="p.id"
            class="option"
            :class="{ active: modelProviderId === p.id }"
            @click.stop="selectModelProvider(p.id)"
          >
            <span class="icon">🤖</span>
            <div class="text">
              <div class="label">{{ p.name || p.id }}</div>
              <div class="desc">{{ p.provider }} · {{ p.model || '未设置模型' }}</div>
            </div>
            <svg v-if="modelProviderId === p.id" class="check" viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2">
              <polyline points="20 6 9 17 4 12"></polyline>
            </svg>
          </div>
        </template>
      </div>
    </Transition>

    <div class="divider"></div>
    
    <!-- 执行风险等级按钮（纯图标，节约空间） -->
    <div class="setting-btn compact" @click.stop="toggleMenu('risk')" :title="'工具执行风险等级：' + currentRiskInfo.label + '（低于该等级的工具需审批）'">
      <span class="icon">{{ currentRiskInfo.icon }}</span>
    </div>

    <!-- 心跳任务按钮（纯图标，定时任务图标） -->
    <div
      class="setting-btn heartbeat-btn compact"
      :class="{ on: heartbeatEnabled }"
      @click.stop="emit('open-heartbeat')"
      :title="heartbeatEnabled ? '心跳任务已开启，点击配置' : '配置会话心跳（定时）任务'"
    >
      <span class="icon">⏰</span>
    </div>

    <!-- 执行风险等级下拉菜单 -->
    <Transition name="dropdown">
      <div v-if="activeMenu === 'risk'" class="menu" @click.stop>
        <div
          v-for="r in riskLevels"
          :key="r.value"
          class="option"
          :class="{ active: riskLevel === r.value }"
          @click.stop="selectRiskLevel(r.value)"
        >
          <span class="icon">{{ r.icon }}</span>
          <div class="text">
            <div class="label">{{ r.label }}</div>
            <div class="desc">{{ r.description }}</div>
          </div>
          <svg v-if="riskLevel === r.value" class="check" viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2">
            <polyline points="20 6 9 17 4 12"></polyline>
          </svg>
        </div>
      </div>
    </Transition>

    <!-- 运行模式按钮（auto / interactive，纯图标） -->
    <div class="setting-btn compact" @click.stop="toggleMenu('mode')" :title="'运行模式：' + currentModeInfo.label + '（' + currentModeInfo.description + '）'">
      <span class="icon">{{ currentModeInfo.icon }}</span>
    </div>

    <!-- 运行模式下拉菜单 -->
    <Transition name="dropdown">
      <div v-if="activeMenu === 'mode'" class="menu" @click.stop>
        <div
          v-for="m in runModes"
          :key="m.value"
          class="option"
          :class="{ active: currentMode === m.value }"
          @click.stop="selectMode(m.value)"
        >
          <span class="icon">{{ m.icon }}</span>
          <div class="text">
            <div class="label">{{ m.label }}</div>
            <div class="desc">{{ m.description }}</div>
          </div>
          <svg v-if="currentMode === m.value" class="check" viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2">
            <polyline points="20 6 9 17 4 12"></polyline>
          </svg>
        </div>
      </div>
    </Transition>

    <!-- 创建 Agent 弹窗 -->
    <div v-if="showCreateAgentModal" class="modal-overlay" @click.self="showCreateAgentModal = false">
      <div class="modal">
        <h3>创建认知人格</h3>
        <div class="form-group">
          <label>名称 (Name)</label>
          <input v-model="newAgent.name" placeholder="输入名称，例如：前端专家" />
        </div>
        <div class="form-group">
          <label>标识符 (ID)</label>
          <input v-model="newAgent.id" placeholder="唯一标识符，如：frontend_expert" />
        </div>
        <div class="form-group">
          <label>描述 (Description)</label>
          <textarea v-model="newAgent.description" rows="2" placeholder="简短描述这个角色"></textarea>
        </div>
        <div class="form-group">
          <label>规则与偏好 (7D 设定 - 可选，每行一条)</label>
          <textarea v-model="rawAgentRules" rows="5" placeholder="例如：\n- 始终使用 TypeScript\n- 喜欢 TailwindCSS"></textarea>
        </div>
        <div class="form-group">
          <label class="checkbox-label">
            <input type="checkbox" v-model="isGlobalAgent" />
            保存为全局角色 (跨项目可用)
          </label>
        </div>
        <div class="modal-actions">
          <button class="action-btn secondary" @click="showCreateAgentModal = false">取消</button>
          <button class="action-btn" @click="saveNewAgent" :disabled="savingAgent">保存</button>
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import { AgentProfile } from '@/types'
import { callPlugin } from '@/services/plugin'
import { listModelProviders } from '@/services/modelProviders'
import type { ModelProviderConfig, ModelProvidersConfig } from '@/schemas/model_providers'
import { useSessionsStore } from '@/stores/sessions'
import { logger } from '@/utils/logger'

const props = defineProps<{
  agentId: string
  availableAgents: AgentProfile[]
  modelProviderId: string
  availableModelProviders: ModelProviderConfig[]
  /** 当前会话 id：用于读/写"运行模式"（auto / interactive，按会话记忆） */
  sessionId?: string
}>()

const emit = defineEmits<{
  'update:agentId': [value: string]
  'update:availableAgents': [value: AgentProfile[]]
  'update:modelProviderId': [value: string]
  'update:availableModelProviders': [value: ModelProviderConfig[]]
  /** 点击「心跳」按钮：打开会话设置（心跳任务配置）弹窗 */
  'open-heartbeat': []
}>()

const sessionsStore = useSessionsStore()

// 当前会话是否启用了心跳任务（用于在「心跳」按钮上显示 ♥ 状态）
const heartbeatEnabled = computed(
  () => !!sessionsStore.activeListItem?.metadata?.heartbeat?.enabled,
)

const activeMenu = ref<'agent' | 'risk' | 'model' | 'mode' | null>(null)
const agents = computed(() => props.availableAgents || [])
const loadingAgents = ref(false)
const loadingProviders = ref(false)

// Create Agent states
const showCreateAgentModal = ref(false)
const savingAgent = ref(false)
const isGlobalAgent = ref(false)
const newAgent = ref<Partial<AgentProfile>>({ id: '', name: '', description: '', context_messages: 6 })
const rawAgentRules = ref('')

// 执行风险等级（替代原「审批模式」）：与工具自身风险等级一致。
// 工具风险等级 ≥ 执行风险等级 → 自动执行；< 执行风险等级 → 需用户审批。
const riskLevels = [
  { value: 'low' as const, label: '低风险', icon: '🟢', description: '仅自动执行低风险工具；中/高风险需审批' },
  { value: 'medium' as const, label: '中风险', icon: '🟡', description: '中风险及以下自动执行；高风险需审批' },
  { value: 'high' as const, label: '高风险', icon: '🔴', description: '所有工具自动执行（含高风险）' }
]
// 执行风险等级：与 agent_id/provider_id/mode 同级别——走 store（per-session 记忆 + 持久化到 metadata.risk_level）。
// 切换会话/刷新页面后下拉框保留选中值；不再使用全局 LocalConfig（全局值仅作为旧会话的迁移源）。
const riskLevel = computed<'low' | 'medium' | 'high'>(() => {
  return props.sessionId ? sessionsStore.getSessionRiskLevel(props.sessionId) : 'medium'
})
const currentRiskInfo = computed(() => riskLevels.find(r => r.value === riskLevel.value) || riskLevels[1])

// ── 运行模式（auto / interactive）──
// 与"风险等级"正交：风险等级决定哪些工具需审批；运行模式决定工具失败时是否阻塞 LLM。
// - interactive：默认，需交互的工具（confirm/ask_user）在会话流中产 user_prompt 节点（卡片）；
//   需审批的工具产确认卡，等用户点同意后才执行。
// - auto：无人值守，所有工具失败（含需审批/需交互）直接返回友好错误，让 LLM 自行决策继续；
//   不会产 user_prompt 节点，避免阻塞后台批处理。
// 按会话记忆：切换会话不丢；不写入 session metadata（仅前端偏好）。
const runModes = [
  {
    value: 'interactive' as const,
    label: '交互',
    icon: '💬',
    description: '默认：需审批/需交互的工具在会话流中显示卡片，等待用户响应后继续'
  },
  {
    value: 'auto' as const,
    label: '自动',
    icon: '🤖',
    description: '无人值守：工具失败返回友好错误让 LLM 自行继续，不弹交互卡（适合后台批处理）'
  }
]
const currentMode = computed<'auto' | 'interactive'>(() => {
  return props.sessionId ? sessionsStore.getSessionMode(props.sessionId) : 'interactive'
})
const currentModeInfo = computed(() => runModes.find(m => m.value === currentMode.value) || runModes[0])
function selectMode(value: 'auto' | 'interactive') {
  if (props.sessionId) {
    sessionsStore.setSessionMode(props.sessionId, value)
  }
  closeAllMenus()
}

// 计算当前显示信息
const currentAgentInfo = computed(() => agents.value.find((p: AgentProfile) => p.id === props.agentId) || agents.value[0])

// 菜单逻辑
function toggleMenu(menu: 'agent' | 'risk' | 'model' | 'mode') {
  const nextMenu = activeMenu.value === menu ? null : menu
  if (nextMenu === 'agent') {
    fetchAgents()
  } else if (nextMenu === 'model') {
    fetchModelProviders()
  }
  activeMenu.value = nextMenu
}

function closeAllMenus() {
  activeMenu.value = null
}

async function selectRiskLevel(level: 'low' | 'medium' | 'high') {
  if (props.sessionId) {
    sessionsStore.setSessionRiskLevel(props.sessionId, level)
  }
  closeAllMenus()
}

function selectAgent(id: string) {
  emit('update:agentId', id)
  closeAllMenus()
}

// ─────── Model Provider ───────
const availableEnabledProviders = computed(() =>
  (props.availableModelProviders || []).filter((p) => p.enabled !== false)
)

const currentProviderInfo = computed(() => {
  const all = availableEnabledProviders.value
  const id = props.modelProviderId
  const p = all.find((x) => x.id === id)
  if (p) {
    return {
      short: p.name || p.id,
      title: `${p.provider} · ${p.model || '未设置模型'}（限流 ${p.rate_limit_ms ?? 0}ms）`
    }
  }
  // 未显式选择或 id 失效 → 提示"默认"
  return {
    short: id ? `${id}（已失效）` : '默认',
    title: '未选择 Model，将使用默认 Provider'
  }
})

async function fetchModelProviders(force = false) {
  if (loadingProviders.value) return
  // 已有数据时只做软刷新（用户在设置页可能调整过）
  if (!force && props.availableModelProviders && props.availableModelProviders.length > 0) {
    // 静默后台刷新
    listModelProviders()
      .then((cfg) => emit('update:availableModelProviders', Object.values(cfg.providers ?? {})))
      .catch((err) => logger.error('ChatSettings', '后台刷新 Model 列表失败', err))
    return
  }
  loadingProviders.value = true
  try {
    const cfg: ModelProvidersConfig = await listModelProviders()
    emit('update:availableModelProviders', Object.values(cfg.providers ?? {}))
  } catch (err) {
    logger.error('ChatSettings', '加载 Model 列表失败', err)
  } finally {
    loadingProviders.value = false
  }
}

function selectModelProvider(id: string) {
  emit('update:modelProviderId', id)
  closeAllMenus()
}

// 监听外部传入的 provider 列表变化：若当前 modelProviderId 失效，清空
watch(
  () => availableEnabledProviders.value,
  (list) => {
    if (props.modelProviderId && !list.some((p) => p.id === props.modelProviderId)) {
      emit('update:modelProviderId', '')
    }
  }
)

async function fetchAgents() {
  loadingAgents.value = true
  try {
    const list = await callPlugin('agent/list', {})
    if (Array.isArray(list)) {
      emit('update:availableAgents', list)
    } else {
      logger.warn('ChatSettings', 'Agent list is not an array', list)
    }
  } catch (e) {
    logger.error('ChatSettings', 'Failed to load agents', e)
    // 失败时不清除旧数据，避免用户看到空列表
  } finally {
    loadingAgents.value = false
  }
}

function openCreateAgentModal() {
  closeAllMenus()
  newAgent.value = { id: '', name: '', description: '' }
  rawAgentRules.value = ''
  isGlobalAgent.value = false
  showCreateAgentModal.value = true
}

async function saveNewAgent() {
  if (!newAgent.value.id || !newAgent.value.name) return
  savingAgent.value = true
  
  const rules = rawAgentRules.value.split('\n').map(s => s.trim().replace(/^- /, '')).filter(s => s.length > 0)
  
  const profile = {
    ...newAgent.value,
    judgment: rules,
    knowledge: [], experience: [], skill: [], strategy: [], intuition: [], emotion: []
  }
  
  try {
    await callPlugin('agent/save', {
      ...profile,
      is_global: isGlobalAgent.value
    })
    await fetchAgents()
    selectAgent(profile.id!)
    showCreateAgentModal.value = false
  } catch (e) {
    logger.error('ChatSettings', 'Failed to save agent', e)
    alert('保存失败')
  } finally {
    savingAgent.value = false
  }
}

// 点击外部关闭
function handleClickOutside(event: MouseEvent) {
  const el = event.target as HTMLElement
  if (!el.closest('.chat-settings') && !el.closest('.modal')) {
    closeAllMenus()
  }
}

onMounted(() => {
  document.addEventListener('click', handleClickOutside)
  fetchAgents()
  fetchModelProviders()
})

onUnmounted(() => {
  document.removeEventListener('click', handleClickOutside)
})
</script>

<style scoped>
.chat-settings {
  display: flex;
  align-items: center;
  gap: 0.25rem;
  position: relative;
}

.setting-btn {
  display: inline-flex;
  align-items: center;
  gap: 0.3rem;
  padding: 0.35rem 0.6rem;
  border-radius: 6px;
  cursor: pointer;
  transition: all 0.2s;
  font-size: 0.75rem;
  color: var(--color-text-muted);
  user-select: none;
}

.setting-btn:hover {
  background: rgba(0, 0, 0, 0.04);
  color: var(--color-text-secondary);
}

.setting-btn .icon { font-size: 0.875rem; }
.setting-btn .label { font-weight: 500; }

/* 心跳任务按钮：与 Agent/Model/Mode 选项保持同一视觉规格，垂直居中对齐 */
.heartbeat-btn {
  gap: 0.3rem;
  color: var(--color-text-muted);
}
.heartbeat-btn .icon { font-size: 0.9rem; line-height: 1; }

/* 纯图标按钮（风险等级 / 心跳 / 交互模式）：去掉文字与下拉箭头，进一步压缩显示区域 */
.setting-btn.compact {
  padding: 0.35rem 0.45rem;
  gap: 0;
}
.setting-btn.compact .icon { font-size: 1rem; }
.heartbeat-btn.on {
  color: #ef4444;
  background: rgba(239, 68, 68, 0.1);
}
.heartbeat-btn.on:hover {
  background: rgba(239, 68, 68, 0.16);
  color: #ef4444;
}

.arrow { transition: transform 0.2s; opacity: 0.5; }
.arrow.open { transform: rotate(180deg); opacity: 1; }

.divider {
  width: 1px;
  height: 16px;
  background: var(--color-border);
  margin: 0 0.25rem;
}

.menu {
  position: absolute;
  bottom: calc(100% + 8px);
  left: 0;
  min-width: 220px;
  background: white;
  border-radius: 12px;
  box-shadow: 0 10px 40px rgba(0, 0, 0, 0.15), 0 0 0 1px rgba(0, 0, 0, 0.05);
  padding: 0.5rem;
  z-index: 100;
  max-height: 400px;
  overflow-y: auto;
}

.option {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding: 0.75rem;
  border-radius: 8px;
  cursor: pointer;
  transition: background 0.15s;
}

.option:hover { background: #f5f5f5; }
.option.active { background: #f0f0ff; }

.option .icon { font-size: 1.25rem; flex-shrink: 0; }
.text { flex: 1; min-width: 0; }
.text .label { font-size: 0.875rem; font-weight: 500; color: var(--color-text); margin-bottom: 0.15rem; }
.text .desc { font-size: 0.75rem; color: var(--color-text-muted); line-height: 1.3; }

.check { color: var(--color-primary); flex-shrink: 0; }

.dropdown-enter-active, .dropdown-leave-active { transition: all 0.2s ease; }
.dropdown-enter-from, .dropdown-leave-to { opacity: 0; transform: translateY(8px); }

@keyframes slideUp {
  from { opacity: 0; transform: translateY(8px); }
  to { opacity: 1; transform: translateY(0); }
}

.menu {
  animation: slideUp 0.2s ease-out;
}

.loading-state {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 0.5rem;
  padding: 1rem;
  color: var(--color-text-muted);
}

.spinner {
  animation: spin 1s linear infinite;
}

.spin {
  stroke-linecap: round;
  animation: dash 1.5s ease-in-out infinite;
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

@keyframes dash {
  0% { stroke-dashoffset: 100; }
  50% { stroke-dashoffset: 0; }
  100% { stroke-dashoffset: -100; }
}

.modal-overlay { position: fixed; top: 0; left: 0; right: 0; bottom: 0; background: rgba(0, 0, 0, 0.5); display: flex; align-items: center; justify-content: center; z-index: 1000; }
.modal { background: white; border-radius: 12px; padding: 1.5rem; width: 100%; max-width: 400px; }
.modal h3 { margin-bottom: 1rem; }
.form-group { margin-bottom: 1rem; }
.form-group label { display: block; font-size: 0.875rem; font-weight: 500; margin-bottom: 0.25rem; }
.form-group input, .form-group textarea { width: 100%; padding: 0.5rem 0.75rem; border: 1px solid var(--color-border); border-radius: 6px; font-size: 0.875rem; box-sizing: border-box; }
.form-group textarea { font-family: inherit; }
.checkbox-label { display: flex !important; align-items: center; gap: 0.5rem; cursor: pointer; }
.checkbox-label input { width: auto; }
.modal-actions { display: flex; justify-content: flex-end; gap: 0.75rem; margin-top: 1.5rem; }
.action-btn { padding: 0.5rem 1rem; background: var(--color-primary); color: white; border: none; border-radius: 6px; cursor: pointer; font-size: 0.875rem; }
.action-btn:disabled { opacity: 0.6; cursor: not-allowed; }
.action-btn.secondary { background: #f0f0f0; color: var(--color-text); }
</style>
