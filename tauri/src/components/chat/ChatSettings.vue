<template>
  <div class="chat-settings">
    <!-- Agent 按钮 -->
    <div class="setting-btn" @click.stop="toggleMenu('agent')" :title="currentAgentInfo?.description || '选择认知人格（可不选）'">
      <span class="icon">🎭</span>
      <span class="label">{{ currentAgentInfo?.name || '不使用 Agent' }}</span>
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
          <!-- 不使用 Agent：纯工具模式（会话由 session 编排，agent 插件不参与） -->
          <div
            class="option"
            :class="{ active: !agentId }"
            @click.stop="selectAgent(null)"
          >
            <span class="icon">🚫</span>
            <div class="text">
              <div class="label">不使用 Agent</div>
              <div class="desc">纯工具模式：直接与 Model 对话，可用文件/搜索等基础工具</div>
            </div>
            <svg v-if="!agentId" class="check" viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2">
              <polyline points="20 6 9 17 4 12"></polyline>
            </svg>
          </div>
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
        <div class="divider" style="margin: var(--space-1) 0;"></div>
        <div class="option create-btn" @click.stop="openCreateAgentModal">
          <span class="icon">➕</span>
          <div class="text">
            <div class="label" style="color: var(--accent)">创建新人格</div>
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
import { listResources } from '@/services/resources'
import { listModelProviders } from '@/services/modelProviders'
import type { ModelProviderConfig, ModelProvidersConfig } from '@/schemas/model_providers'
import { useSessionsStore } from '@/stores/sessions'
import { logger } from '@/utils/logger'

const props = defineProps<{
  /** 当前选定的智能体 id；null/空 = 不使用 Agent（纯工具模式） */
  agentId: string | null
  availableAgents: AgentProfile[]
  modelProviderId: string
  availableModelProviders: ModelProviderConfig[]
  /** 当前会话 id：用于读/写"运行模式"（auto / interactive，按会话记忆） */
  sessionId?: string
}>()

const emit = defineEmits<{
  'update:agentId': [value: string | null]
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
const newAgent = ref<Partial<AgentProfile>>({ id: '', name: '', description: '' })
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

// 计算当前显示信息：未选择智能体时为 undefined（按钮显示"不使用 Agent"）
const currentAgentInfo = computed(() => agents.value.find((p: AgentProfile) => p.id === props.agentId))

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

/** 选择智能体；null = 不使用 Agent（纯工具模式，session 编排不带智能体人格） */
function selectAgent(id: string | null) {
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
    // 统一资源协议 resources/list：ResourceSummary → 选择器选项（id/name/description）
    const resp = await listResources('agent')
    const list: AgentProfile[] = (resp.items ?? []).map((it) => ({
      id: it.id,
      name: it.name || it.id,
      description: it.description || it.summary || '',
      // 列表仅用于选择；7D 详情字段由资源管理页维护，这里填默认值满足类型
      knowledge: [],
      experience: [],
      skill: [],
      judgment: [],
      strategy: [],
      intuition: [],
      emotion: [],
      context_messages: 6,
    }))
    emit('update:availableAgents', list)
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
  showCreateAgentModal.value = true
}

async function saveNewAgent() {
  if (!newAgent.value.id || !newAgent.value.name) return
  savingAgent.value = true

  const rules = rawAgentRules.value.split('\n').map(s => s.trim().replace(/^- /, '')).filter(s => s.length > 0)

  // agent/create 契约：cognition_units 至少含 id='identity' 的单元（name 必填）；
  // 规则/偏好映射为 judgment 认知单元（与 seed_agents 数据同一约定）。
  // 所有 agent 统一写入全局目录（is_global 已弃用，仅为 API 兼容保留）。
  const cognitionUnits = [
    {
      id: 'identity',
      is_a: ['fact'],
      level: 'sys',
      name: newAgent.value.name,
      description: newAgent.value.description || '',
    },
    ...rules.map((r, i) => ({
      id: `rule_${i + 1}`,
      is_a: ['judgment'],
      level: 'sys',
      description: r,
    })),
  ]

  try {
    await callPlugin('agent/create', {
      id: newAgent.value.id,
      is_global: true,
      cognition_units: cognitionUnits,
    })
    await fetchAgents()
    selectAgent(newAgent.value.id)
    showCreateAgentModal.value = false
  } catch (e) {
    logger.error('ChatSettings', 'Failed to save agent', e)
    alert(`保存失败：${e instanceof Error ? e.message : String(e)}`)
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
  gap: var(--space-1);
  position: relative;
}

.setting-btn {
  display: inline-flex;
  align-items: center;
  gap: var(--space-1);
  padding: var(--space-2) 0.6rem;
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: background-color var(--motion-fast) var(--motion-ease),
    color var(--motion-fast) var(--motion-ease);
  font-size: var(--font-size-xs);
  color: var(--text-muted);
  user-select: none;
}

.setting-btn:hover {
  background: var(--surface-hover);
  color: var(--text-secondary);
}

.setting-btn .icon { font-size: 0.875rem; }
.setting-btn .label { font-weight: var(--font-weight-medium); }

/* 心跳任务按钮：与 Agent/Model/Mode 选项保持同一视觉规格，垂直居中对齐 */
.heartbeat-btn {
  gap: var(--space-1);
  color: var(--text-muted);
}
.heartbeat-btn .icon { font-size: 0.9rem; line-height: 1; }

/* 纯图标按钮（风险等级 / 心跳 / 交互模式）：去掉文字与下拉箭头，进一步压缩显示区域 */
.setting-btn.compact {
  padding: var(--space-2) 0.45rem;
  gap: 0;
}
.setting-btn.compact .icon { font-size: 1rem; }
.heartbeat-btn.on {
  color: var(--danger-solid);
  background: var(--danger-bg);
}
.heartbeat-btn.on:hover {
  background: var(--danger-bg);
  color: var(--danger-solid);
}

.arrow { transition: transform var(--motion-base) var(--motion-ease); opacity: 0.5; }
.arrow.open { transform: rotate(180deg); opacity: 1; }

.divider {
  width: 1px;
  height: 1rem;
  background: var(--border-default);
  margin: 0 var(--space-1);
}

.menu {
  position: absolute;
  bottom: calc(100% + var(--space-2));
  left: 0;
  min-width: 13.75rem;
  background: var(--surface-overlay);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-xl);
  box-shadow: var(--shadow-2);
  padding: var(--space-2);
  z-index: var(--z-overlay);
  max-height: 25rem;
  overflow-y: auto;
}

.option {
  display: flex;
  align-items: center;
  gap: var(--space-3);
  padding: var(--space-3);
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: background-color var(--motion-fast) var(--motion-ease);
}

.option:hover { background: var(--surface-hover); }
.option.active { background: var(--surface-selected); }

.option .icon { font-size: 1.25rem; flex-shrink: 0; }
.text { flex: 1; min-width: 0; }
.text .label { font-size: var(--font-size-base); font-weight: var(--font-weight-medium); color: var(--text-primary); margin-bottom: var(--space-05); }
.text .desc { font-size: var(--font-size-xs); color: var(--text-muted); line-height: var(--line-height-tight); }

.check { color: var(--accent); flex-shrink: 0; }

.dropdown-enter-active, .dropdown-leave-active { transition: all 0.2s ease; }
.dropdown-enter-from, .dropdown-leave-to { opacity: 0; transform: translateY(0.5rem); }

.loading-state {
  display: flex;
  align-items: center;
  justify-content: center;
  gap: var(--space-2);
  padding: var(--space-4);
  color: var(--text-muted);
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

.modal-overlay { position: fixed; top: 0; left: 0; right: 0; bottom: 0; background: var(--overlay); display: flex; align-items: center; justify-content: center; z-index: var(--z-dialog); }
.modal { background: var(--surface-overlay); border-radius: var(--radius-xl); padding: var(--space-5); width: 100%; max-width: 25rem; }
.modal h3 { margin-bottom: var(--space-4); }
.form-group { margin-bottom: var(--space-4); }
.form-group label { display: block; font-size: var(--font-size-base); font-weight: var(--font-weight-medium); margin-bottom: var(--space-1); }
.form-group input, .form-group textarea { width: 100%; padding: var(--space-2) var(--space-3); border: 1px solid var(--border-default); border-radius: var(--radius-md); font-size: var(--font-size-base); box-sizing: border-box; }
.form-group textarea { font-family: inherit; }
.checkbox-label { display: flex !important; align-items: center; gap: var(--space-2); cursor: pointer; }
.checkbox-label input { width: auto; }
.modal-actions { display: flex; justify-content: flex-end; gap: var(--space-3); margin-top: var(--space-5); }
.action-btn { padding: var(--space-2) var(--space-4); background: var(--accent); color: var(--text-on-accent); border: none; border-radius: var(--radius-md); cursor: pointer; font-size: var(--font-size-base); }
.action-btn:disabled { opacity: 0.6; cursor: not-allowed; }
.action-btn.secondary { background: var(--surface-hover); color: var(--text-primary); }
</style>
