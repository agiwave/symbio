<!--
  VdfsView — VDFS 通用资源页（全 App 唯一的资源管理器）

  页面逻辑全部在 `composables/useVdfs`（资源类型零知识），本文件只做装配：
  左栏 = 挂载点（后端注册）、中栏 = 当前目录内容、右栏 = 按节点 `ext`
  解析出的详情渲染器（`registry/vdfsRenderers` 装配）。

  与实体机制（WorkbenchView）的分工：
  - 实体机制按**类型（kind）**组织资源，导航/能力来自 entities/provider_registry；
  - VDFS 按**路径与访问位**组织资源，导航来自挂载点，能力来自 r/w/l/t。
  两者共用同一套 UI 原语（Workbench / NavRail / EntityShell / EntityCard /
  DetailForm / CodeEditor），因此新增资源只需实现 VdfsProvider，前端零开发。
-->
<template>
  <div class="vdfs-page">
    <Workbench
      :rail-items="railItems"
      :back="true"
      back-title="返回根"
      :title="title"
      :list-width="260"
      hide-default-new
      :has-list-content="items.length > 0"
      :loading="loading"
      @rail-select="switchMount"
      @rail-back="goRoot"
    >
      <template #header-actions>
        <!-- 新建（节点声明了可接受的新建类型时可见；类型由后端下发，前端不硬编码） -->
        <button
          v-if="canCreate"
          class="icon-btn"
          :title="creatableTypes.length > 1 ? '新建（选择类型）' : `新建 ${creatableTypes[0]?.title ?? ''}`"
          :disabled="loading || saving"
          @click="startTypedNew"
        >
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2">
            <line x1="12" y1="5" x2="12" y2="19" />
            <line x1="5" y1="12" x2="19" y2="12" />
          </svg>
        </button>
        <!-- 新建目录（可写目录可见；结构节点，非「新建类型」） -->
        <button
          v-if="canWriteHere"
          class="icon-btn"
          title="新建目录"
          :disabled="loading || saving"
          @click="startCreate"
        >
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
            <path d="M3 7a2 2 0 0 1 2-2h4l2 2h8a2 2 0 0 1 2 2v8a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2V7z" />
            <line x1="12" y1="11" x2="12" y2="16" />
            <line x1="9.5" y1="13.5" x2="14.5" y2="13.5" />
          </svg>
        </button>
        <button class="icon-btn" title="刷新" :disabled="loading" @click="refresh">
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M21 12a9 9 0 1 1-3-6.7L21 8" />
            <polyline points="21 3 21 8 16 8" />
          </svg>
        </button>
      </template>

      <template #list>
        <!-- 面包屑：路径即导航（VDFS 的寻址模型） -->
        <nav v-if="breadcrumbs.length > 1" class="crumbs">
          <template v-for="(c, i) in breadcrumbs" :key="c.path">
            <span v-if="i > 0" class="crumb-sep">/</span>
            <button
              type="button"
              class="crumb"
              :class="{ current: i === breadcrumbs.length - 1 }"
              @click="enter(c.path)"
            >
              {{ c.label }}
            </button>
          </template>
        </nav>

        <div class="vdfs-list" role="listbox" aria-label="资源列表">
          <EntityCard
            v-for="n in items"
            :key="n.path"
            :title="n.title || n.name"
            :subtitle="n.description"
            :status="cardStatus(n)"
            :status-title="n.status"
            :badge="badgeOf(n)"
            :badge-kind="badgeKindOf(n)"
            :tags="tagsOf(n)"
            :icon="mountOrKindIcon(n)"
            :is-active="selectedId === n.path"
            @click="activate(n)"
          />
        </div>
      </template>

      <template #empty>
        <p>{{ cwd === VFDS_ROOT ? '暂无挂载点' : '此目录为空' }}</p>
        <p v-if="canCreate" class="hint">点击右上角「新建」添加</p>
        <p v-else-if="canWriteHere" class="hint">点击右上角「新建目录」添加</p>
        <p v-else class="hint">该挂载点由系统管理</p>
      </template>

      <template #detail>
        <!-- 新建（类型化）：类型由节点 new_types 声明；多于一项先选类型 -->
        <div v-if="creatingTyped" class="vdfs-prompt">
          <template v-if="!createType">
            <h3 class="prompt-title">新建</h3>
            <p class="prompt-hint">请选择要新建的类型</p>
            <div class="type-choice-list">
              <button
                v-for="t in creatableTypes"
                :key="t.ext"
                type="button"
                class="type-choice-btn"
                @click="createType = t"
              >
                <span class="type-choice-label">{{ t.title || t.ext }}</span>
                <span class="type-choice-hint">{{ t.ext }}</span>
              </button>
            </div>
            <p v-if="detailError" class="prompt-error">{{ detailError }}</p>
            <div class="prompt-actions">
              <button class="action-btn secondary" :disabled="saving" @click="cancelTyped">取消</button>
            </div>
          </template>

          <template v-else>
            <h3 class="prompt-title">新建{{ createType.title || createType.ext }}</h3>
            <input
              v-model="typedName"
              class="prompt-input"
              placeholder="名称"
              spellcheck="false"
              @keyup.enter="submitTyped"
            />
            <p class="prompt-hint">写入地址：<code>{{ typedPreview }}</code></p>
            <p v-if="createType.description" class="prompt-hint">{{ createType.description }}</p>
            <p v-if="detailError" class="prompt-error">{{ detailError }}</p>
            <div class="prompt-actions">
              <button class="action-btn" :disabled="saving" @click="submitTyped">
                {{ saving ? '创建中…' : '创建' }}
              </button>
              <button
                v-if="creatableTypes.length > 1"
                class="action-btn secondary"
                :disabled="saving"
                @click="createType = null"
              >
                上一步
              </button>
              <button class="action-btn secondary" :disabled="saving" @click="cancelTyped">取消</button>
            </div>
          </template>
        </div>

        <!-- 新建目录（内联，写入路径实时预览） -->
        <div v-else-if="creating" class="vdfs-prompt">
          <h3 class="prompt-title">新建目录</h3>
          <input
            v-model="draftName"
            class="prompt-input"
            placeholder="目录名"
            spellcheck="false"
            @keyup.enter="submitCreate"
          />
          <p class="prompt-hint">写入路径：<code>{{ createPreview }}</code></p>
          <p v-if="detailError" class="prompt-error">{{ detailError }}</p>
          <div class="prompt-actions">
            <button class="action-btn" :disabled="saving" @click="submitCreate">
              {{ saving ? '创建中…' : '创建' }}
            </button>
            <button class="action-btn secondary" :disabled="saving" @click="cancelCreate">取消</button>
          </div>
        </div>

        <!-- 重命名（内联；同挂载点内移动） -->
        <div v-else-if="renaming && selectedNode" class="vdfs-prompt">
          <h3 class="prompt-title">重命名</h3>
          <input
            v-model="draftName"
            class="prompt-input"
            spellcheck="false"
            @keyup.enter="submitRename"
          />
          <p class="prompt-hint">
            由 <code>{{ selectedNode.path }}</code> 移动至
            <code>{{ renamePreview }}</code>
          </p>
          <p v-if="detailError" class="prompt-error">{{ detailError }}</p>
          <div class="prompt-actions">
            <button class="action-btn" :disabled="saving" @click="submitRename">
              {{ saving ? '处理中…' : '确定' }}
            </button>
            <button class="action-btn secondary" :disabled="saving" @click="cancelRename">取消</button>
          </div>
        </div>

        <!-- 详情：渲染器由节点 ext 决定（唯一分发点） -->
        <component
          :is="rendererComp"
          v-else-if="selectedNode && rendererComp"
          :key="selectedNode.path"
          :node="selectedNode"
          :data="rendererData"
          :error="detailError"
          :field-errors="fieldErrors"
          :saving="saving"
          @save="onSave"
          @delete="onDelete"
          @rename="startRename"
        />

        <div v-else-if="loadingDetail" class="vdfs-placeholder">加载中…</div>
        <div v-else class="vdfs-placeholder">
          <p>← 选择一个资源查看/编辑</p>
        </div>
      </template>
    </Workbench>
    <Toast />
  </div>
</template>

<script setup lang="ts">
import { computed, onMounted, ref, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import Workbench from '@/components/common/Workbench.vue'
import EntityCard from '@/components/common/EntityCard.vue'
import Toast from '@/components/common/Toast.vue'
import { useVdfs } from '@/composables/useVdfs'
import { getVdfsRenderer, mountIconOf, resolveVdfsRenderer } from '@/registry/vdfsTypes'
// 装配渲染器组件（副作用导入：登记 ext → 组件；本页是唯一消费方）
import '@/registry/vdfsRenderers'
import {
  VFDS_KIND_MOUNT,
  VFDS_ROOT,
  isVdfsDir,
  vdfsAccessOf,
  vdfsJoin,
  vdfsParent,
  type VdfsNewType,
  type VdfsNode,
} from '@/schemas/vdfs'
import { getEntityIcon } from '@/registry/entityTypes'

const props = defineProps<{
  /** 路由 `:mount`（可选）：深链直接进入某挂载点 */
  mount?: string
}>()

const route = useRoute()
const router = useRouter()

const {
  railItems,
  activeMount,
  switchMount,
  cwd,
  title,
  breadcrumbs,
  items,
  loading,
  canWriteHere,
  refresh,
  enter,
  activate,
  selectedNode,
  selectedId,
  renderer,
  nodeText,
  formData,
  loadingDetail,
  saving,
  detailError,
  fieldErrors,
  saveFields,
  saveText,
  removeSelected,
  createDir,
  createTyped,
  creatableTypes,
  canCreate,
  renameSelected,
  boot,
} = useVdfs({ mountParam: computed(() => (route.params.mount as string) || undefined) })

// ==================== 路由 ↔ 挂载点双向同步（深链 + 可回退） ====================
// 仅在挂载点层面同步路径，目录层级留在页内状态（挂载点才是「资源类别」）。
watch(activeMount, (m) => {
  const cur = (route.params.mount as string) || ''
  if (cur === m) return
  void router.replace(m ? `/vdfs/${m}` : '/vdfs')
})

function goRoot() {
  void enter(VFDS_ROOT)
}

onMounted(() => {
  void boot()
})

// ==================== 详情装配 ====================
/** 当前渲染器组件（未登记 → null，视图回退占位） */
const rendererComp = computed(() => {
  if (!selectedNode.value) return null
  return getVdfsRenderer(resolveVdfsRenderer(selectedNode.value)) ?? null
})

/** 渲染器数据：form → 字段值对象；文本类 → 文本；其余 → 不传 */
const rendererData = computed<unknown>(() => {
  const r = renderer.value
  if (r === 'form') return formData.value
  if (r === 'text' || r === 'json' || r === 'markdown') return nodeText.value
  return null
})

function onSave(payload: unknown) {
  if (renderer.value === 'form') void saveFields((payload ?? {}) as Record<string, unknown>)
  else void saveText()
}

function onDelete() {
  void removeSelected()
}

// ==================== 新建（类型化；类型由节点声明，§5） ====================
const creatingTyped = ref(false)
const createType = ref<VdfsNewType | null>(null)
const typedName = ref('')

/** 新建地址预览（类型 ext 决定扩展名） */
const typedPreview = computed(() => {
  const t = createType.value
  if (!t) return ''
  const name = typedName.value.trim() || '<名称>'
  return vdfsJoin(cwd.value, t.ext ? `${name}.${t.ext}` : name)
})

function startTypedNew() {
  creating.value = false
  renaming.value = false
  creatingTyped.value = true
  typedName.value = ''
  detailError.value = ''
  // 恰好一种类型：跳过类型选择，直接进入命名
  createType.value = creatableTypes.value.length === 1 ? creatableTypes.value[0] : null
}
function cancelTyped() {
  creatingTyped.value = false
  createType.value = null
  typedName.value = ''
}
async function submitTyped() {
  const t = createType.value
  if (!t) return
  if (await createTyped(t, typedName.value)) cancelTyped()
}

// ==================== 新建目录 / 重命名（页面级交互态） ====================
const creating = ref(false)
const renaming = ref(false)
const draftName = ref('')

const createPreview = computed(() => vdfsJoin(cwd.value, draftName.value || '<名称>'))
const renamePreview = computed(() =>
  selectedNode.value ? vdfsJoin(vdfsParent(selectedNode.value.path), draftName.value || '<名称>') : ''
)

function startCreate() {
  creatingTyped.value = false
  renaming.value = false
  creating.value = true
  draftName.value = ''
  detailError.value = ''
}
function cancelCreate() {
  creating.value = false
  draftName.value = ''
}
async function submitCreate() {
  if (await createDir(draftName.value)) cancelCreate()
}

function startRename() {
  if (!selectedNode.value) return
  creating.value = false
  creatingTyped.value = false
  renaming.value = true
  draftName.value = selectedNode.value.name
  detailError.value = ''
}
function cancelRename() {
  renaming.value = false
  draftName.value = ''
}
async function submitRename() {
  if (await renameSelected(draftName.value)) cancelRename()
}

// ==================== 列表项展示（机制级，无类型知识） ====================
function cardStatus(n: VdfsNode): 'active' | 'working' | 'disabled' | 'warning' | 'error' | 'muted' {
  switch (n.status) {
    case 'working': return 'working'
    case 'disabled': return 'disabled'
    case 'error': return 'error'
    case 'active': return 'active'
    default: return 'muted'
  }
}

/** 徽标：目录显示子项数，文件显示扩展名（纯 UI 呈现） */
function badgeOf(n: VdfsNode): string | undefined {
  if (isVdfsDir(n)) return typeof n.children === 'number' ? String(n.children) : undefined
  return n.ext || undefined
}
function badgeKindOf(n: VdfsNode): 'default' | 'primary' | 'success' | 'warn' | 'info' {
  return isVdfsDir(n) ? 'primary' : 'default'
}

/** 标签：可写标记 + 相对时间（访问位是唯一的能力判据） */
function tagsOf(n: VdfsNode): Array<{ label: string; kind?: 'muted' | 'primary' }> {
  const out: Array<{ label: string; kind?: 'muted' | 'primary' }> = []
  if (vdfsAccessOf(n).write) out.push({ label: '可写', kind: 'primary' })
  const t = relativeTime(n.updated_at)
  if (t) out.push({ label: t, kind: 'muted' })
  return out
}

function relativeTime(ts?: number): string {
  if (!ts) return ''
  const ms = ts < 1e12 ? ts * 1000 : ts
  const diff = Date.now() - ms
  if (diff < 0) return ''
  if (diff < 60_000) return '刚刚'
  if (diff < 3_600_000) return `${Math.floor(diff / 60_000)} 分钟前`
  if (diff < 86_400_000) return `${Math.floor(diff / 3_600_000)} 小时前`
  return `${Math.floor(diff / 86_400_000)} 天前`
}

/** 图标：挂载点用挂载点图标，其余按节点 kind（均为纯 UI 映射） */
function mountOrKindIcon(n: VdfsNode) {
  if (n.kind === VFDS_KIND_MOUNT) return mountIconOf(n.name) ?? undefined
  return getEntityIcon(n.kind) ?? mountIconOf(n.kind) ?? undefined
}

// props.mount 仅作路由声明；实际取值走 route.params（两者同源）
void props
</script>

<style scoped>
.vdfs-page {
  display: flex;
  width: 100%;
  height: 100vh;
  min-height: 0;
  overflow: hidden;
  background: var(--surface-page);
}

/* ============== 面包屑 ============== */
.crumbs {
  display: flex;
  align-items: center;
  gap: 0.15rem;
  flex-wrap: wrap;
  padding: 0.4rem 0.75rem;
  border-bottom: 1px solid var(--border-default);
  flex-shrink: 0;
  font-size: 0.72rem;
}
.crumb {
  border: none;
  background: none;
  color: var(--accent);
  cursor: pointer;
  padding: 0.1rem 0.25rem;
  border-radius: var(--radius-sm);
}
.crumb:hover { background: var(--surface-hover); }
.crumb.current {
  color: var(--text-primary);
  font-weight: var(--font-weight-semibold);
  cursor: default;
}
.crumb-sep { color: var(--text-muted); }

/* ============== 列表 ============== */
.vdfs-list {
  flex: 1;
  overflow-y: auto;
  padding: 0.25rem 0;
}

/* ============== 内联提示栏（新建 / 重命名） ============== */
.vdfs-prompt {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
  padding: 1rem;
  max-width: 28rem;
}
.prompt-title {
  margin: 0;
  font-size: var(--font-size-base);
  font-weight: var(--font-weight-semibold);
  color: var(--text-primary);
}
.prompt-input {
  padding: 0.45rem 0.7rem;
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: var(--surface-sunken);
  color: var(--text-primary);
  font-size: 0.85rem;
}
.prompt-input:focus {
  outline: none;
  border-color: var(--accent);
  box-shadow: 0 0 0 2px var(--accent-subtle-bg);
}
.prompt-hint {
  margin: 0;
  font-size: 0.72rem;
  color: var(--text-muted);
}
.prompt-hint code {
  font-family: var(--font-mono);
  background: var(--surface-sunken);
  padding: 0.05rem 0.3rem;
  border-radius: var(--radius-sm);
}
.prompt-error {
  margin: 0;
  font-size: 0.78rem;
  color: var(--danger-fg);
}
.prompt-actions {
  display: flex;
  gap: 0.5rem;
  padding-top: 0.25rem;
}

/* ============== 新建类型选择（类型由节点声明） ============== */
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
  transition: border-color var(--motion-fast) var(--motion-ease),
    background var(--motion-fast) var(--motion-ease);
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
  font-family: var(--font-mono);
}

/* ============== 占位 ============== */
.vdfs-placeholder {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-muted);
  font-size: 0.85rem;
}
</style>
