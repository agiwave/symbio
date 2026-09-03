<!--
  ModelProvidersSettings — Provider 详情编辑器

  视觉风格对齐 SettingsPage（setting-item / panel-header / action-btn）
  + 左栏 ModelProviderCard 的徽标 / 状态点语言。
-->
<template>
  <div class="provider-editor">
    <!-- 空状态 -->
    <div v-if="!provider" class="empty-detail">
      <div class="empty-icon">🧠</div>
      <h3>选择一个 Provider 查看或编辑</h3>
      <p>在左侧列表中选择一个 Provider，或点击右上角「+」新建。</p>
    </div>

    <template v-else>
      <!-- 顶部面板：标题 + 操作（操作按钮全部集中在右上角） -->
      <header class="panel-header editor-panel-header">
        <div class="title-area">
          <span class="status-dot" :class="statusDotClass" />
          <div class="title-block">
            <div class="title-line">
              <h2 class="title-text">{{ form.name || form.model || displayLabel || '新建 Provider' }}</h2>
              <span v-if="isDefault" class="badge default">默认</span>
              <span v-if="!form.enabled" class="badge disabled">已停用</span>
            </div>
            <p class="subtitle">
              <code class="provider-pill">{{ displayLabel }}</code>
              <span class="dot">·</span>
              <span>{{ form.model || '未设置模型' }}</span>
            </p>
          </div>
        </div>
        <div class="header-actions">
          <!-- 主要操作（右上角） -->
          <button
            type="button"
            class="action-btn secondary"
            :disabled="saving || testing"
            @click="$emit('test', buildPayload())"
          >
            {{ testing ? '校验中…' : '校验连接' }}
          </button>
          <button
            type="button"
            class="action-btn secondary"
            :disabled="saving"
            @click="$emit('save', { provider: buildPayload(), skipValidation: true })"
          >
            跳过校验保存
          </button>
          <button
            type="button"
            class="action-btn"
            :disabled="saving"
            @click="$emit('save', { provider: buildPayload(), skipValidation: false })"
          >
            {{ saving ? '保存中…' : '保存' }}
          </button>

          <!-- 次要操作（图标按钮，紧贴主操作右侧） -->
          <span class="header-actions-divider" aria-hidden="true" />
          <button
            v-if="!isDefault && form.id"
            class="icon-btn"
            :title="`将 ${form.id} 设为默认`"
            @click="$emit('setDefault', form.id)"
          >
            <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2">
              <polygon points="12 2 15 8.5 22 9.3 17 14 18.2 21 12 17.8 5.8 21 7 14 2 9.3 9 8.5 12 2" />
            </svg>
          </button>
          <button
            class="icon-btn danger"
            :disabled="!isExisting || deleting"
            :title="deleting ? '删除中…' : isExisting ? '删除 Provider' : '保存后才能删除'"
            @click="$emit('delete', provider)"
          >
            <svg viewBox="0 0 24 24" width="14" height="14" fill="none" stroke="currentColor" stroke-width="2">
              <polyline points="3 6 5 6 21 6" />
              <path d="M19 6v14a2 2 0 0 1-2 2H7a2 2 0 0 1-2-2V6m3 0V4a2 2 0 0 1 2-2h4a2 2 0 0 1 2 2v2" />
            </svg>
          </button>
        </div>
      </header>

      <!-- 表单主体 -->
      <div class="editor-body">
        <!-- 快速配置组：最常用字段保持可见 -->
        <div class="setting-group">
          <div class="setting-item">
            <div class="setting-info">
              <label>名称</label>
              <p class="setting-desc">未填写时自动使用模型名称（ID 自动生成，无需填写）</p>
            </div>
            <input
              v-model="form.name"
              type="text"
              placeholder="例如：OpenAI GPT-4o"
            />
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

          <div class="setting-item">
            <div class="setting-info">
              <label>API Base URL <span class="required">*</span></label>
              <p class="setting-desc">切换提供商时会自动填入对应端点</p>
            </div>
            <input
              v-model="form.api_base"
              type="text"
              placeholder="https://api.openai.com/v1"
            />
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

        <!-- 高级设置（默认折叠，点击展开） -->
        <button
          type="button"
          class="advanced-toggle"
          @click="showAdvanced = !showAdvanced"
        >
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
              <option v-for="p in formProtocols" :key="p" :value="p">
                {{ protocolLabels[p] || p }}
              </option>
            </select>
          </div>

          <div class="setting-item">
            <div class="setting-info">
              <label>Temperature</label>
              <p class="setting-desc">控制输出随机性（0 - 2）</p>
            </div>
            <input
              v-model.number="form.temperature"
              type="number"
              min="0"
              max="2"
              step="0.1"
            />
          </div>

          <div class="setting-item">
            <div class="setting-info">
              <label>Max Tokens</label>
              <p class="setting-desc">单次回复最大 token 数（留空使用默认）</p>
            </div>
            <input
              v-model.number="form.max_tokens"
              type="number"
              min="100"
              max="128000"
              placeholder="4096"
            />
          </div>

          <div class="setting-item">
            <div class="setting-info">
              <label>请求频率限制（毫秒）</label>
              <p class="setting-desc">两次请求之间的最小间隔；0 表示不限制</p>
            </div>
            <input
              v-model.number="form.rate_limit_ms"
              type="number"
              min="0"
              step="100"
              placeholder="0"
            />
          </div>

          <div class="setting-item column">
            <div class="setting-info">
              <label>系统提示词</label>
              <p class="setting-desc">应用于此 Provider 的全局系统提示词</p>
            </div>
            <textarea
              v-model="form.system_prompt"
              rows="3"
              placeholder="可选：默认 system prompt"
            />
          </div>
        </div>
      </div>
    </template>
  </div>
</template>

<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import { providerPresets, providerLabels, protocolLabels } from '@/constants/modelProviders'
import type { ModelProviderConfig } from '@/schemas/model_providers'

const props = defineProps<{
  provider: ModelProviderConfig | null
  isDefault?: boolean
  saving?: boolean
  testing?: boolean
  deleting?: boolean
}>()

const emit = defineEmits<{
  save: [payload: { provider: ModelProviderConfig; skipValidation: boolean }]
  test: [provider: ModelProviderConfig]
  delete: [provider: ModelProviderConfig]
  setDefault: [providerId: string]
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
  enabled: true
})

const formAvailableModels = ref<string[]>([])
const formProtocols = ref<string[]>([])
const showApiKey = ref(false)
const showAdvanced = ref(false)

const providerKeys = computed(() => Object.keys(providerPresets))
const isExisting = computed(() => Boolean(props.provider && props.provider.id))

/** 提供商的展示名（含中文映射，未知标识回退原始 key） */
const displayLabel = computed(() => (form.provider ? providerLabels[form.provider] || form.provider : ''))

const statusDotClass = computed(() => {
  if (!form.enabled) return 'failed'
  if (props.isDefault) return 'running'
  return 'idle'
})

function buildPayload(): ModelProviderConfig {
  return {
    ...form,
    id: form.id.trim(),
    // 名称未填写时自动取模型名称，其次取提供商展示名
    name: (form.name?.trim() || form.model.trim() || displayLabel.value || form.provider).trim(),
    rate_limit_ms: Math.max(0, Math.floor(form.rate_limit_ms ?? 0))
  }
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
  // 用户在"切换提供商"时自动填充模型与端点（编辑状态不覆盖已填值）
  refreshFormOptions(form.provider, { fillModel: true })
}

watch(
  () => props.provider,
  (p) => {
    if (p) {
      Object.assign(form, {
        id: p.id,
        name: p.name || p.id,
        provider: p.provider,
        api_base: p.api_base,
        api_key: p.api_key ?? '',
        model: p.model,
        temperature: p.temperature ?? 0.7,
        max_tokens: p.max_tokens ?? 4096,
        api_protocol: p.api_protocol ?? 'openai_responses',
        system_prompt: p.system_prompt ?? '',
        rate_limit_ms: p.rate_limit_ms ?? 0,
        enabled: p.enabled ?? true
      })
      showApiKey.value = false
      refreshFormOptions(p.provider)
    }
  },
  { immediate: true }
)
</script>

<style scoped>
.provider-editor {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  background: var(--color-bg);
}

/* ============ 顶部 header：与左栏 panel-header 完全同构 ============ */
.editor-panel-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 0.5rem 1rem;
  border-bottom: 1px solid var(--color-border);
  flex-shrink: 0;
  gap: 0.75rem;
  background: var(--color-bg);
}

.title-area {
  display: flex;
  align-items: center;
  gap: 0.65rem;
  min-width: 0;
  flex: 1 1 auto;
  overflow: hidden;
}

.title-area .status-dot {
  flex-shrink: 0;
}

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
  font-size: 1rem;
  font-weight: 600;
  color: var(--color-text);
  margin: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.subtitle {
  font-size: 0.75rem;
  color: var(--color-text-muted);
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
  background: rgba(0, 0, 0, 0.05);
  border-radius: 0.1875rem;
  font-size: 0.65rem;
  color: var(--color-text-secondary);
  font-family: var(--font-mono, 'JetBrains Mono', Consolas, monospace);
  white-space: nowrap;
}

.dot { opacity: 0.5; }

.badge {
  font-size: 0.65rem;
  padding: 0.1rem 0.45rem;
  border-radius: 62.4375rem;
  font-weight: 500;
  white-space: nowrap;
}
.badge.default { background: rgba(34, 197, 94, 0.12); color: #15803d; }
.badge.disabled { background: rgba(0, 0, 0, 0.06); color: var(--color-text-muted); }

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
  background: var(--color-border);
  margin: 0 0.15rem;
  flex-shrink: 0;
}

/* ============ 表单主体（与 SettingsPage 完全同构） ============ */
.editor-body {
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
  border-bottom: 1px solid var(--color-border-subtle, rgba(0, 0, 0, 0.04));
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
  font-size: 0.85rem;
  font-weight: 500;
  color: var(--color-text);
  margin: 0;
}

.setting-desc {
  font-size: 0.72rem;
  color: var(--color-text-muted);
  margin: 0.15rem 0 0;
}

.required { color: #dc2626; }

.setting-divider {
  height: 1px;
  background: var(--color-border);
  margin: 0.5rem 0;
}

/* 高级设置折叠开关 */
.advanced-toggle {
  display: inline-flex;
  align-items: center;
  gap: 0.4rem;
  margin: 0.6rem 0 0.15rem;
  align-self: flex-start;
  border: none;
  background: none;
  color: var(--color-text-secondary);
  cursor: pointer;
  font-size: 0.85rem;
  padding: 0.25rem 0.5rem;
  border-radius: 0.375rem;
}
.advanced-toggle:hover {
  color: var(--color-primary, #4f46e5);
  background: rgba(0, 0, 0, 0.04);
}
.chevron {
  display: inline-block;
  transition: transform 0.15s;
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
  border: 1px solid var(--color-border);
  border-radius: 0.375rem;
  padding: 0.4rem 0.55rem;
  font-size: 0.85rem;
  background: var(--color-bg, #fff);
  color: var(--color-text);
  font-family: inherit;
  width: 17.5rem;
  max-width: 100%;
  box-sizing: border-box;
  transition: border-color 0.12s, box-shadow 0.12s;
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
  border-color: var(--color-primary, #4f46e5);
  box-shadow: 0 0 0 2px rgba(79, 70, 229, 0.15);
}

.setting-item input:disabled,
.setting-item select:disabled {
  background: rgba(0, 0, 0, 0.03);
  color: var(--color-text-muted);
  cursor: not-allowed;
}

.setting-item textarea {
  resize: vertical;
  min-height: 3.75rem;
  font-family: inherit;
}

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
  border-radius: 0.375rem;
  cursor: pointer;
  color: var(--color-text-secondary);
  transition: all 0.15s;
  flex-shrink: 0;
}
.icon-btn:hover:not(:disabled) {
  background: rgba(0, 0, 0, 0.06);
  color: var(--color-text);
}
.icon-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}
.icon-btn.danger:hover:not(:disabled) {
  background: rgba(239, 68, 68, 0.1);
  color: #ef4444;
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
  background: #d1d5db;
  border-radius: 62.4375rem;
  transition: background 0.2s;
}
.toggle-slider::before {
  content: '';
  position: absolute;
  width: 1rem;
  height: 1rem;
  left: var(--space-05);
  top: var(--space-05);
  background: #fff;
  border-radius: 50%;
  transition: transform 0.2s;
}
.toggle input:checked + .toggle-slider { background: var(--color-primary, #4f46e5); }
.toggle input:checked + .toggle-slider::before { transform: translateX(1rem); }

/* ============ 操作按钮（header 右上角） ============ */
.action-btn {
  display: inline-flex;
  align-items: center;
  gap: 0.4rem;
  background: var(--color-primary, #4f46e5);
  color: #fff;
  border: none;
  border-radius: 0.375rem;
  padding: 0.35rem 0.75rem;
  cursor: pointer;
  font-size: 0.8rem;
  white-space: nowrap;
  transition: background 0.15s, opacity 0.15s;
}
.action-btn:hover:not(:disabled) { background: var(--color-primary-hover, #4338ca); }
.action-btn:disabled { opacity: 0.5; cursor: not-allowed; }
.action-btn.secondary {
  background: transparent;
  color: var(--color-text);
  border: 1px solid var(--color-border);
}
.action-btn.secondary:hover:not(:disabled) {
  background: rgba(0, 0, 0, 0.04);
}

/* ============ 空状态 ============ */
.empty-detail {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  height: 100%;
  color: var(--color-text-muted);
  text-align: center;
  gap: 0.4rem;
  padding: 1rem;
}
.empty-icon { font-size: 2.5rem; opacity: 0.5; }
.empty-detail h3 {
  color: var(--color-text);
  margin: 0;
  font-weight: 600;
  font-size: 0.95rem;
}
.empty-detail p { font-size: 0.8rem; margin: 0; }

@media (max-width: 45rem) {
  .setting-item { flex-direction: column; align-items: stretch; gap: 0.4rem; }
  .setting-item input,
  .setting-item select,
  .setting-item textarea,
  .api-key-row input { width: 100%; }
}
</style>