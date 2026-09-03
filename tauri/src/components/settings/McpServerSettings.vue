<!--
  McpServerSettings — MCP Server 详情编辑器

  视觉风格对齐 ModelProvidersSettings（setting-item / panel-header / action-btn）
  + 左栏 McpServerCard 的状态点语言。
-->
<template>
  <div class="server-editor">
    <!-- 空状态 -->
    <div v-if="!name" class="empty-detail">
      <div class="empty-icon">🔌</div>
      <h3>选择一个 MCP Server 查看或编辑</h3>
      <p>在左侧列表中选择一个 Server，或点击右上角「+」新建。</p>
    </div>

    <template v-else>
      <!-- 顶部面板：标题 + 操作（操作按钮全部集中在右上角） -->
      <header class="panel-header editor-panel-header">
        <div class="title-area">
          <span class="status-dot" :class="statusDotClass" />
          <div class="title-block">
            <div class="title-line">
              <h2 class="title-text">{{ name }}</h2>
              <span v-if="!form.enabled" class="badge disabled">已停用</span>
            </div>
            <p class="subtitle">
              <code class="command-pill">{{ transportSummary }}</code>
            </p>
          </div>
        </div>
        <div class="header-actions">
          <!-- 主要操作（右上角） -->
          <button
            type="button"
            class="action-btn secondary"
            :disabled="!canTest || saving || testing"
            :title="!isExisting ? '请先保存该 Server' : ''"
            @click="onTestClick"
          >
            {{ testing ? '测试中…' : '测试连接' }}
          </button>
          <button
            type="button"
            class="action-btn"
            :disabled="saving"
            @click="onSaveClick"
          >
            {{ saving ? '保存中…' : '保存' }}
          </button>

          <!-- 次要操作（图标按钮，紧贴主操作右侧） -->
          <span class="header-actions-divider" aria-hidden="true" />
          <button
            class="icon-btn danger"
            :disabled="!isExisting || deleting"
            :title="deleting ? '删除中…' : isExisting ? '删除 Server' : '保存后才能删除'"
            @click="$emit('delete', name)"
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
        <!-- 基本信息组 -->
        <div class="setting-group">
          <div class="setting-item">
            <div class="setting-info">
              <label>名称 <span class="required">*</span></label>
              <p class="setting-desc">Server 的唯一标识，保存后不可修改</p>
            </div>
            <input
              v-model="formName"
              type="text"
              placeholder="filesystem"
              :disabled="isExisting"
            />
          </div>

          <div class="setting-item">
            <div class="setting-info">
              <label>启用</label>
              <p class="setting-desc">禁用后该 Server 不会出现在 AI 工具列表中</p>
            </div>
            <label class="toggle">
              <input type="checkbox" v-model="form.enabled" />
              <span class="toggle-slider"></span>
            </label>
          </div>
        </div>

        <div class="setting-divider"></div>

        <!-- Transport 配置组 -->
        <div class="setting-group">
          <div class="setting-item">
            <div class="setting-info">
              <label>传输类型 <span class="required">*</span></label>
              <p class="setting-desc">stdio 启动本地子进程；http/sse 调用远端 HTTP 服务</p>
            </div>
            <select v-model="form.type">
              <option value="stdio">stdio（本地子进程）</option>
              <option value="http">http（REST API）</option>
              <option value="sse">sse（Server-Sent Events）</option>
            </select>
          </div>

          <!-- stdio: 命令 / 参数 / 环境变量 -->
          <template v-if="form.type === 'stdio'">
            <div class="setting-item">
              <div class="setting-info">
                <label>命令 <span class="required">*</span></label>
                <p class="setting-desc">启动该 MCP Server 的可执行文件路径</p>
              </div>
              <div class="field-with-error">
                <input
                  v-model="form.command"
                  type="text"
                  placeholder="/path/to/mcp-server 或 npx"
                  :class="{ invalid: fieldErrors.command }"
                  @blur="validateField('command')"
                />
                <span v-if="fieldErrors.command" class="field-error">
                  {{ fieldErrors.command }}
                </span>
              </div>
            </div>

            <div class="setting-item column">
              <div class="setting-info">
                <label>参数</label>
                <p class="setting-desc">每行一个参数（或使用空格分隔）</p>
              </div>
              <textarea
                :value="argsString"
                @input="setArgsFromString(($event.target as HTMLTextAreaElement).value)"
                rows="3"
                placeholder="--stdio&#10;--option value"
              />
            </div>

            <div class="setting-item column">
              <div class="setting-info">
                <label>环境变量</label>
                <p class="setting-desc">每行一个 <code>KEY=value</code>，运行时注入到 Server 进程</p>
              </div>
              <textarea
                :value="envString"
                @input="setEnvFromString(($event.target as HTMLTextAreaElement).value)"
                rows="4"
                placeholder="API_KEY=xxx&#10;DEBUG=true"
              />
            </div>
          </template>

          <!-- http / sse: URL -->
          <template v-else>
            <div class="setting-item">
              <div class="setting-info">
                <label>URL <span class="required">*</span></label>
                <p class="setting-desc">远端 MCP Server 的入口地址（如 <code>http://localhost:8080/mcp</code>）</p>
              </div>
              <div class="field-with-error">
                <input
                  v-model="form.url"
                  type="text"
                  placeholder="http://localhost:8080/mcp"
                  :class="{ invalid: fieldErrors.url }"
                  @blur="validateField('url')"
                />
                <span v-if="fieldErrors.url" class="field-error">
                  {{ fieldErrors.url }}
                </span>
              </div>
            </div>

            <!-- BUG-MR28：HTTP 自定义请求头 -->
            <div class="setting-item column">
              <div class="setting-info">
                <label>自定义请求头</label>
                <p class="setting-desc">
                  每行一个 <code>KEY=value</code>（如 <code>Authorization=Bearer xxx</code>）。
                  保留头 <code>Content-Type / Accept / Mcp-Session-Id</code> 由 client 管理，会覆盖用户配置。
                </p>
              </div>
              <textarea
                :value="headersString"
                @input="setHeadersFromString(($event.target as HTMLTextAreaElement).value)"
                rows="3"
                placeholder="Authorization=Bearer xxx&#10;X-Custom-Header=value"
              />
            </div>

            <!-- BUG-MR31：HTTP 超时配置 -->
            <div class="setting-item">
              <div class="setting-info">
                <label>请求超时（秒）</label>
                <p class="setting-desc">默认 30s。空 = 使用默认值</p>
              </div>
              <input
                v-model.number="timeoutSecsInput"
                type="number"
                min="1"
                placeholder="30"
                style="max-width: 7.5rem;"
              />
            </div>
          </template>
        </div>

        <div class="setting-divider"></div>

        <!-- 工具过滤组 -->
        <div class="setting-group">
          <div class="setting-item column">
            <div class="setting-info">
              <label>白名单（include_tools）</label>
              <p class="setting-desc">每行一个工具名；填写后只暴露这些工具</p>
            </div>
            <textarea
              :value="includeToolsString"
              @input="setIncludeToolsFromString(($event.target as HTMLTextAreaElement).value)"
              rows="3"
              placeholder="search&#10;fetch"
            />
          </div>
          <div class="setting-item column">
            <div class="setting-info">
              <label>黑名单（exclude_tools）</label>
              <p class="setting-desc">每行一个工具名；优先级高于白名单</p>
            </div>
            <textarea
              :value="excludeToolsString"
              @input="setExcludeToolsFromString(($event.target as HTMLTextAreaElement).value)"
              rows="3"
              placeholder="dangerous_tool"
            />
          </div>
        </div>
      </div>
    </template>
  </div>
</template>

<script setup lang="ts">
import { computed, reactive, ref, watch } from 'vue'
import type { McpServerConfig, McpTransportType } from '@/schemas/mcp_config'

const props = defineProps<{
  /** 选中的 Server 名称；为 null 时显示空状态 */
  name: string | null
  /** Server 详细配置；新建时可能为 null（使用空表单） */
  server: McpServerConfig | null
  saving?: boolean
  testing?: boolean
  deleting?: boolean
}>()

const emit = defineEmits<{
  save: [payload: { name: string; server: McpServerConfig }]
  delete: [name: string]
  test: [payload: { name: string; server: McpServerConfig }]
}>()

/** 可编辑的 Server name（新建时可改，已存在时锁死） */
const formName = ref('')

const form = reactive<McpServerConfig>({
  type: 'stdio',
  command: '',
  args: [],
  env: {},
  url: '',
  headers: undefined,
  include_tools: [],
  exclude_tools: [],
  timeout_secs: undefined,
  enabled: true
})

/** BUG-MR31：HTTP 超时输入（空字符串 = 使用默认） */
const timeoutSecsInput = ref<number | null>(null)

/** BUG-FR6：失焦字段错误（key = 字段名，value = 错误消息） */
const fieldErrors = reactive<Record<string, string>>({})

const isExisting = computed(() => Boolean(props.name))

/** BUG-FR7：能否测试连接（必须已保存 + 必填字段已填） */
const canTest = computed(() => {
  if (!isExisting.value) return false
  const t = form.type ?? 'stdio'
  if (t === 'stdio') return Boolean(form.command?.trim())
  if (t === 'http' || t === 'sse') return Boolean(form.url?.trim())
  return false
})

const statusDotClass = computed(() => {
  return form.enabled ? 'running' : 'failed'
})

/** 在标题区显示的简要标识（命令 或 URL） */
const transportSummary = computed(() => {
  if (form.type === 'stdio') {
    return form.command || '未设置命令'
  }
  return form.url || '未设置 URL'
})

const argsString = computed(() => (form.args ?? []).join('\n'))
const envString = computed(() =>
  Object.entries(form.env ?? {})
    .map(([k, v]) => `${k}=${v}`)
    .join('\n')
)
const includeToolsString = computed(() => (form.include_tools ?? []).join('\n'))
const excludeToolsString = computed(() => (form.exclude_tools ?? []).join('\n'))

/** BUG-MR28：自定义 headers 的多行字符串表示 */
const headersString = computed(() =>
  Object.entries(form.headers ?? {})
    .map(([k, v]) => `${k}=${v}`)
    .join('\n')
)

/** BUG-MR28：把 textarea 内容解析为 headers 字典 */
function setHeadersFromString(v: string) {
  const headers: Record<string, string> = {}
  v.split('\n').forEach((line) => {
    const idx = line.indexOf('=')
    if (idx > 0) {
      const key = line.substring(0, idx).trim()
      const value = line.substring(idx + 1).trim()
      if (key) headers[key] = value
    }
  })
  form.headers = Object.keys(headers).length > 0 ? headers : undefined
}

function setArgsFromString(v: string) {
  const parts = v
    .split(/[\n\s]+/)
    .map((s) => s.trim())
    .filter((s) => s.length > 0)
  form.args = parts
}

function setEnvFromString(v: string) {
  const env: Record<string, string> = {}
  v.split('\n').forEach((line) => {
    const idx = line.indexOf('=')
    if (idx > 0) {
      const key = line.substring(0, idx).trim()
      const value = line.substring(idx + 1).trim()
      if (key) env[key] = value
    }
  })
  form.env = env
}

function setToolsFromString(v: string): string[] {
  return v
    .split('\n')
    .map((s) => s.trim())
    .filter((s) => s.length > 0)
}

function setIncludeToolsFromString(v: string) {
  form.include_tools = setToolsFromString(v)
}

function setExcludeToolsFromString(v: string) {
  form.exclude_tools = setToolsFromString(v)
}

/** BUG-FR6：校验单个字段（失焦时调用） */
function validateField(field: 'command' | 'url' | 'name') {
  delete fieldErrors[field]
  if (field === 'command') {
    if (form.type === 'stdio' && !form.command?.trim()) {
      fieldErrors.command = '请填写启动命令'
    }
  } else if (field === 'url') {
    if ((form.type === 'http' || form.type === 'sse') && !form.url?.trim()) {
      fieldErrors.url = '请填写 URL'
    } else if (form.url?.trim()) {
      // 简单 URL 格式校验
      try {
        new URL(form.url.trim())
      } catch {
        fieldErrors.url = 'URL 格式不合法（需以 http:// 或 https:// 开头）'
      }
    }
  } else if (field === 'name') {
    if (!formName.value.trim()) {
      fieldErrors.name = '请填写名称'
    }
  }
}

/** 校验所有必填字段，返回是否全部通过 */
function validateAll(): boolean {
  validateField('command')
  validateField('url')
  validateField('name')
  return Object.keys(fieldErrors).length === 0
}

function buildPayload(): { name: string; server: McpServerConfig } {
  const t: McpTransportType = form.type ?? 'stdio'
  const payload: McpServerConfig = {
    type: t,
    enabled: form.enabled
  }
  if (t === 'stdio') {
    payload.command = form.command?.trim() || undefined
    payload.args = form.args ? [...form.args] : undefined
    payload.env = form.env ? { ...form.env } : undefined
  } else {
    payload.url = form.url?.trim() || undefined
    // BUG-MR28：自定义 headers
    if (form.headers && Object.keys(form.headers).length > 0) {
      payload.headers = { ...form.headers }
    }
    // BUG-MR31：超时配置
    if (timeoutSecsInput.value && timeoutSecsInput.value > 0) {
      payload.timeout_secs = timeoutSecsInput.value
    }
  }
  if (form.include_tools && form.include_tools.length > 0) {
    payload.include_tools = [...form.include_tools]
  }
  if (form.exclude_tools && form.exclude_tools.length > 0) {
    payload.exclude_tools = [...form.exclude_tools]
  }
  return {
    name: formName.value.trim(),
    server: payload
  }
}

function onSaveClick() {
  if (!validateAll()) return
  emit('save', buildPayload())
}

function onTestClick() {
  if (!validateAll()) return
  emit('test', buildPayload())
}

watch(
  () => [props.name, props.server] as const,
  ([n, s]) => {
    formName.value = n ?? ''
    // 切换 server 时清空之前的错误
    Object.keys(fieldErrors).forEach((k) => delete fieldErrors[k])
    if (s) {
      Object.assign(form, {
        type: s.type ?? 'stdio',
        command: s.command ?? '',
        args: [...(s.args ?? [])],
        env: { ...(s.env ?? {}) },
        url: s.url ?? '',
        headers: s.headers ? { ...s.headers } : undefined,
        include_tools: [...(s.include_tools ?? [])],
        exclude_tools: [...(s.exclude_tools ?? [])],
        enabled: s.enabled ?? true
      })
      timeoutSecsInput.value = s.timeout_secs ?? null
    } else {
      // 新建：重置为空白表单（保留 enabled=true, type=stdio）
      Object.assign(form, {
        type: 'stdio',
        command: '',
        args: [],
        env: {},
        url: '',
        headers: undefined,
        include_tools: [],
        exclude_tools: [],
        enabled: true
      })
      timeoutSecsInput.value = null
    }
  },
  { immediate: true }
)
</script>

<style scoped>
.server-editor {
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
  gap: 0.4rem;
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

.command-pill {
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

/* ============ 通用按钮 ============ */
.icon-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 1.75rem;
  height: 1.75rem;
  border: none;
  background: transparent;
  border-radius: 0.375rem;
  cursor: pointer;
  color: var(--color-text-secondary);
  transition: all 0.15s;
}

.icon-btn:hover:not(:disabled) {
  background: rgba(0, 0, 0, 0.06);
  color: var(--color-text);
}

.icon-btn:disabled {
  opacity: 0.4;
  cursor: not-allowed;
}

.icon-btn.danger {
  color: #ef4444;
}

.icon-btn.danger:hover:not(:disabled) {
  background: rgba(239, 68, 68, 0.1);
}

.action-btn {
  padding: 0.35rem 0.75rem;
  font-size: 0.8rem;
  border: 1px solid var(--color-primary);
  background: var(--color-primary);
  color: #fff;
  border-radius: 0.375rem;
  cursor: pointer;
  font-weight: 500;
  transition: all 0.15s;
  white-space: nowrap;
}

.action-btn:hover:not(:disabled) {
  background: var(--color-primary-dark);
  border-color: var(--color-primary-dark);
}

.action-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

/* ============ 空状态 ============ */
.empty-detail {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  color: var(--color-text-muted);
  padding: 2rem;
  text-align: center;
  gap: 0.5rem;
}

.empty-icon {
  font-size: 3rem;
  opacity: 0.5;
}

.empty-detail h3 {
  margin: 0;
  font-size: 1rem;
  font-weight: 500;
}

.empty-detail p {
  margin: 0;
  font-size: 0.85rem;
  max-width: 22.5rem;
}

/* ============ 表单主体 ============ */
.editor-body {
  flex: 1;
  overflow-y: auto;
  padding: 1rem 1.25rem 1.5rem;
}

.setting-group {
  display: flex;
  flex-direction: column;
  gap: 0.6rem;
}

.setting-item {
  display: flex;
  align-items: center;
  gap: 1rem;
  padding: 0.5rem 0;
  border-bottom: 1px solid var(--color-border-subtle, rgba(0, 0, 0, 0.04));
}

.setting-item.column {
  flex-direction: column;
  align-items: stretch;
}

.setting-item:last-child {
  border-bottom: none;
}

.setting-info {
  flex: 0 0 13.75rem;
  min-width: 0;
}

.setting-item.column .setting-info {
  flex: 0 0 auto;
  margin-bottom: 0.4rem;
}

.setting-info label {
  display: block;
  font-size: 0.85rem;
  font-weight: 500;
  color: var(--color-text);
  margin-bottom: 0.2rem;
}

.setting-info .required {
  color: #ef4444;
}

.setting-desc {
  font-size: 0.72rem;
  color: var(--color-text-muted);
  margin: 0;
  line-height: 1.4;
}

.setting-desc code {
  font-family: var(--font-mono, 'JetBrains Mono', Consolas, monospace);
  background: rgba(0, 0, 0, 0.05);
  padding: 0 0.25rem;
  border-radius: 0.1875rem;
  font-size: 0.7rem;
}

.setting-item input[type="text"],
.setting-item input[type="number"],
.setting-item select,
.setting-item textarea {
  flex: 1;
  min-width: 0;
  padding: 0.4rem 0.6rem;
  border: 1px solid var(--color-border);
  border-radius: 0.25rem;
  background: var(--color-bg);
  color: var(--color-text);
  font-size: 0.85rem;
  font-family: inherit;
  font-family: var(--font-mono, 'JetBrains Mono', Consolas, monospace);
}

.setting-item select {
  font-family: inherit;
  cursor: pointer;
}

.setting-item textarea {
  resize: vertical;
  min-height: 3.75rem;
  line-height: 1.5;
}

.setting-item input:focus,
.setting-item textarea:focus {
  outline: none;
  border-color: var(--color-primary);
  box-shadow: 0 0 0 2px rgba(102, 126, 234, 0.15);
}

.setting-item input:disabled {
  background: rgba(0, 0, 0, 0.04);
  color: var(--color-text-muted);
  cursor: not-allowed;
}

.setting-divider {
  height: 1px;
  background: var(--color-border);
  margin: 0.4rem 0;
}

/* ============ Toggle 开关 ============ */
.toggle {
  position: relative;
  display: inline-block;
  width: 2.375rem;
  height: 1.375rem;
  flex-shrink: 0;
}

.toggle input {
  opacity: 0;
  width: 0;
  height: 0;
}

.toggle-slider {
  position: absolute;
  cursor: pointer;
  top: 0; left: 0; right: 0; bottom: 0;
  background: #cbd5e1;
  border-radius: 1.375rem;
  transition: 0.2s;
}

.toggle-slider::before {
  content: "";
  position: absolute;
  height: 1rem; width: 1rem;
  left: 0.1875rem; bottom: 0.1875rem;
  background: white;
  border-radius: 50%;
  transition: 0.2s;
}

.toggle input:checked + .toggle-slider {
  background: var(--color-primary, #667eea);
}

.toggle input:checked + .toggle-slider::before {
  transform: translateX(1rem);
}

/* ============ 状态点 ============ */
.status-dot {
  display: inline-block;
  width: 0.625rem;
  height: 0.625rem;
  border-radius: 50%;
  background: #94a3b8;
}

.status-dot.running {
  background: #22c55e;
  box-shadow: 0 0 0 0 rgba(34, 197, 94, 0.4);
  animation: pulse 1.6s ease-in-out infinite;
}

.status-dot.failed {
  background: #94a3b8;
}

@keyframes pulse {
  0%, 100% { box-shadow: 0 0 0 0 rgba(34, 197, 94, 0.4); }
  50% { box-shadow: 0 0 0 4px rgba(34, 197, 94, 0); }
}
</style>
