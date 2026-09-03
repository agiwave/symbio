<!--
  ResourceManagerView — 统一资源管理页（一份页面，按 resourceType 实例化多个）

  架构分工（列表统一、详情差异化）：
  - 列表机制统一：五类资源共享 resources/* 协议与 ResourceSummary 契约，
    列表加载/刷新/实时状态订阅/删除等公共流程在本页统一承载；
  - 详情/编辑差异化：通过 FORM_COMPONENTS 注册表按类型注入专属表单组件
    （如 model 的 ModelProviderForm），未注册类型走通用兜底
    （zip 上传面板 / JSON 编辑器 / 只读详情面板）。

  能力开关驱动差异（后端 capabilities_for 为单一真相源）：
  - zip_upload        ：新建走「上传 zip」，文件名即资源目录名
  - independent_form  ：新建/编辑走表单（model 已注册专属表单；其余为通用 JSON 兜底）
  - realtime_status   ：列表项实时状态（初始态来自 list，运行时变化由 resource 事件推送，不做前端轮询）
  - mutable / read_only：是否允许删除 / 新建
  - test_connection   ：详情页「测试连接」按钮，走 resources/status 按需校验
-->
<template>
  <ResourceShell
    :title="label"
    :loading="loading"
    :has-list-content="items.length > 0"
    hide-default-new
    @new="onNew"
  >
    <template #header-actions>
      <!-- 注意：本 slot 会整体替换 ResourceShell 的默认 + 按钮，须在此显式补回 -->
      <button
        v-if="canCreate"
        class="icon-btn"
        :title="`新建 ${label}`"
        :disabled="loading"
        @click="onNew"
      >
        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2">
          <line x1="12" y1="5" x2="12" y2="19" />
          <line x1="5" y1="12" x2="19" y2="12" />
        </svg>
      </button>
      <button class="icon-btn" title="刷新" :disabled="loading" @click="loadAll">
        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M21 12a9 9 0 1 1-3-6.7L21 8" />
          <polyline points="21 3 21 8 16 8" />
        </svg>
      </button>
    </template>

    <template #meta v-if="items.length > 0">
      <span class="running-pulse" />
      共 {{ items.length }} 个{{ label }}
      <span v-if="enabledCount > 0" class="meta-sub">{{ enabledCount }} 可用</span>
    </template>

    <template #list>
      <div class="resource-list" role="listbox" :aria-label="`${label} 列表`">
        <ResourceCard
          v-for="item in items"
          :key="item.id"
          :title="item.name || item.id"
          :subtitle="item.description || item.summary"
          :status="cardStatus(item)"
          :status-title="item.status_detail || item.status"
          :is-active="selectedId === item.id"
          @click="select(item.id)"
        >
          <template #meta>
            <span class="tag tag-muted" :title="item.id">{{ item.id }}</span>
          </template>
        </ResourceCard>
      </div>
    </template>

    <template #empty>
      <p>暂无 {{ label }}</p>
      <p class="hint">{{ emptyHint }}</p>
    </template>

    <template #detail>
      <!-- ============== 新建模式 ============== -->
      <template v-if="creating">
        <!-- 注册的专属表单（model） -->
        <component
          :is="formComponent"
          v-if="formComponent"
          :item="null"
          :capabilities="capabilities"
          :saving="saving"
          :existing-ids="items.map((i) => i.id)"
          @save="onFormSave"
          @cancel="cancelCreate"
        />

        <!-- zip 上传创建（mcp / skill / agent） -->
        <div v-else-if="capabilities.zip_upload" class="create-panel">
          <div class="create-card">
            <h3 class="create-title">新建 {{ label }}</h3>
            <p class="create-desc">{{ createHint }}</p>
            <input
              ref="zipInput"
              type="file"
              accept=".zip,application/zip"
              hidden
              @change="onZipSelected"
            />
            <button class="action-btn" :disabled="uploading" @click="zipInput?.click()">
              {{ uploading ? '上传中…' : '选择 ZIP 文件上传' }}
            </button>
            <p v-if="uploadError" class="create-error">{{ uploadError }}</p>
          </div>
        </div>

        <!-- 通用 JSON 表单兜底（independent_form 且未注册专属表单） -->
        <div v-else class="create-panel">
          <div class="create-card narrow">
            <h3 class="create-title">新建 {{ label }}</h3>
            <p class="create-desc">{{ createHint }}</p>
            <div class="create-form">
              <label class="field-label">名称（目录名）</label>
              <input v-model="draftName" type="text" class="text-input" :placeholder="`${label} 名称`" />
              <label class="field-label">配置（JSON）</label>
              <textarea v-model="draftManifest" class="json-input" rows="10" spellcheck="false" />
              <p v-if="manifestError" class="create-error">{{ manifestError }}</p>
              <div class="create-actions">
                <button class="action-btn" :disabled="saving" @click="onSaveManifest">
                  {{ saving ? '保存中…' : '创建' }}
                </button>
                <button class="action-btn secondary" :disabled="saving" @click="cancelCreate">取消</button>
              </div>
            </div>
          </div>
        </div>
      </template>

      <!-- ============== 选中项：编辑/详情 ============== -->
      <template v-else-if="selected">
        <!-- 注册的专属表单（model） -->
        <component
          :is="formComponent"
          v-if="formComponent"
          :item="selected"
          :capabilities="capabilities"
          :saving="saving"
          :testing="testing"
          :deleting="deletingId === selected.id"
          @save="onFormSave"
          @test="testConnection"
          @delete="requestDelete(selected)"
          @set-default="onSetDefault"
        />

        <!-- 通用详情 + 操作工具栏（mcp / skill / agent） -->
        <template v-else>
          <div v-if="capabilities.test_connection || canDelete" class="detail-toolbar">
            <button
              v-if="capabilities.test_connection"
              class="action-btn secondary"
              :disabled="testing"
              @click="testConnection"
            >
              {{ testing ? '测试中…' : '测试连接' }}
            </button>
            <button
              v-if="canDelete"
              class="danger-btn"
              :disabled="deletingId === selected.id"
              @click="requestDelete(selected)"
            >
              {{ deletingId === selected.id ? '删除中…' : '删除' }}
            </button>
          </div>
          <ResourceDetailPanel :item="selected" />
        </template>
      </template>

      <ResourceDetailPanel v-else :item="null" />
    </template>
  </ResourceShell>
</template>

<script setup lang="ts">
import { computed, markRaw, onBeforeUnmount, onMounted, ref, watch, type Component } from 'vue'
import {
  capabilitiesFor,
  deleteResource,
  getResourceStatus,
  listResources,
  uploadResourceForm,
  uploadResourceZip,
} from '@/services/resources'
import { useResourceManager } from '@/composables/useResourceManager'
import {
  RESOURCE_LABELS,
  type ResourceCapabilities,
  type ResourceSummary,
  type ResourceType,
} from '@/schemas/resources'
import ResourceShell from '../components/common/ResourceShell.vue'
import ResourceCard from '../components/common/ResourceCard.vue'
import ResourceDetailPanel from '../components/resources/ResourceDetailPanel.vue'
import ModelProviderForm from '../components/resources/ModelProviderForm.vue'
import { subscribeResourceStatus } from '@/services/eventBus'

const props = defineProps<{ resourceType: ResourceType }>()

const {
  loading,
  saving,
  creating,
  selectedId,
  deletingId,
  showToast,
  enterCreateMode,
  select,
  markDeleting,
} = useResourceManager({ logTag: `ResourceManagerView:${props.resourceType}` })

// === 详情/编辑表单注册表：列表机制统一，详情差异化 ===
// 未注册类型走通用兜底（zip 面板 / JSON 编辑器 / 只读详情）
const FORM_COMPONENTS: Partial<Record<ResourceType, Component>> = {
  model: markRaw(ModelProviderForm),
}
const formComponent = computed(() => FORM_COMPONENTS[props.resourceType])

// === 状态 ===
const items = ref<ResourceSummary[]>([])
const capabilities = ref<ResourceCapabilities>(capabilitiesFor(props.resourceType))
const uploading = ref(false)
const uploadError = ref<string | null>(null)
const manifestError = ref<string | null>(null)
const draftName = ref('')
const draftManifest = ref('{}')
const testing = ref(false)

// === 计算属性 ===
const label = computed(() => RESOURCE_LABELS[props.resourceType])
const canCreate = computed(() => capabilities.value.mutable && !capabilities.value.read_only)
const canDelete = computed(() => capabilities.value.mutable && !capabilities.value.read_only)
const enabledCount = computed(
  () => items.value.filter((i) => i.status === 'active' || i.status === 'working').length
)
const selected = computed<ResourceSummary | null>(
  () => (selectedId.value ? items.value.find((i) => i.id === selectedId.value) ?? null : null)
)

const emptyHint = computed(() => {
  const c = capabilities.value
  if (c.zip_upload) return '点击右上角「新建」上传 ZIP（文件名即资源目录名）'
  if (c.independent_form) return '点击右上角「新建」填写表单创建'
  return ''
})
const createHint = computed(() => {
  const c = capabilities.value
  if (c.zip_upload) return 'ZIP 中的内容将解压为 <目录名>/，文件名即资源目录名'
  return '填写名称与 JSON 配置，保存即创建'
})

// === 列表 ===
async function loadAll() {
  loading.value = true
  try {
    const resp = await listResources(props.resourceType)
    capabilities.value = resp.capabilities || capabilitiesFor(props.resourceType)
    items.value = resp.items || []
    if (!selectedId.value && !creating.value && items.value.length > 0) {
      select(items.value[0].id)
    }
  } catch (err) {
    showToast('error', `加载失败: ${err}`)
  } finally {
    loading.value = false
  }
}

// === 新建 ===
function onNew() {
  uploadError.value = null
  manifestError.value = null
  if (!capabilities.value.zip_upload && !formComponent.value) {
    draftName.value = ''
    draftManifest.value = blankManifest()
  }
  enterCreateMode()
}

// 通用 JSON 兜底路径：选中时预填编辑 JSON
watch(
  () => selected.value,
  (it) => {
    if (!it) return
    if (capabilities.value.independent_form && !capabilities.value.zip_upload && !formComponent.value) {
      draftName.value = it.name || it.id || ''
      const body: Record<string, unknown> = {}
      for (const [k, v] of Object.entries(it)) {
        if (['kind', 'name', 'id', 'description', 'summary', 'updated_at', 'status', 'status_detail'].includes(k)) {
          continue
        }
        if (v !== null && v !== undefined) body[k] = v
      }
      draftManifest.value = JSON.stringify(body, null, 2)
      manifestError.value = null
    }
  }
)

function blankManifest(): string {
  return JSON.stringify({ enabled: true }, null, 2)
}

function cancelCreate() {
  creating.value = false
  if (items.value.length > 0) select(items.value[0].id)
}

// === zip 上传 ===
const zipInput = ref<HTMLInputElement | null>(null)

async function onZipSelected(e: Event) {
  const input = e.target as HTMLInputElement
  const file = input.files?.[0]
  input.value = '' // 允许重复选同一文件
  if (!file) return
  const name = file.name.replace(/\.zip$/i, '')
  if (!name) {
    uploadError.value = '请提供以 .zip 结尾的文件'
    return
  }
  uploading.value = true
  uploadError.value = null
  try {
    const buf = await file.arrayBuffer()
    const resp = await uploadResourceZip(props.resourceType, name, buf)
    showToast('success', `已上传 ${label.value}「${resp.id || name}」`)
    creating.value = false
    const list = await listResources(props.resourceType)
    items.value = list.items || []
    select(name)
  } catch (err) {
    uploadError.value = `上传失败: ${err}`
    showToast('error', `上传失败: ${err}`)
  } finally {
    uploading.value = false
    zipInput.value = null
  }
}

// === 专属表单保存（统一走 resources/upload manifest 通道） ===
async function onFormSave(payload: {
  id: string
  manifest: Record<string, unknown>
  skipValidation?: boolean
}) {
  saving.value = true
  try {
    const manifest = { ...payload.manifest }
    if (payload.skipValidation) manifest.skip_validation = true
    const resp = await uploadResourceForm(props.resourceType, payload.id, manifest)
    showToast('success', `已保存 ${label.value}「${payload.id}」`)
    creating.value = false
    const list = await listResources(props.resourceType)
    items.value = list.items || []
    select(resp.id || payload.id)
  } catch (err) {
    showToast('error', `保存失败: ${err}`)
  } finally {
    saving.value = false
  }
}

// === 设为默认（manifest 携带 is_default 标记，机制复用 upload 通道） ===
async function onSetDefault() {
  const item = selected.value
  if (!item) return
  const config = (item.config ?? {}) as Record<string, unknown>
  saving.value = true
  try {
    await uploadResourceForm(props.resourceType, item.id, {
      ...config,
      is_default: true,
      skip_validation: true,
    })
    showToast('success', `已将「${item.name || item.id}」设为默认`)
    const list = await listResources(props.resourceType)
    items.value = list.items || []
  } catch (err) {
    showToast('error', `设置默认失败: ${err}`)
  } finally {
    saving.value = false
  }
}

// === 通用 JSON 表单 创建/编辑 ===
async function onSaveManifest() {
  const name = draftName.value.trim()
  if (!name) {
    manifestError.value = '请填写名称'
    showToast('error', '请填写名称')
    return
  }
  let parsed: Record<string, unknown>
  try {
    parsed = JSON.parse(draftManifest.value || '{}')
  } catch (err) {
    manifestError.value = `JSON 解析失败: ${err}`
    showToast('error', 'JSON 格式错误')
    return
  }
  manifestError.value = null
  saving.value = true
  try {
    const resp = await uploadResourceForm(props.resourceType, name, { ...parsed, id: name })
    showToast('success', `已保存 ${label.value}「${name}」`)
    creating.value = false
    const list = await listResources(props.resourceType)
    items.value = list.items || []
    select(resp.id || name)
  } catch (err) {
    manifestError.value = `保存失败: ${err}`
    showToast('error', `保存失败: ${err}`)
  } finally {
    saving.value = false
  }
}

// === 删除 ===
async function requestDelete(item: ResourceSummary) {
  if (!confirm(`确认删除 ${label.value}「${item.name || item.id}」？`)) return
  markDeleting(item.id)
  try {
    await deleteResource(props.resourceType, item.id)
    showToast('success', `已删除 ${label.value} ${item.id}`)
    items.value = items.value.filter((i) => i.id !== item.id)
    if (selectedId.value === item.id) {
      select(items.value[0]?.id ?? '')
    }
  } catch (err) {
    showToast('error', `删除失败: ${err}`)
  } finally {
    markDeleting(null)
  }
}

// === 连接测试（资源 status 按需校验，结果也会 push 到 resource 总线）===
async function testConnection() {
  const item = selected.value
  if (!item) return
  testing.value = true
  try {
    const resp = await getResourceStatus(props.resourceType, item.id)
    if (!resp) {
      showToast('error', '该后端暂不支持连接测试')
      return
    }
    item.status = resp.status
    item.status_detail = resp.status_detail ?? undefined
    showToast(resp.status === 'connected' ? 'success' : 'error', resp.status_detail || resp.status)
  } catch (err) {
    showToast('error', `测试失败: ${err}`)
  } finally {
    testing.value = false
  }
}

// === 实时状态订阅（走事件总线 resource 事件，而非轮询）===
// 初始态由 loadAll 的 resources/list 携带；后续状态运行时变化由后端 push
// `resource` 事件，这里按 id 即时刷新列表项的状态角标。
let unsubscribeResource: (() => void) | null = null

function cardStatus(item: ResourceSummary): 'active' | 'disabled' | 'warning' | 'error' | 'muted' {
  switch (item.status) {
    case 'working': return 'warning'
    case 'disabled': return 'disabled'
    case 'error':
    case 'failed': return 'error'
    case 'active':
    case 'connected': return 'active'
    default: return 'muted'
  }
}

onMounted(() => {
  loadAll()
  unsubscribeResource = subscribeResourceStatus(props.resourceType, ({ id, status, status_detail }) => {
    const it = items.value.find((x) => x.id === id)
    if (it) {
      it.status = status
      it.status_detail = status_detail ?? undefined
    }
  })
})
onBeforeUnmount(() => unsubscribeResource?.())
</script>

<style scoped>
.resource-list {
  flex: 1;
  overflow-y: auto;
  padding: 0.25rem 0;
}
.meta-sub {
  opacity: 0.75;
}

/* ============== 新建面板（居中卡片） ============== */
.create-panel {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 2rem;
  overflow-y: auto;
}
.create-card {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.75rem;
  text-align: center;
  max-width: 24rem;
}
.create-card.narrow {
  width: 100%;
  max-width: 30rem;
}
.create-title {
  margin: 0;
  font-size: var(--font-size-lg);
  font-weight: var(--font-weight-semibold);
  color: var(--text-primary);
}
.create-desc {
  margin: 0;
  font-size: var(--font-size-sm);
  color: var(--text-muted);
  line-height: var(--line-height-normal);
}
.create-form {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  width: 100%;
  text-align: left;
}
.create-error {
  color: var(--danger-fg);
  font-size: 0.8rem;
  margin: 0;
}
.create-actions {
  display: flex;
  gap: 0.5rem;
  padding-top: 0.5rem;
}

.field-label {
  font-size: var(--font-size-xs);
  font-weight: var(--font-weight-semibold);
  color: var(--text-muted);
  text-transform: uppercase;
}
.text-input {
  padding: 0.5rem 0.75rem;
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  font-size: var(--font-size-base);
  background: var(--surface-sunken);
  color: var(--text-primary);
}
.text-input:focus,
.json-input:focus {
  outline: none;
  border-color: var(--accent);
  box-shadow: 0 0 0 2px var(--accent-subtle-bg);
}
.json-input {
  padding: 0.5rem 0.75rem;
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  font-family: var(--font-mono);
  font-size: 0.8rem;
  resize: vertical;
  line-height: 1.4;
  background: var(--surface-sunken);
  color: var(--text-primary);
}

/* ============== 通用详情工具栏 ============== */
.detail-toolbar {
  display: flex;
  justify-content: flex-end;
  align-items: center;
  gap: 0.5rem;
  padding: 0.5rem 1rem;
  border-bottom: 1px solid var(--border-default);
  background: var(--surface-panel);
  flex-shrink: 0;
}

/* ============== 按钮 ============== */
.action-btn {
  display: inline-flex;
  align-items: center;
  gap: 0.4rem;
  padding: 0.5rem 1rem;
  border: none;
  border-radius: var(--radius-md);
  background: var(--accent);
  color: var(--text-on-accent);
  font-size: var(--font-size-base);
  cursor: pointer;
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
.action-btn.secondary:hover:not(:disabled) { background: var(--surface-hover); }

.danger-btn {
  padding: 0.5rem 1rem;
  border: none;
  border-radius: var(--radius-md);
  background: var(--danger-solid);
  color: var(--text-inverse);
  font-size: var(--font-size-base);
  cursor: pointer;
  white-space: nowrap;
  transition: opacity var(--motion-fast) var(--motion-ease);
}
.danger-btn:disabled { opacity: 0.5; cursor: not-allowed; }

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
}
.icon-btn:hover:not(:disabled) {
  background: var(--surface-hover);
  color: var(--text-primary);
}
</style>
