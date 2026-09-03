<!--
  ResourceManagerView — 统一资源管理页（一份页面，:types 路由参数决定一或多种资源）

  路由：/resources/:types?（types = 'all' | 'model,mcp' | 'model'，缺省 all）

  核心逻辑在 useResourcePage（可单测）；本组件只做模板、事件绑定与 UI 副作用。
  - 类型集合来自后端 resources/providers 注册表（ProviderInfo），非前端硬编码；
  - 列表为**混合平排**（不分类型分组，类似目录按后缀混排文件），按 name 排序；
  - 详情/编辑按类型注入专属 editor，未注册走通用兜底（zip 面板 / JSON 编辑器 / 只读详情）；
  - 新建：多类型先选类型（仅列可创建 provider）；session 等 supports_upload=false 的类型
    在资源管理器内不可创建/删除（修复"新建必失败"）。
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
      <!-- 本 slot 整体替换 ResourceShell 默认 + 按钮，须显式补回 -->
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

    <!-- 混合平排列表：所有类型所有项，按 name 排序，无分组。
         compact 模式（后端 ProviderInfo.compact_list，如设置分区）：
         仅显示类型图标 + 标题，隐藏描述与路径标签 -->
    <template #list>
      <div class="resource-list" role="listbox" aria-label="资源列表">
        <ResourceCard
          v-for="item in items"
          :key="`${item.kind}:${item.id}`"
          :title="item.name || item.id"
          :subtitle="isCompact ? undefined : (item.description || item.summary)"
          :status="cardStatus(item)"
          :status-title="item.status_detail || item.status"
          :icon="getResourceIconFor(item)"
          :is-active="selectedId === `${item.kind}:${item.id}`"
          @click="select(`${item.kind}:${item.id}`)"
        >
          <template v-if="!isCompact" #meta>
            <span
              class="tag tag-muted tag-copy"
              :title="`${itemPath(item)}（点击复制）`"
              @click.stop="copyItemPath(item)"
            >{{ itemPath(item) }}</span>
          </template>
        </ResourceCard>
      </div>
    </template>

    <template #empty>
      <p>{{ isMulti ? '暂无资源' : `暂无 ${title}` }}</p>
      <p class="hint">{{ emptyHint }}</p>
    </template>

    <template #detail>
      <!-- ============== 新建模式 ============== -->
      <template v-if="creating">
        <!-- 多类型：先选类型（单类型 createKind 已在 onNew 确定） -->
        <div v-if="!createKind" class="create-panel">
          <div class="create-card">
            <h3 class="create-title">新建资源</h3>
            <p class="create-desc">请选择要创建的资源类型</p>
            <div class="type-choice-list">
              <button
                v-for="d in creatableInActive"
                :key="d.kind"
                class="type-choice-btn"
                type="button"
                @click="beginCreate(d.kind)"
              >
                <span class="type-choice-label">{{ kindLabel(d.kind) }}</span>
                <span class="type-choice-hint">{{ capsOf(d.kind).zip_upload ? 'ZIP 上传' : '表单' }}</span>
              </button>
            </div>
            <div class="create-actions">
              <button class="action-btn secondary" type="button" @click="cancelCreate">取消</button>
            </div>
          </div>
        </div>

        <template v-else>
          <!-- 注册的专属 editor（model） -->
          <component
            :is="createEditor(createKind)"
            v-if="createEditor(createKind)"
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

          <!-- 通用 JSON 表单兜底（independent_form 且未注册专属 editor） -->
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
        <!-- 注册的专属 editor（model） -->
        <component
          :is="selectedEditor"
          v-if="selectedEditor"
          :item="selected.item"
          :capabilities="capsOf(selected.kind)"
          :saving="saving"
          :testing="testing"
          :deleting="deletingId === selected.item.id"
          @save="onFormSave"
          @test="testConnection"
          @delete="requestDelete(selected)"
          @set-default="onSetDefault"
        />

        <!-- 通用详情 + 操作工具栏（mcp / skill / agent） -->
        <template v-else>
          <div v-if="capsOf(selected.kind).test_connection || canDeleteSelected" class="detail-toolbar">
            <button
              v-if="capsOf(selected.kind).test_connection"
              class="action-btn secondary"
              :disabled="testing"
              @click="testConnection"
            >
              {{ testing ? '测试中…' : '测试连接' }}
            </button>
            <button
              v-if="canDeleteSelected"
              class="danger-btn"
              :disabled="deletingId === selected.item.id"
              @click="requestDelete(selected)"
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
import { deleteResource, getResourceStatus, uploadResourceForm, uploadResourceZip } from '@/services/resources'
import { useResourcePage } from '@/composables/useResourcePage'
import { resourcePath, getResourceIconFor } from '@/registry/resourceTypes'
import type { SelectedResource } from '@/composables/useResourcePage'
import type { ResourceSummary } from '@/schemas/resources'
import { useToast } from '@/composables/useToast'
import ResourceShell from '../components/common/ResourceShell.vue'
import ResourceCard from '../components/common/ResourceCard.vue'
import ResourceDetailPanel from '../components/resources/ResourceDetailPanel.vue'
import { subscribeResourceStatus } from '@/services/eventBus'

const props = defineProps<{ typesParam?: string }>()
const typesParam = computed(() => props.typesParam)

const toast = useToast()

// === 组合式核心逻辑（可单测） ===
const {
  activeTypes,
  isMulti,
  typeStates,
  items,
  selected,
  selectedId,
  creating,
  saving,
  loading,
  deletingId,
  totalCount,
  enabledCount,
  creatableInActive,
  canCreate,
  canDeleteSelected,
  kindLabel,
  capsOf,
  createEditor,
  selectedEditor,
  activeFormKind,
  createKind,
  select,
  loadAll,
  refreshKind,
  beginCreate,
  startTypeChoice,
  cancelCreate,
  showToast,
} = useResourcePage(typesParam)

// === 展示派生 ===
const title = computed(() => (isMulti.value ? '资源' : kindLabel(activeTypes.value[0]?.kind ?? '')))

/** 列表简洁模式：活动类型中任一开启 compact_list 即生效（仅图标 + 标题） */
const isCompact = computed(() => activeTypes.value.some((p) => p.compact_list))

const emptyHint = computed(() => {
  if (isMulti.value) return canCreate.value ? '点击右上角「新建」创建资源' : ''
  const kind = activeTypes.value[0]?.kind
  if (!kind) return ''
  const c = capsOf(kind)
  if (c.zip_upload) return '点击右上角「新建」上传 ZIP（文件名即资源目录名）'
  if (c.independent_form) return '点击右上角「新建」填写表单创建'
  return ''
})

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

// === 新建 ===
function onNew() {
  uploadError.value = null
  manifestError.value = null
  if (isMulti.value) {
    // 多类型：进入创建态但不绑定类型 → 渲染类型选择面板
    startTypeChoice()
    return
  }
  // 单类型：直接进入该类型现有新建流程
  const kind = activeTypes.value[0]?.kind
  if (kind) beginCreate(kind)
}

const zipInput = ref<HTMLInputElement | null>(null)
const uploading = ref(false)
const uploadError = ref<string | null>(null)
const manifestError = ref<string | null>(null)
const draftName = ref('')
const draftManifest = ref('{}')
const testing = ref(false)

// 通用 JSON 兜底：选中时预填编辑 JSON
watch(
  () => selected.value,
  (sel) => {
    if (!sel) return
    const caps = capsOf(sel.kind)
    const hasEditor = Boolean(selectedEditor.value)
    if (caps.independent_form && !caps.zip_upload && !hasEditor) {
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

async function onZipSelected(e: Event) {
  const kind = createKind.value
  if (!kind) return
  const input = e.target as HTMLInputElement
  const file = input.files?.[0]
  input.value = ''
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
    cancelCreate()
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
    const resp = await uploadResourceForm(kind, payload.id, manifest)
    showToast('success', `已保存 ${kindLabel(kind)}「${payload.id}」`)
    cancelCreate()
    await refreshKind(kind)
    select(`${kind}:${resp.id || payload.id}`)
  } catch (err) {
    showToast('error', `保存失败: ${err}`)
  } finally {
    saving.value = false
  }
}

async function onSetDefault() {
  const sel = selected.value
  if (!sel) return
  const config = (sel.item.config ?? {}) as Record<string, unknown>
  saving.value = true
  try {
    await uploadResourceForm(sel.kind, sel.item.id, {
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
    const resp = await uploadResourceForm(kind, name, { ...parsed, id: name })
    showToast('success', `已保存 ${kindLabel(kind)}「${name}」`)
    cancelCreate()
    await refreshKind(kind)
    select(`${kind}:${resp.id || name}`)
  } catch (err) {
    manifestError.value = `保存失败: ${err}`
    showToast('error', `保存失败: ${err}`)
  } finally {
    saving.value = false
  }
}

async function requestDelete(sel: SelectedResource) {
  if (!confirm(`确认删除 ${kindLabel(sel.kind)}「${sel.item.name || sel.item.id}」？`)) return
  if (deletingId.value === sel.item.id) return
  deletingId.value = sel.item.id
  try {
    await deleteResource(sel.kind, sel.item.id)
    showToast('success', `已删除 ${kindLabel(sel.kind)} ${sel.item.id}`)
    if (typeStates.value[sel.kind]) {
      typeStates.value[sel.kind].items = typeStates.value[sel.kind].items.filter((i) => i.id !== sel.item.id)
    }
    if (selectedId.value === `${sel.kind}:${sel.item.id}`) {
      select('')
    }
  } catch (err) {
    showToast('error', `删除失败: ${err}`)
  } finally {
    deletingId.value = null
  }
}

async function testConnection() {
  const sel = selected.value
  if (!sel) return
  testing.value = true
  try {
    const resp = await getResourceStatus(sel.kind, sel.item.id)
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

// === 实时状态订阅（事件总线 resource 事件，非轮询） ===
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