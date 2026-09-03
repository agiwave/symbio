<!--
  ResourceManagerView — 统一资源管理页（一份页面，按 :types 路由参数管理一或多种资源）

  路由：/resources/:types?（types = 'all' | 'model,mcp' | 'model'，缺省 all）
  类型注册表：src/registry/resourceTypes.ts（单一真相源：label/前缀/表单/兜底能力）

  架构分工（列表统一、详情差异化）：
  - 列表机制统一：资源共享 resources/* 协议与 ResourceSummary 契约，
    列表加载/刷新/实时状态订阅/删除等公共流程在本页统一承载；
    多类型时按类型分组展示，选中项以 `${kind}:${id}` 复合键标识；
  - 详情/编辑差异化：注册表 form 字段按类型注入专属表单组件
    （如 model 的 ModelProviderForm），未注册类型走通用兜底
    （zip 上传面板 / JSON 编辑器 / 只读详情面板）——类似文件系统
    "扩展名 → 编辑器"机制；
  - 新建：多类型时先选类型（类型选择面板），单类型直接进入现有流程。

  能力开关驱动差异（后端 capabilities_for 为单一真相源）：
  - zip_upload        ：新建走「上传 zip」，文件名即资源目录名
  - independent_form  ：新建/编辑走表单（model 已注册专属表单；其余为通用 JSON 兜底）
  - realtime_status   ：列表项实时状态（初始态来自 list，运行时变化由 resource 事件推送，不做前端轮询）
  - mutable / read_only：是否允许删除 / 新建
  - test_connection   ：详情页「测试连接」按钮，走 resources/status 按需校验
-->
<template>
  <ResourceShell
    :title="title"
    :loading="loading"
    :has-list-content="totalCount > 0"
    hide-default-new
    @new="onNew"
  >
    <template #header-actions>
      <!-- 注意：本 slot 会整体替换 ResourceShell 的默认 + 按钮，须在此显式补回 -->
      <button
        v-if="canCreate"
        class="icon-btn"
        :title="isMulti ? '新建资源' : `新建 ${title}`"
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

    <template #meta v-if="totalCount > 0">
      <span class="running-pulse" />
      共 {{ totalCount }} {{ isMulti ? '个资源' : `个${title}` }}
      <span v-if="enabledCount > 0" class="meta-sub">{{ enabledCount }} 可用</span>
    </template>

    <template #list>
      <div class="resource-list" role="listbox" aria-label="资源列表">
        <template v-for="d in activeTypes" :key="d.kind">
          <div v-if="isMulti && (typeStates[d.kind]?.items.length ?? 0) > 0" class="list-group-header">
            <span class="tag tag-muted">{{ d.label }}</span>
            <span class="group-count">{{ typeStates[d.kind].items.length }}</span>
          </div>
          <ResourceCard
            v-for="item in typeStates[d.kind]?.items ?? []"
            :key="`${d.kind}:${item.id}`"
            :title="item.name || item.id"
            :subtitle="item.description || item.summary"
            :status="cardStatus(item)"
            :status-title="item.status_detail || item.status"
            :is-active="selectedId === `${d.kind}:${item.id}`"
            @click="select(`${d.kind}:${item.id}`)"
          >
            <template #meta>
              <span
                class="tag tag-muted tag-copy"
                :title="`${itemPath(item)}（点击复制）`"
                @click.stop="copyItemPath(item)"
              >{{ itemPath(item) }}</span>
            </template>
          </ResourceCard>
        </template>
      </div>
    </template>

    <template #empty>
      <p>{{ isMulti ? '暂无资源' : `暂无 ${title}` }}</p>
      <p class="hint">{{ emptyHint }}</p>
    </template>

    <template #detail>
      <!-- ============== 新建模式 ============== -->
      <template v-if="creating">
        <!-- 多类型：先选类型（单类型 createKind 已在 onNew 中直接确定） -->
        <div v-if="!createKind" class="create-panel">
          <div class="create-card">
            <h3 class="create-title">新建资源</h3>
            <p class="create-desc">请选择要创建的资源类型</p>
            <div class="type-choice-list">
              <button
                v-for="d in creatableTypes"
                :key="d.kind"
                class="type-choice-btn"
                type="button"
                @click="chooseCreateKind(d.kind)"
              >
                <span class="type-choice-label">{{ d.label }}</span>
                <span class="type-choice-hint">{{ capsOf(d.kind).zip_upload ? 'ZIP 上传' : '表单' }}</span>
              </button>
            </div>
            <div class="create-actions">
              <button class="action-btn secondary" type="button" @click="cancelCreate">取消</button>
            </div>
          </div>
        </div>

        <template v-else>
          <!-- 注册的专属表单（model） -->
          <component
            :is="createFormComponent"
            v-if="createFormComponent"
            :item="null"
            :capabilities="capsOf(createKind)"
            :saving="saving"
            :existing-ids="typeStates[createKind]?.items.map((i) => i.id) ?? []"
            @save="onFormSave"
            @cancel="cancelCreate"
          />

          <!-- zip 上传创建（mcp / skill / agent） -->
          <div v-else-if="capsOf(createKind).zip_upload" class="create-panel">
            <div class="create-card">
              <h3 class="create-title">新建 {{ kindLabel(createKind) }}</h3>
              <p class="create-desc">ZIP 中的内容将解压为 &lt;目录名&gt;/，文件名即资源目录名</p>
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
              <h3 class="create-title">新建 {{ kindLabel(createKind) }}</h3>
              <p class="create-desc">填写名称与 JSON 配置，保存即创建</p>
              <div class="create-form">
                <label class="field-label">名称（目录名）</label>
                <input v-model="draftName" type="text" class="text-input" :placeholder="`${kindLabel(createKind)} 名称`" />
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
      </template>

      <!-- ============== 选中项：编辑/详情 ============== -->
      <template v-else-if="selected">
        <!-- 注册的专属表单（model） -->
        <component
          :is="selectedFormComponent"
          v-if="selectedFormComponent"
          :item="selected.item"
          :capabilities="capsOf(selected.kind)"
          :saving="saving"
          :testing="testing"
          :deleting="deletingId === selected.item.id"
          @save="onFormSave"
          @test="testConnection"
          @delete="requestDelete(selected.kind, selected.item)"
          @set-default="onSetDefault"
        />

        <!-- 通用详情 + 操作工具栏（mcp / skill / agent） -->
        <template v-else>
          <div v-if="capsOf(selected.kind).test_connection || canDelete" class="detail-toolbar">
            <button
              v-if="capsOf(selected.kind).test_connection"
              class="action-btn secondary"
              :disabled="testing"
              @click="testConnection"
            >
              {{ testing ? '测试中…' : '测试连接' }}
            </button>
            <button
              v-if="canDelete"
              class="danger-btn"
              :disabled="deletingId === selected.item.id"
              @click="requestDelete(selected.kind, selected.item)"
            >
              {{ deletingId === selected.item.id ? '删除中…' : '删除' }}
            </button>
          </div>
          <ResourceDetailPanel :item="selected.item" />
        </template>
      </template>

      <ResourceDetailPanel v-else :item="null" />
    </template>
  </ResourceShell>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { deleteResource, getResourceStatus, listResources, uploadResourceForm, uploadResourceZip } from '@/services/resources'
import { useResourceManager } from '@/composables/useResourceManager'
import {
  RESOURCE_TYPE_REGISTRY,
  parseTypesParam,
  resourcePath,
  type ResourceTypeDescriptor,
} from '@/registry/resourceTypes'
import type { ResourceCapabilities, ResourceSummary, ResourceType } from '@/schemas/resources'
import { useToast } from '@/composables/useToast'
import ResourceShell from '../components/common/ResourceShell.vue'
import ResourceCard from '../components/common/ResourceCard.vue'
import ResourceDetailPanel from '../components/resources/ResourceDetailPanel.vue'
import { subscribeResourceStatus } from '@/services/eventBus'

const props = defineProps<{ typesParam?: string }>()

const toast = useToast()

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
} = useResourceManager({ logTag: 'ResourceManagerView' })

// === 活动类型（注册表派生；路由 :types 变化时经 :key="route.path" 重建组件） ===
const activeTypes = computed<ResourceTypeDescriptor[]>(() => parseTypesParam(props.typesParam))
const isMulti = computed(() => activeTypes.value.length > 1)

// === 每类独立数据：capabilities 后端下发为真相源，descriptor 兜底 ===
interface TypeState {
  items: ResourceSummary[]
  capabilities: ResourceCapabilities
}
const typeStates = ref<Record<string, TypeState>>({})

function capsOf(kind: string): ResourceCapabilities {
  return (
    typeStates.value[kind]?.capabilities ??
    (RESOURCE_TYPE_REGISTRY as Record<string, ResourceTypeDescriptor | undefined>)[kind]?.capabilities ?? {
      zip_upload: false,
      independent_form: false,
      realtime_status: false,
      mutable: false,
      test_connection: false,
      read_only: true,
    }
  )
}

function kindLabel(kind: string): string {
  return (
    (RESOURCE_TYPE_REGISTRY as Record<string, ResourceTypeDescriptor | undefined>)[kind]?.label ?? kind
  )
}

// === 选中项：复合键 `${kind}:${id}`（跨类型唯一） ===
const selected = computed<{ kind: string; item: ResourceSummary } | null>(() => {
  const key = selectedId.value
  if (!key) return null
  const idx = key.indexOf(':')
  if (idx < 0) return null
  const kind = key.slice(0, idx)
  const id = key.slice(idx + 1)
  const item = typeStates.value[kind]?.items.find((i) => i.id === id)
  return item ? { kind, item } : null
})

// === 新建：多类型先选类型 ===
const createKind = ref<ResourceType | null>(null)
const createFormComponent = computed(() => {
  const d = createKind.value ? RESOURCE_TYPE_REGISTRY[createKind.value] : null
  return d?.form ?? null
})
const selectedFormComponent = computed(() => {
  const kind = selected.value?.kind as ResourceType | undefined
  return kind ? RESOURCE_TYPE_REGISTRY[kind]?.form ?? null : null
})

/** 新建中 → createKind；编辑选中项 → 选中项 kind */
const activeFormKind = computed<string | null>(() => {
  if (creating.value) return createKind.value
  return selected.value?.kind ?? null
})

// === 状态 ===
const uploading = ref(false)
const uploadError = ref<string | null>(null)
const manifestError = ref<string | null>(null)
const draftName = ref('')
const draftManifest = ref('{}')
const testing = ref(false)

// === 计算属性 ===
const title = computed(() => (isMulti.value ? '资源' : activeTypes.value[0]?.label ?? '资源'))
const totalCount = computed(() =>
  Object.values(typeStates.value).reduce((n, s) => n + s.items.length, 0)
)
const enabledCount = computed(() =>
  Object.values(typeStates.value).reduce(
    (n, s) => n + s.items.filter((i) => i.status === 'active' || i.status === 'working').length,
    0
  )
)
const canCreate = computed(() =>
  activeTypes.value.some((d) => capsOf(d.kind).mutable && !capsOf(d.kind).read_only)
)
const canDelete = computed(() => {
  const sel = selected.value
  if (!sel) return false
  const c = capsOf(sel.kind)
  return c.mutable && !c.read_only
})
/** 类型选择面板中可创建的类型（能力开关过滤，session 等只读/不可写类型排除） */
const creatableTypes = computed(() =>
  activeTypes.value.filter((d) => capsOf(d.kind).mutable && !capsOf(d.kind).read_only)
)

const emptyHint = computed(() => {
  if (isMulti.value) return canCreate.value ? '点击右上角「新建」创建资源' : ''
  const c = activeTypes.value[0] ? capsOf(activeTypes.value[0].kind) : null
  if (!c) return ''
  if (c.zip_upload) return '点击右上角「新建」上传 ZIP（文件名即资源目录名）'
  if (c.independent_form) return '点击右上角「新建」填写表单创建'
  return ''
})

/** 资源路径唯一标识：[provider]/[id].[kind]（provider 缺省回退 kind） */
function itemPath(item: ResourceSummary): string {
  return resourcePath(item.provider || item.kind, item.id, item.kind)
}

async function copyItemPath(item: ResourceSummary) {
  const path = itemPath(item)
  try {
    await navigator.clipboard.writeText(path)
    toast.showToast('success', '已复制资源路径')
  } catch {
    toast.showToast('info', `路径：${path}`)
  }
}

// === 列表 ===
async function loadAll() {
  loading.value = true
  try {
    const entries = await Promise.all(
      activeTypes.value.map(async (d) => [d.kind, await listResources(d.kind)] as const)
    )
    const states: Record<string, TypeState> = {}
    for (const [kind, resp] of entries) {
      states[kind] = {
        items: resp.items || [],
        capabilities: resp.capabilities || capsOf(kind),
      }
    }
    typeStates.value = states
    if (!selectedId.value && !creating.value) {
      // 首个非空分组的首项自动选中（保持单类型页既有行为）
      for (const d of activeTypes.value) {
        const first = states[d.kind]?.items[0]
        if (first) {
          select(`${d.kind}:${first.id}`)
          break
        }
      }
    }
  } catch (err) {
    showToast('error', `加载失败: ${err}`)
  } finally {
    loading.value = false
  }
}

/** 只刷新受影响类型（保存/删除/上传后调用，不全量重载） */
async function refreshKind(kind: string) {
  const list = await listResources(kind as ResourceType)
  typeStates.value = {
    ...typeStates.value,
    [kind]: {
      items: list.items || [],
      capabilities: list.capabilities || capsOf(kind),
    },
  }
}

// === 新建 ===
function onNew() {
  uploadError.value = null
  manifestError.value = null
  if (isMulti.value) {
    // 多类型：先弹类型选择面板
    createKind.value = null
    enterCreateMode()
    return
  }
  const d = activeTypes.value[0]
  if (!d) return
  beginCreateFor(d.kind)
}

function chooseCreateKind(kind: ResourceType) {
  uploadError.value = null
  manifestError.value = null
  beginCreateFor(kind)
}

function beginCreateFor(kind: ResourceType) {
  createKind.value = kind
  const caps = capsOf(kind)
  const hasForm = Boolean(RESOURCE_TYPE_REGISTRY[kind]?.form)
  if (!caps.zip_upload && !hasForm) {
    draftName.value = ''
    draftManifest.value = blankManifest()
  }
  if (!creating.value) enterCreateMode()
}

function blankManifest(): string {
  return JSON.stringify({ enabled: true }, null, 2)
}

// 通用 JSON 兜底路径：选中时预填编辑 JSON
watch(
  () => selected.value,
  (sel) => {
    if (!sel) return
    const caps = capsOf(sel.kind)
    const form = (RESOURCE_TYPE_REGISTRY as Record<string, ResourceTypeDescriptor | undefined>)[sel.kind]?.form
    if (caps.independent_form && !caps.zip_upload && !form) {
      draftName.value = sel.item.name || sel.item.id || ''
      const body: Record<string, unknown> = {}
      for (const [k, v] of Object.entries(sel.item)) {
        if (['kind', 'provider', 'name', 'id', 'description', 'summary', 'updated_at', 'status', 'status_detail'].includes(k)) {
          continue
        }
        if (v !== null && v !== undefined) body[k] = v
      }
      draftManifest.value = JSON.stringify(body, null, 2)
      manifestError.value = null
    }
  }
)

function cancelCreate() {
  creating.value = false
  createKind.value = null
  const firstKey = firstItemKey()
  if (firstKey) select(firstKey)
}

function firstItemKey(): string | null {
  for (const d of activeTypes.value) {
    const first = typeStates.value[d.kind]?.items[0]
    if (first) return `${d.kind}:${first.id}`
  }
  return null
}

// === zip 上传 ===
const zipInput = ref<HTMLInputElement | null>(null)

async function onZipSelected(e: Event) {
  const kind = createKind.value
  if (!kind) return
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
    const resp = await uploadResourceZip(kind, name, buf)
    showToast('success', `已上传 ${kindLabel(kind)}「${resp.id || name}」`)
    creating.value = false
    createKind.value = null
    await refreshKind(kind)
    select(`${kind}:${resp.id || name}`)
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
  const kind = activeFormKind.value
  if (!kind) return
  saving.value = true
  try {
    const manifest = { ...payload.manifest }
    if (payload.skipValidation) manifest.skip_validation = true
    const resp = await uploadResourceForm(kind as ResourceType, payload.id, manifest)
    showToast('success', `已保存 ${kindLabel(kind)}「${payload.id}」`)
    creating.value = false
    createKind.value = null
    await refreshKind(kind)
    select(`${kind}:${resp.id || payload.id}`)
  } catch (err) {
    showToast('error', `保存失败: ${err}`)
  } finally {
    saving.value = false
  }
}

// === 设为默认（manifest 携带 is_default 标记，机制复用 upload 通道） ===
async function onSetDefault() {
  const sel = selected.value
  if (!sel) return
  const config = (sel.item.config ?? {}) as Record<string, unknown>
  saving.value = true
  try {
    await uploadResourceForm(sel.kind as ResourceType, sel.item.id, {
      ...config,
      is_default: true,
      skip_validation: true,
    })
    showToast('success', `已将「${sel.item.name || sel.item.id}」设为默认`)
    await refreshKind(sel.kind)
  } catch (err) {
    showToast('error', `设置默认失败: ${err}`)
  } finally {
    saving.value = false
  }
}

// === 通用 JSON 表单 创建/编辑 ===
async function onSaveManifest() {
  const kind = activeFormKind.value
  if (!kind) return
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
    const resp = await uploadResourceForm(kind as ResourceType, name, { ...parsed, id: name })
    showToast('success', `已保存 ${kindLabel(kind)}「${name}」`)
    creating.value = false
    createKind.value = null
    await refreshKind(kind)
    select(`${kind}:${resp.id || name}`)
  } catch (err) {
    manifestError.value = `保存失败: ${err}`
    showToast('error', `保存失败: ${err}`)
  } finally {
    saving.value = false
  }
}

// === 删除 ===
async function requestDelete(kind: string, item: ResourceSummary) {
  if (!confirm(`确认删除 ${kindLabel(kind)}「${item.name || item.id}」？`)) return
  markDeleting(item.id)
  try {
    await deleteResource(kind as ResourceType, item.id)
    showToast('success', `已删除 ${kindLabel(kind)} ${item.id}`)
    if (typeStates.value[kind]) {
      typeStates.value[kind].items = typeStates.value[kind].items.filter((i) => i.id !== item.id)
    }
    if (selectedId.value === `${kind}:${item.id}`) {
      select(firstItemKey() ?? '')
    }
  } catch (err) {
    showToast('error', `删除失败: ${err}`)
  } finally {
    markDeleting(null)
  }
}

// === 连接测试（资源 status 按需校验，结果也会 push 到 resource 总线）===
async function testConnection() {
  const sel = selected.value
  if (!sel) return
  testing.value = true
  try {
    const resp = await getResourceStatus(sel.kind as ResourceType, sel.item.id)
    if (!resp) {
      showToast('error', '该后端暂不支持连接测试')
      return
    }
    sel.item.status = resp.status
    sel.item.status_detail = resp.status_detail ?? undefined
    showToast(resp.status === 'connected' ? 'success' : 'error', resp.status_detail || resp.status)
  } catch (err) {
    showToast('error', `测试失败: ${err}`)
  } finally {
    testing.value = false
  }
}

// === 实时状态订阅（走事件总线 resource 事件，而非轮询）===
// 初始态由 loadAll 的 resources/list 携带；后续状态运行时变化由后端 push
// `resource` 事件，这里按 kind + id 即时刷新列表项的状态角标。
let unsubscribers: Array<() => void> = []

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
  unsubscribers = activeTypes.value.map((d) =>
    subscribeResourceStatus(d.kind, ({ id, status, status_detail }) => {
      const it = typeStates.value[d.kind]?.items.find((x) => x.id === id)
      if (it) {
        it.status = status
        it.status_detail = status_detail ?? undefined
      }
    })
  )
})
onBeforeUnmount(() => {
  unsubscribers.forEach((fn) => fn())
  unsubscribers = []
})
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

/* ============== 列表分组头（多类型） ============== */
.list-group-header {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.5rem 0.75rem 0.25rem;
}
.list-group-header:first-child {
  padding-top: 0.25rem;
}
.group-count {
  font-size: var(--font-size-xs);
  color: var(--text-muted);
}

.tag-copy {
  cursor: pointer;
  transition: color var(--motion-fast) var(--motion-ease), background var(--motion-fast) var(--motion-ease);
}
.tag-copy:hover {
  color: var(--text-primary);
  background: var(--surface-hover);
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
  justify-content: center;
}

/* ============== 类型选择列表（多类型新建） ============== */
.type-choice-list {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  width: 100%;
  max-width: 18rem;
}
.type-choice-btn {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.75rem;
  padding: 0.7rem 1rem;
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: var(--surface-panel);
  cursor: pointer;
  transition: border-color var(--motion-fast) var(--motion-ease), background var(--motion-fast) var(--motion-ease);
}
.type-choice-btn:hover {
  border-color: var(--accent);
  background: var(--surface-hover);
}
.type-choice-label {
  font-size: var(--font-size-base);
  font-weight: var(--font-weight-semibold);
  color: var(--text-primary);
}
.type-choice-hint {
  font-size: var(--font-size-xs);
  color: var(--text-muted);
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
