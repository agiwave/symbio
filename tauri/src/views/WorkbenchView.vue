<!--
  WorkbenchView — 统一实体管理页（全 App 唯一实体页面，机制化配置驱动）

  页面与 ts 均为复用的标准件：路由参数 + 后端注册表自动配置，新增实体类型
  **零页面/零 ts 开发**（仅当详情需要专属编辑器时，才向 entityTypes 注册表
  登记一个差异化组件——这是唯一的逐特性扩展位）。

  两种模式（由路由参数自动判定，逻辑在 useWorkbenchView）：

  1. entity（/entities/:types? 、 /settings）：
     侧边栏由 MainLayout 应用外壳承担；类别 = providers 注册表解析 :types；
     混合平排列表；详情/编辑按 kind / kind:config_type 注册表分发，
     未注册走通用兜底（zip 面板 / JSON 表单 / 只读详情）。
  2. container（/container/:kind/:id/entities 、 /agent/:agentId/entities）：
     整页推入（MainLayout 之外），自带侧边栏 = ProviderInfo.container_kinds
     （子类别导航 + 计数角标 + 返回键）；详情/新建 = 标准文本编辑器
     （路径模板 / 内容模板均后端下发）。

  协议同一套 `${prefix}/entities/*`，容器语义仅多 container 字段。
-->
<template>
  <div :class="isContainer ? 'workbench-page fullscreen' : 'workbench-page'">
    <Workbench
      :rail-items="railItems"
      :back="isContainer"
      :title="title"
      :list-width="isContainer ? 240 : 260"
      hide-default-new
      :has-list-content="listCount > 0"
      :loading="loading"
      @rail-select="switchKind"
      @rail-back="goBack"
      @new="onNew"
    >
      <template #header-actions>
        <button v-if="canCreate" class="icon-btn" :title="`新建 ${title}`" :disabled="loading" @click="onNew">
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

      <template #meta>
        <!-- container：容器条目名 -->
        <template v-if="isContainer">
          <span class="meta-container" :title="containerId">{{ containerName }}</span>
        </template>
      </template>

      <!-- ===== 列表：entity = 混合平排（尊重服务器返回顺序）；
             container = 当前子类别条目（id 为容器内相对路径） ===== -->
      <template #list>
        <div v-if="!isContainer" class="entity-list" role="listbox" aria-label="实体列表">
          <EntityCard
            v-for="item in items"
            :key="`${item.kind}:${item.id}`"
            :title="item.name || item.id"
            :subtitle="isCompact ? undefined : (item.description || item.summary)"
            :status="cardStatus(item)"
            :status-title="item.status_detail || item.status"
            :icon="getEntityIconFor(item)"
            :show-status="showStatusFor(item)"
            :is-active="selectedId === `${item.kind}:${item.id}`"
            @click="select(`${item.kind}:${item.id}`)"
          />
        </div>
        <div v-else class="entry-list">
          <EntityCard
            v-for="e in kindEntries"
            :key="e.id"
            :title="e.name"
            :subtitle="e.id"
            :badge="priorityLabel(e)"
            :is-active="selectedId === e.kind + ':' + e.id && !creating"
            show-status
            status="muted"
            @click="selectEntry(e.id)"
          />
        </div>
      </template>

      <template #empty>
        <template v-if="isContainer">
          <p>暂无{{ activeKindMeta?.label ?? '实体' }}</p>
          <p class="hint">点击右上角「新建」添加{{ activeKindMeta?.label ?? '' }}</p>
        </template>
        <template v-else>
          <p>{{ isMulti ? '暂无实体' : `暂无 ${title}` }}</p>
          <p class="hint">{{ emptyHint }}</p>
        </template>
      </template>

      <template #detail>
        <!-- ============== 新建模式 ============== -->
        <!-- container：名称 + 内容（路径模板/内容模板由后端 container_kinds 下发） -->
        <div v-if="creating && isContainer" class="entry-editor">
          <div class="editor-head">
            <h3 class="editor-title">新建{{ activeKindMeta?.label ?? '' }}</h3>
          </div>
          <div class="editor-form">
            <label class="field-label">名称</label>
            <input v-model="newName" type="text" class="text-input" :placeholder="`填写${activeKindMeta?.label ?? ''}名称`" />
            <p class="field-hint">写入路径：<code>{{ createPathPreview }}</code></p>
            <label class="field-label">内容</label>
            <textarea v-model="newContent" class="content-input" rows="16" spellcheck="false" />
            <p class="field-hint">{{ activeKindMeta?.description ?? '' }}</p>
            <p v-if="editorError" class="editor-error">{{ editorError }}</p>
            <div class="editor-actions">
              <button class="action-btn" :disabled="saving" @click="saveCreate">
                {{ saving ? '保存中…' : '创建' }}
              </button>
              <button class="action-btn secondary" :disabled="saving" @click="cancelCreate">取消</button>
            </div>
          </div>
        </div>

        <!-- entity 新建 -->
        <template v-else-if="creating">
          <!-- 多类型：先选类型（单类型 createKind 已在 onNew 确定） -->
          <div v-if="!createKind" class="create-panel">
            <div class="create-card">
              <h3 class="create-title">新建实体</h3>
              <p class="create-desc">请选择要创建的实体类型</p>
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
            <!-- 注册的专属 editor（model）；:key 确保切换实体/类型时重挂载，避免表单状态残留 -->
            <component
              :is="createEditor(createKind)"
              v-if="createEditor(createKind)"
              :key="'create:' + (createKind || '')"
              :item="null"
              :capabilities="capsOf(createKind)"
              :saving="saving"
              :existing-ids="typeStates[createKind]?.items.map((i) => i.id) ?? []"
              @save="saveForm"
              @cancel="cancelCreate"
            />

            <!-- 定义驱动的新建表单（后端 entities/detail 空态定义，如 model） -->
            <DetailForm
              v-else-if="detailDefinition"
              :key="'create-def:' + (createKind || '')"
              :definition="detailDefinition"
              :item="null"
              :capabilities="capsOf(createKind)"
              :saving="saving"
              :existing-ids="typeStates[createKind]?.items.map((i) => i.id) ?? []"
              @save="saveForm"
              @cancel="cancelCreate"
            />

            <!-- zip 上传创建（mcp / skill / agent） -->
            <div v-else-if="capsOf(createKind).zip_upload" class="create-panel">
              <div class="create-card">
                <h3 class="create-title">新建 {{ kindLabel(createKind) }}</h3>
                <p class="create-desc">ZIP 中的内容将解压为 &lt;目录名&gt;/，文件名即实体目录名</p>
                <input
                  ref="zipInput"
                  type="file"
                  accept=".zip,application/zip"
                  hidden
                  @change="onZipSelected"
                />
                <button class="action-btn" :disabled="zipUploading" @click="zipInput?.click()">
                  {{ zipUploading ? '上传中…' : '选择 ZIP 文件上传' }}
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
                    <button class="action-btn" :disabled="saving" @click="saveManifest">
                      {{ saving ? '保存中…' : '创建' }}
                    </button>
                    <button class="action-btn secondary" :disabled="saving" @click="cancelCreate">取消</button>
                  </div>
                </div>
              </div>
            </div>
          </template>
        </template>

        <!-- ============== 选中项 ============== -->
        <!-- container：标准文本编辑器（entities/get content + entities/put 写回） -->
        <div v-else-if="isContainer && selectedEntry" class="entry-editor">
          <div class="editor-head">
            <div class="title-block">
              <h3 class="editor-title">{{ selectedEntry.name }}</h3>
              <span v-if="priorityLabel(selectedEntry)" class="badge">{{ priorityLabel(selectedEntry) }}</span>
            </div>
            <code class="editor-path">{{ selectedEntry.id }}</code>
          </div>
          <div class="editor-form">
            <textarea
              v-model="content"
              class="content-input"
              rows="18"
              spellcheck="false"
              :disabled="loadingContent"
              :placeholder="loadingContent ? '加载中…' : ''"
            />
            <p class="field-hint">{{ activeKindMeta?.description ?? '' }}</p>
            <p v-if="editorError" class="editor-error">{{ editorError }}</p>
            <div class="editor-actions">
              <button class="action-btn" :disabled="loadingContent || saving || !dirty" @click="saveEntry">
                {{ saving ? '保存中…' : '保存' }}
              </button>
              <button class="action-btn secondary" :disabled="saving" @click="reloadSelected">还原</button>
              <span class="spacer" />
              <button v-if="canDeleteSelected" class="danger-btn" :disabled="!!deletingId" @click="removeSelectedEntry">
                {{ deletingId ? '删除中…' : '删除' }}
              </button>
            </div>
          </div>
        </div>

        <!-- entity 选中 -->
        <template v-else-if="selected">
          <!-- 注册的专属 editor（model / agent:bundle / 设置分区）；:key 确保切换时重挂载 -->
          <component
            :is="selectedEditor"
            v-if="selectedEditor"
            :key="selected.kind + ':' + selected.item.id"
            :item="selected.item"
            :capabilities="capsOf(selected.kind)"
            :saving="saving"
            :testing="testing"
            :deleting="deletingId === selected.item.id"
            @save="saveForm"
            @test="testConnection"
            @delete="removeSelected"
            @set-default="saveDefault"
            @created="onEditorCreated"
          />

          <!-- 定义驱动的详情表单（后端 entities/detail 下发，如 model / 设置三分区） -->
          <DetailForm
            v-else-if="detailDefinition"
            :key="'def:' + selected.kind + ':' + selected.item.id"
            :definition="detailDefinition"
            :item="selected.item"
            :capabilities="capsOf(selected.kind)"
            :saving="saving"
            :testing="testing"
            :deleting="deletingId === selected.item.id"
            @save="saveForm"
            @test="testConnection"
            @delete="removeSelected"
            @set-default="saveDefault"
          />

          <!-- 通用详情 + 操作工具栏（mcp / skill / agent 等） -->
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
                @click="removeSelected"
              >
                {{ deletingId === selected.item.id ? '删除中…' : '删除' }}
              </button>
            </div>
            <EntityDetailPanel :item="selected.item" />
          </template>
        </template>

        <!-- ============== 未选中 ============== -->
        <div v-else-if="isContainer" class="no-selection">
          <p>← 选择一个{{ activeKindMeta?.label ?? '实体' }}查看/编辑</p>
        </div>
        <EntityDetailPanel v-else :item="null" />
      </template>
    </Workbench>

    <!-- container 模式整页在 MainLayout 之外，需自带全局浮动消息浮层（useToast 单例驱动） -->
    <Toast v-if="isContainer" />
  </div>
</template>

<script setup lang="ts">
import { computed, onBeforeUnmount, onMounted, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import Workbench from '@/components/common/Workbench.vue'
import EntityCard from '@/components/common/EntityCard.vue'
import Toast from '@/components/common/Toast.vue'
import EntityDetailPanel from '@/components/entities/EntityDetailPanel.vue'
import DetailForm from '@/components/entities/DetailForm.vue'
import { useWorkbenchView } from '@/composables/useWorkbenchView'
import { getEntityIconFor } from '@/registry/entityTypes'
import type { EntitySummary } from '@/schemas/entities'
import { subscribe, subscribeEntityStatus } from '@/services/eventBus'

const props = defineProps<{
  /** leaf 模式：路由 :types 参数（'all' | 逗号分隔 kind | 单 kind） */
  typesParam?: string
  /** container 模式：容器所属 provider kind（如 'agent'） */
  containerKind?: string
  /** container 模式：容器条目 id */
  containerId?: string
}>()

const router = useRouter()

const pageMode = props.containerKind ? 'container' : 'leaf'

// === 唯一页面逻辑（useWorkbenchView，可单测） ===
const {
  isContainer,
  railItems,
  title,
  isMulti,
  isCompact,
  activeKindMeta,
  switchKind,
  kindEntries,
  items,
  totalCount,
  emptyHint,
  loading,
  loadAll,
  refreshKind,
  selected,
  selectedId,
  select,
  creating,
  canCreate,
  onNew,
  createKind,
  creatableInActive,
  beginCreate,
  cancelCreate,
  newName,
  newContent,
  createPathPreview,
  saveCreate,
  saving,
  zipUploading,
  uploadError,
  manifestError,
  draftName,
  draftManifest,
  saveZip,
  saveForm,
  saveManifest,
  saveDefault,
  capsOf,
  kindLabel,
  createEditor,
  selectedEditor,
  detailDefinition,
  canDeleteSelected,
  containerName,
  containerIdRef,
  selectedEntry,
  content,
  dirty,
  loadingContent,
  editorError,
  selectEntry,
  reloadSelected,
  saveEntry,
  priorityLabel,
  deletingId,
  remove,
  testConnection,
  testing,
  typeStates,
  activeTypes,
} = useWorkbenchView({
  mode: pageMode,
  typesParam: computed(() => props.typesParam),
  containerKind: props.containerKind ?? '',
  containerId: () => props.containerId,
})

/** 列表计数（has-list-content 判定：container 看当前类别，entity 看总数） */
const listCount = computed(() => (isContainer ? kindEntries.value.length : totalCount.value))

/** 返回容器条目所在的顶层实体列表页 */
function goBack() {
  router.push(`/entities/${props.containerKind}`)
}

// === entity 列表展示辅助 ===
function cardStatus(
  item: EntitySummary
): 'active' | 'working' | 'disabled' | 'warning' | 'error' | 'muted' {
  switch (item.status) {
    case 'working': return 'working'
    case 'disabled': return 'disabled'
    case 'error':
    case 'failed': return 'error'
    case 'active':
    case 'connected': return 'active'
    default: return 'muted'
  }
}

/** 该列表项所属类型是否显示状态点（后端 ProviderInfo.status_indicator；缺省 true） */
function showStatusFor(item: EntitySummary): boolean {
  return activeTypes.value.find((p) => p.kind === item.kind)?.status_indicator ?? true
}

// === leaf：zip 文件选择 → 统一保存流 ===
const zipInput = ref<HTMLInputElement | null>(null)

function onZipSelected(e: Event) {
  const input = e.target as HTMLInputElement
  const file = input.files?.[0]
  input.value = ''
  if (file) void saveZip(file)
}

// === 删除（统一走核心状态机） ===
function removeSelected() {
  const sel = selected.value
  if (sel) void remove(sel.kind, sel.item.id)
}
function removeSelectedEntry() {
  const entry = selectedEntry.value
  if (entry) void remove(entry.kind, entry.id)
}

// === 机制约定：专属 editor 创建实体后上报 id → 刷新清单并选中之 ===
async function onEditorCreated(id: string) {
  const kind = createKind.value ?? selected.value?.kind
  cancelCreate()
  if (kind) {
    await refreshKind(kind)
    select(`${kind}:${id}`)
  }
}

// === 实时订阅（entity；事件总线推送，非轮询） ===
// ① entity 状态事件 → 即时补丁列表项状态角标（不重拉清单）；
// ② 该 kind 的任意 bus 事件 → 防抖刷新该类清单（机制级实时能力：
//    会话新建/删除/标题变更、模型连通状态等后端变化自动反映到列表）。
let unsubscribers: Array<() => void> = []
const pendingRefresh = new Map<string, ReturnType<typeof setTimeout>>()

function scheduleRefresh(kind: string, delay = 800) {
  const t = pendingRefresh.get(kind)
  if (t) clearTimeout(t)
  pendingRefresh.set(
    kind,
    setTimeout(() => {
      pendingRefresh.delete(kind)
      void refreshKind(kind)
    }, delay)
  )
}

onMounted(() => {
  loadAll()
  if (!isContainer) {
    unsubscribers = activeTypes.value.flatMap((d) => [
      subscribeEntityStatus(d.kind, ({ id, status, status_detail }) => {
        const it = typeStates.value[d.kind]?.items.find((x) => x.id === id)
        if (it) {
          it.status = status
          it.status_detail = status_detail ?? undefined
        }
      }),
      // 粗粒度事件才刷新清单（status=工作状态流转、title=会话命名）；
      // 流式 update 等细粒度事件与列表无关，忽略以免空闲期反复重拉
      subscribe({ kind: d.kind }, (e) => {
        const t = (e.data?.data as { type?: string } | undefined)?.type
        if (t === 'status' || t === 'title') scheduleRefresh(d.kind)
      }),
    ])
  }
})
onBeforeUnmount(() => {
  unsubscribers.forEach((fn) => fn())
  unsubscribers = []
  pendingRefresh.forEach((t) => clearTimeout(t))
  pendingRefresh.clear()
})

// containerId 变化（同记录参数复用实例）时重新加载清单
watch(containerIdRef, () => {
  if (isContainer) void loadAll()
})
</script>

<style scoped>
.workbench-page {
  display: flex;
  width: 100%;
  height: 100%;
  min-height: 0;
  overflow: hidden;
}
.workbench-page.fullscreen {
  height: 100vh;
  background: var(--surface-page);
}

/* ============== 列表 ============== */
.entity-list,
.entry-list {
  flex: 1;
  overflow-y: auto;
  padding: 0.25rem 0;
}
.meta-sub {
  opacity: 0.75;
}
.meta-container {
  font-weight: var(--font-weight-semibold);
  color: var(--text-secondary);
  max-width: 10rem;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* tag 系列用在 EntityCard 的 #meta 插槽内容上（scoped 样式不穿透插槽，须本地定义） */
.tag {
  padding: 0.1rem 0.4rem;
  background: var(--surface-sunken);
  border-radius: var(--radius-sm);
  font-size: 0.65rem;
}
.tag-muted { background: var(--surface-sunken); color: var(--text-muted); }

/* 运行状态脉冲点（#list / #meta 插槽内容） */
.running-pulse {
  display: inline-block;
  width: 0.4375rem;
  height: 0.4375rem;
  background: var(--success-solid);
  border-radius: 50%;
  animation: pulse 1.4s ease-in-out infinite;
  flex-shrink: 0;
}
@keyframes pulse {
  0%, 100% { opacity: 1; transform: scale(1); }
  50% { opacity: 0.5; transform: scale(0.85); }
}

/* ============== 新建面板（居中卡片，entity） ============== */
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
.json-input:focus {
  outline: none;
  border-color: var(--accent);
  box-shadow: 0 0 0 2px var(--accent-subtle-bg);
}

/* ============== 容器文本编辑器 ============== */
.entry-editor {
  flex: 1;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  padding: 0.75rem 1rem;
  gap: 0.5rem;
}
.editor-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 0.75rem;
  flex-shrink: 0;
  flex-wrap: wrap;
}
.title-block {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  min-width: 0;
}
.editor-title {
  margin: 0;
  font-size: var(--font-size-base);
  font-weight: var(--font-weight-semibold);
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.badge {
  font-size: 0.65rem;
  padding: 0.1rem 0.45rem;
  border-radius: var(--radius-full);
  background: var(--accent-subtle-bg);
  color: var(--accent);
  flex-shrink: 0;
}
.editor-path {
  font-size: 0.68rem;
  font-family: var(--font-mono);
  color: var(--text-muted);
  background: var(--surface-sunken);
  padding: 0.15rem 0.5rem;
  border-radius: var(--radius-sm);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.editor-form {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 0.4rem;
  min-height: 0;
  overflow-y: auto;
}
.field-hint {
  margin: 0;
  font-size: 0.72rem;
  color: var(--text-muted);
  line-height: 1.4;
}
.field-hint code {
  font-family: var(--font-mono);
  background: var(--surface-sunken);
  padding: 0.05rem 0.3rem;
  border-radius: var(--radius-sm);
}
.content-input {
  flex: 1;
  min-height: 12rem;
  padding: 0.6rem 0.75rem;
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  font-family: var(--font-mono);
  font-size: 0.8rem;
  line-height: 1.5;
  resize: vertical;
  background: var(--surface-sunken);
  color: var(--text-primary);
}
.content-input:focus {
  outline: none;
  border-color: var(--accent);
  box-shadow: 0 0 0 2px var(--accent-subtle-bg);
}
.editor-error {
  margin: 0;
  color: var(--danger-fg);
  font-size: 0.8rem;
}
.editor-actions {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding-top: 0.25rem;
}
.spacer { flex: 1; }
.no-selection {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-muted);
  font-size: 0.85rem;
}

/* ============== 通用详情工具栏（entity） ============== */
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

/* ============== 控件样式统一由 styles/controls.css 提供 ============== */
</style>
