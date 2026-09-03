<!--
  ModelProviderForm — Model 资源专属详情/编辑表单

  统一资源协议下的"详情差异化"组件：
  - 列表机制统一由 ResourceManagerView 承载，本组件只负责 model 的编辑表单；
  - 数据契约：props.item 为 null 时是"新建"模式，否则从 item.extra.config 预填；
  - 保存统一走 resources/upload（manifest 上传），不引入独立协议。

  表单能力（恢复自旧 ModelProvidersSettings）：
  - 提供商预设下拉（自动填充 API Base / 模型候选 / 可用协议）
  - 模型 datalist 候选、API Key 显隐、启用开关
  - 高级设置折叠（协议 / 温度 / Max Tokens / 频率限制 / 系统提示词）
  - 校验连接、跳过校验保存、设为默认、删除
-->
<template>
  <div class="provider-form">
    <!-- 顶部：标题 + 状态徽标 + 操作 -->
    <header class="form-header">
      <div class="title-area">
        <span class="status-dot" :class="statusDotClass" />
        <div class="title-block">
          <div class="title-line">
            <h2 class="title-text">{{ form.name || form.model || '新建 Provider' }}</h2>
            <span v-if="isDefault" class="badge default">默认</span>
            <span v-if="!form.enabled" class="badge disabled">已停用</span>
          </div>
          <p class="subtitle">
            <code class="provider-pill">{{ displayLabel || form.provider }}</code>
            <span class="dot">·</span>
            <span>{{ form.model || '未设置模型' }}</span>
            <template v-if="isExisting && resourcePathLabel">
              <span class="dot">·</span>
              <code class="provider-pill path-pill" :title="`${resourcePathLabel}（点击复制）`" @click="copyPath">{{ resourcePathLabel }}</code>
            </template>
          </p>
        </div>
      </div>
      <div class="header-actions">
        <button
          v-if="capabilities.test_connection"
          type="button"
          class="action-btn secondary"
          :disabled="saving || testing || !isExisting"
          @click="$emit('test')"
        >
          {{ testing ? '校验中…' : '校验连接' }}
        </button>
        <button
          type="button"
          class="action-btn secondary"
          :disabled="saving"
          @click="emitSave(true)"
        >
          跳过校验保存
        </button>
        <button type="button" class="action-btn" :disabled="saving" @click="emitSave(false)">
          {{ saving ? '保存中…' : '保存' }}
        </button>
        <span class="header-actions-divider" aria-hidden="true" />
        <button
          v-if="!isDefault && isExisting"
          class="icon-btn"
          title="设为默认 Provider"
          @click="$emit('set-default')"
        >
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2">
            <polygon points="12 2 15 8.5 22 9.3 17 14 18.2 21 12 17.8 5.8 21 7 14 2 9.3 9 8.5 12 2" />
          </svg>
        </button>
        <button
          class="icon-btn danger"
          :disabled="!isExisting || deleting"
          :title="isExisting ? '删除 Provider' : '保存后才能删除'"
          @click="$emit('delete')"
        >
          <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2">
            <polyline points="3 6 5 6 21 6" />
            <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
          </svg>
        </button>
      </div>
    </header>

    <!-- 表单主体 -->
    <div class="form-body">
      <div class="setting-group">
        <div class="setting-item">
          <div class="setting-info">
            <label>名称</label>
            <p class="setting-desc">未填写时自动使用模型名称（ID 自动生成，无需填写）</p>
          </div>
          <input v-model="form.name" type="text" placeholder="例如：OpenAI GPT-4o" />
        </div>

        <div class="setting-item">
          <div class="setting-info">
            <label>提供商 <span class="required">*</span></label>
            <p class="setting-desc">选择一个内置的 Model 提供商预设</p>
          </div>
          <select v-model="form.provider" @change="onFormProviderChange">
            <option v-for="k in providerKeys" :key="k" :value="k">{{ providerLabels[k] || k }}</option>
          </select>
        </div>

        <div class="setting-item">
          <div class="setting-info">
            <label>模型 <span class="required">*</span></label>
            <p class="setting-desc">选择或输入目标模型名</p>
          </div>
          <div class="input-wrap">
            <input
              v-model="form.model"
              type="text"
              :list="`models-${form.id || 'new'}`"
              placeholder="gpt-4o / claude-3-5-sonnet-latest / ..."
            />
            <datalist :id="`models-${form.id || 'new'}`">
              <option v-for="m in formAvailableModels" :key="m" :value="m" />
            </datalist>
          </div>
        </div>

        <div class="setting-item">
          <div class="setting-info">
            <label>API Base URL <span class="required">*</span></label>
            <p class="setting-desc">切换提供商时会自动填入对应端点</p>
          </div>
          <input v-model="form.api_base" type="text" placeholder="https://api.openai.com/v1" />
        </div>

        <div class="setting-item">
          <div class="setting-info">
            <label>API Key</label>
            <p class="setting-desc">保存在本机配置文件中</p>
          </div>
          <div class="api-key-row">
            <input
              v-model="form.api_key"
              :type="showApiKey ? 'text' : 'password'"
              placeholder="输入 API Key"
            />
            <button
              type="button"
              class="icon-btn"
              :title="showApiKey ? '隐藏' : '显示'"
              @click="showApiKey = !showApiKey"
            >
              <svg v-if="showApiKey" viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2">
                <path d="M17.94 17.94A10.07 10.07 0 0 1 12 20c-7 0-11-8-11-8a18.45 18.45 0 0 1 5.06-5.94M9.9 4.24A9.12 9.12 0 0 1 12 4c7 0 11 8 11 8a18.5 18.5 0 0 1-2.16 3.19m-6.72-1.07a3 3 0 1 1-4.24-4.24" />
                <line x1="1" y1="1" x2="23" y2="23" />
              </svg>
              <svg v-else viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2">
                <path d="M1 12s4-8 11-8 11 8 11 8-4 8-11 8-11-8-11-8z" />
                <circle cx="12" cy="12" r="3" />
              </svg>
            </button>
          </div>
        </div>

        <div class="setting-item">
          <div class="setting-info">
            <label>启用</label>
            <p class="setting-desc">禁用后该 Provider 在 Model 对话选项中不可选</p>
          </div>
          <label class="toggle">
            <input type="checkbox" v-model="form.enabled" />
            <span class="toggle-slider"></span>
          </label>
        </div>
      </div>

      <!-- 高级设置（默认折叠） -->
      <button type="button" class="advanced-toggle" @click="showAdvanced = !showAdvanced">
        <span class="chevron" :class="{ open: showAdvanced }">▸</span>
        高级设置
      </button>

      <div v-show="showAdvanced" class="setting-group advanced-group">
        <div class="setting-item">
          <div class="setting-info">
            <label>API 协议</label>
            <p class="setting-desc">与该 Provider 通信的协议风格</p>
          </div>
          <select v-model="form.api_protocol">
            <option v-for="p in formProtocols" :key="p" :value="p">{{ protocolLabels[p] || p }}</option>
          </select>
        </div>

        <div class="setting-item">
          <div class="setting-info">
            <label>Temperature</label>
            <p class="setting-desc">控制输出随机性（0 - 2）</p>
          </div>
          <input v-model.number="form.temperature" type="number" min="0" max="2" step="0.1" />
        </div>

        <div class="setting-item">
          <div class="setting-info">
            <label>Max Tokens</label>
            <p class="setting-desc">单次回复最大 token 数（留空使用默认）</p>
          </div>
          <input v-model.number="form.max_tokens" type="number" min="100" max="128000" placeholder="4096" />
        </div>

        <div class="setting-item">
          <div class="setting-info">
            <label>请求频率限制（毫秒）</label>
            <p class="setting-desc">两次请求之间的最小间隔；0 表示不限制</p>
          </div>
          <input v-model.number="form.rate_limit_ms" type="number" min="0" step="100" placeholder="0" />
        </div>

        <div class="setting-item column">
          <div class="setting-info">
            <label>系统提示词</label>
            <p class="setting-desc">应用于此 Provider 的全局系统提示词</p>
          </div>
          <textarea v-model="form.system_prompt" rows="3" placeholder="可选：默认 system prompt" />
        </div>
      </div>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import { providerPresets, providerLabels, protocolLabels } from '@/constants/modelProviders'
import type { ModelProviderConfig } from '@/schemas/model_providers'
import type { ResourceCapabilities, ResourceSummary } from '@/schemas/resources'
import { resourcePath } from '@/registry/resourceTypes'
import { useToast } from '@/composables/useToast'

const toast = useToast()

const props = withDefaults(
  defineProps<{
    /** 选中的资源（null = 新建模式） */
    item: ResourceSummary | null
    capabilities: ResourceCapabilities
    saving?: boolean
    testing?: boolean
    deleting?: boolean
    /** 新建模式下用于 ID 去重的现有 id 列表 */
    existingIds?: string[]
  }>(),
  { saving: false, testing: false, deleting: false, existingIds: () => [] }
)

const emit = defineEmits<{
  /** 统一保存入口：id 为资源目录名，manifest 为完整 Provider 配置 */
  save: [payload: { id: string; manifest: Record<string, unknown>; skipValidation: boolean }]
  test: []
  delete: []
  'set-default': []
  cancel: []
}>()

const form = reactive<ModelProviderConfig>({
  id: '',
  name: '',
  provider: 'openai',
  api_base: '',
  api_key: '',
  model: '',
  temperature: 0.7,
  max_tokens: 4096,
  api_protocol: 'openai_responses',
  system_prompt: '',
  rate_limit_ms: 0,
  enabled: true,
})

const formAvailableModels = ref<string[]>([])
const formProtocols = ref<string[]>([])
const showApiKey = ref(false)
const showAdvanced = ref(false)

const providerKeys = computed(() => Object.keys(providerPresets))
const isExisting = computed(() => Boolean(props.item?.id))
const isDefault = computed(() => Boolean(props.item && (props.item.is_default === true)))

/** 资源路径唯一标识：[provider]/[id].[kind]（仅已存在项展示） */
const resourcePathLabel = computed(() => {
  const it = props.item
  if (!it?.id) return ''
  return resourcePath(it.provider || it.kind, it.id, it.kind)
})

async function copyPath() {
  if (!resourcePathLabel.value) return
  try {
    await navigator.clipboard.writeText(resourcePathLabel.value)
    toast.showToast('success', '已复制资源路径')
  } catch {
    toast.showToast('info', `路径：${resourcePathLabel.value}`)
  }
}

const displayLabel = computed(() =>
  form.provider ? providerLabels[form.provider] || form.provider : ''
)

const statusDotClass = computed(() => {
  if (!form.enabled) return 'failed'
  if (isDefault.value) return 'running'
  return 'idle'
})

/** 名称/模型派生可读 slug，`<base>`、`<base>-2` … 递增去重 */
function generateId(base: string): string {
  const used = new Set(props.existingIds)
  const slug =
    base
      .trim()
      .toLowerCase()
      .replace(/[^a-z0-9-_]+/g, '-')
      .replace(/^-+|-+$/g, '') || 'provider'
  let id = slug
  let counter = 2
  while (used.has(id)) {
    id = `${slug}-${counter}`
    counter++
  }
  return id
}

function buildPayload(): ModelProviderConfig {
  return {
    ...form,
    id: form.id.trim(),
    // 名称未填写时自动取模型名称，其次取提供商展示名
    name: (form.name?.trim() || form.model.trim() || displayLabel.value || form.provider).trim(),
    rate_limit_ms: Math.max(0, Math.floor(form.rate_limit_ms ?? 0)),
  }
}

function emitSave(skipValidation: boolean) {
  let id = form.id.trim()
  if (!id) {
    // 新建：由名称/模型/提供商派生不冲突的 ID
    id = generateId(form.name || form.model || form.provider)
  }
  emit('save', { id, manifest: { ...buildPayload(), id }, skipValidation })
}

function refreshFormOptions(providerKey: string, opts: { fillModel?: boolean } = {}) {
  const preset = providerPresets[providerKey]
  if (preset) {
    if (!form.api_base) form.api_base = preset.apiBase
    if (opts.fillModel && !form.model && preset.models.length > 0) {
      form.model = preset.models[0]
    }
    formAvailableModels.value = preset.models
    formProtocols.value = preset.protocols
    if (preset.protocols.length > 0 && !preset.protocols.includes(form.api_protocol ?? '')) {
      form.api_protocol = preset.protocols[0]
    }
  } else {
    formAvailableModels.value = []
    formProtocols.value = ['openai_responses', 'openai_chat', 'anthropic_messages', 'gemini_api']
  }
}

function onFormProviderChange() {
  refreshFormOptions(form.provider, { fillModel: true })
}

watch(
  () => props.item,
  (it) => {
    // 从列表项 extra.config（后端 list_items 携带的完整配置）预填
    const cfg = (it?.config ?? null) as ModelProviderConfig | null
    if (cfg) {
      Object.assign(form, {
        id: cfg.id,
        name: cfg.name || cfg.id,
        provider: cfg.provider,
        api_base: cfg.api_base,
        api_key: cfg.api_key ?? '',
        model: cfg.model,
        temperature: cfg.temperature ?? 0.7,
        max_tokens: cfg.max_tokens ?? 4096,
        api_protocol: cfg.api_protocol ?? 'openai_responses',
        system_prompt: cfg.system_prompt ?? '',
        rate_limit_ms: cfg.rate_limit_ms ?? 0,
        enabled: cfg.enabled ?? true,
      })
      refreshFormOptions(cfg.provider)
    } else {
      // 新建模式：重置为默认表单
      Object.assign(form, {
        id: '',
        name: '',
        provider: 'openai',
        api_base: '',
        api_key: '',
        model: '',
        temperature: 0.7,
        max_tokens: 4096,
        api_protocol: 'openai_responses',
        system_prompt: '',
        rate_limit_ms: 0,
        enabled: true,
      })
      refreshFormOptions('openai', { fillModel: true })
    }
    showApiKey.value = false
  },
  { immediate: true }
)
</script>

<style scoped>
.provider-form {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
}

/* ============ 顶部 header：与左栏 panel-header 完全同构 ============ */
.form-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.5rem 1rem;
  border-bottom: 1px solid var(--border-default);
  flex-shrink: 0;
  gap: 0.75rem;
  background: var(--surface-panel);
}

.title-area {
  display: flex;
  align-items: center;
  gap: 0.65rem;
  min-width: 0;
  flex: 1 1 auto;
  overflow: hidden;
}

.status-dot {
  display: inline-block;
  width: 0.4375rem;
  height: 0.4375rem;
  border-radius: var(--radius-full);
  background: var(--text-disabled);
  flex-shrink: 0;
}
.status-dot.running { background: var(--success-solid); }
.status-dot.failed { background: var(--danger-solid); }

.title-block {
  display: flex;
  flex-direction: column;
  gap: 0.2rem;
  min-width: 0;
}

.title-line {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  min-width: 0;
}

.title-text {
  font-size: var(--font-size-md);
  font-weight: var(--font-weight-semibold);
  color: var(--text-primary);
  margin: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.subtitle {
  font-size: var(--font-size-xs);
  color: var(--text-muted);
  margin: 0;
  display: flex;
  align-items: center;
  gap: 0.3rem;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.provider-pill {
  display: inline-block;
  padding: 0 0.35rem;
  background: var(--surface-sunken);
  border-radius: var(--radius-xs);
  font-size: 0.65rem;
  color: var(--text-secondary);
  font-family: var(--font-mono);
  white-space: nowrap;
}

.dot { opacity: 0.5; }

.path-pill {
  cursor: pointer;
  transition: color var(--motion-fast) var(--motion-ease), background var(--motion-fast) var(--motion-ease);
}
.path-pill:hover {
  color: var(--text-primary);
  background: var(--surface-hover);
}

.badge {
  font-size: 0.65rem;
  padding: 0.1rem 0.45rem;
  border-radius: var(--radius-full);
  font-weight: var(--font-weight-medium);
  white-space: nowrap;
}
.badge.default { background: var(--success-bg); color: var(--success-fg); }
.badge.disabled { background: var(--surface-sunken); color: var(--text-muted); }

.header-actions {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  flex-shrink: 0;
  flex-wrap: wrap;
  justify-content: flex-end;
}

.header-actions-divider {
  display: inline-block;
  width: 1px;
  height: 1.125rem;
  background: var(--border-default);
  margin: 0 0.15rem;
  flex-shrink: 0;
}

/* ============ 表单主体 ============ */
.form-body {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 1rem 1.5rem;
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.setting-group {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}

.setting-item {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1.5rem;
  padding: 0.65rem 0;
  border-bottom: 1px solid var(--border-subtle);
}

.setting-item:last-child {
  border-bottom: none;
}

.setting-item.column {
  flex-direction: column;
  align-items: stretch;
  gap: 0.5rem;
}

.setting-info {
  flex: 1;
  min-width: 0;
}

.setting-info label {
  display: block;
  font-size: var(--font-size-base);
  font-weight: var(--font-weight-medium);
  color: var(--text-primary);
  margin: 0;
}

.setting-desc {
  font-size: 0.72rem;
  color: var(--text-muted);
  margin: 0.15rem 0 0;
}

.required { color: var(--danger-fg); }

/* 高级设置折叠开关 */
.advanced-toggle {
  display: inline-flex;
  align-items: center;
  gap: 0.4rem;
  margin: 0.6rem 0 0.15rem;
  align-self: flex-start;
  border: none;
  background: none;
  color: var(--text-secondary);
  cursor: pointer;
  font-size: var(--font-size-base);
  padding: 0.25rem 0.5rem;
  border-radius: var(--radius-md);
}
.advanced-toggle:hover {
  color: var(--accent);
  background: var(--surface-hover);
}
.chevron {
  display: inline-block;
  transition: transform var(--motion-fast) var(--motion-ease);
  font-size: 0.75rem;
}
.chevron.open {
  transform: rotate(90deg);
}
.advanced-group {
  padding-left: 0.15rem;
}

/* 表单控件 */
.setting-item input,
.setting-item select,
.setting-item textarea,
.api-key-row input {
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  padding: 0.4rem 0.55rem;
  font-size: var(--font-size-base);
  background: var(--surface-sunken);
  color: var(--text-primary);
  font-family: inherit;
  width: 17.5rem;
  max-width: 100%;
  box-sizing: border-box;
  transition: border-color var(--motion-fast) var(--motion-ease),
    box-shadow var(--motion-fast) var(--motion-ease);
}

.setting-item.column input,
.setting-item.column select,
.setting-item.column textarea {
  width: 100%;
}

.setting-item input:focus,
.setting-item select:focus,
.setting-item textarea:focus,
.api-key-row input:focus {
  outline: none;
  border-color: var(--accent);
  box-shadow: 0 0 0 2px var(--accent-subtle-bg);
}

.setting-item input:disabled,
.setting-item select:disabled {
  background: var(--surface-sunken);
  color: var(--text-muted);
  cursor: not-allowed;
}

.setting-item textarea {
  resize: vertical;
  min-height: 3.75rem;
  font-family: inherit;
}

.input-wrap { display: flex; }

.api-key-row {
  display: flex;
  gap: 0.35rem;
  align-items: center;
}
.api-key-row input { flex: 1; min-width: 0; }

.icon-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 1.625rem;
  height: 1.625rem;
  border: none;
  background: transparent;
  border-radius: var(--radius-md);
  cursor: pointer;
  color: var(--text-secondary);
  transition: all var(--motion-fast) var(--motion-ease);
  flex-shrink: 0;
}
.icon-btn:hover:not(:disabled) {
  background: var(--surface-hover);
  color: var(--text-primary);
}
.icon-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
.icon-btn.danger:hover:not(:disabled) {
  background: var(--danger-bg);
  color: var(--danger-solid);
}

/* Toggle */
.toggle {
  position: relative;
  display: inline-block;
  width: 2.25rem;
  height: 1.25rem;
  cursor: pointer;
  flex-shrink: 0;
}
.toggle input { opacity: 0; width: 0; height: 0; }
.toggle-slider {
  position: absolute;
  inset: 0;
  background: var(--border-strong);
  border-radius: var(--radius-full);
  transition: background var(--motion-base) var(--motion-ease);
}
.toggle-slider::before {
  content: '';
  position: absolute;
  width: 1rem;
  height: 1rem;
  left: var(--space-05);
  top: var(--space-05);
  background: var(--surface-panel);
  border-radius: 50%;
  transition: transform var(--motion-base) var(--motion-ease);
}
.toggle input:checked + .toggle-slider { background: var(--accent); }
.toggle input:checked + .toggle-slider::before { transform: translateX(1rem); }

/* ============ 操作按钮 ============ */
.action-btn {
  display: inline-flex;
  align-items: center;
  gap: 0.4rem;
  background: var(--accent);
  color: var(--text-on-accent);
  border: none;
  border-radius: var(--radius-md);
  padding: 0.35rem 0.75rem;
  cursor: pointer;
  font-size: 0.8rem;
  white-space: nowrap;
  transition: background var(--motion-fast) var(--motion-ease), opacity var(--motion-fast) var(--motion-ease);
}
.action-btn:hover:not(:disabled) { background: var(--accent-hover); }
.action-btn:disabled { opacity: 0.5; cursor: not-allowed; }
.action-btn.secondary {
  background: transparent;
  color: var(--text-primary);
  border: 1px solid var(--border-default);
}
.action-btn.secondary:hover:not(:disabled) {
  background: var(--surface-hover);
}

@media (max-width: 45rem) {
  .setting-item { flex-direction: column; align-items: stretch; gap: 0.4rem; }
  .setting-item input,
  .setting-item select,
  .setting-item textarea,
  .api-key-row input { width: 100%; }
}
</style>
