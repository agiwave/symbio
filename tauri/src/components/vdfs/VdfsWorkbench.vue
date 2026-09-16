<!--
  VdfsWorkbench — 三栏工作台控件（全 App 唯一实现，可复用）

  绑定一个 **vdfs 数据地址**（`addr`）自包含渲染，控件不知道浏览器路由
  （数据地址与浏览器地址是两个概念）：

  - **左栏** = `addr` 的内容（子目录导航，来自 `useVdfs` 唯一数据逻辑）；
  - **中栏** = 选中的左栏子目录内容；点文件选中、点目录钻入；
  - **右栏** = 选中项详情（渲染器按节点 `ext` 解析，`registry/vdfsRenderers` 装配）。

  ## 与宿主的边界

  - **钻入**（点中栏目录 / 详情「浏览内部」）：只 `emit('open', addr)`，
    呈现方式由宿主决定——通常是 push 一个新地址页（同一控件承接）；
  - **宿主件**：`rail-header`（左上角，如返回键 / logo）、`rail-footer`
    （左下角，如系统目录入口）由宿主注入，控件不关心宿主是谁——首页绑
    `.vdfs`、会话/智能体内部页绑 `.vdfs/session/<id>`，对控件毫无区别。
-->
<template>
  <div class="vdfs-workbench">
    <Workbench
      :rail-items="navItems"
      :title="title"
      :list-width="260"
      hide-default-new
      :has-list-content="items.length > 0"
      :loading="loading"
      @rail-select="selectDir"
    >
      <template #rail-header><slot name="rail-header" /></template>
      <template #rail-footer><slot name="rail-footer" /></template>

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
        <button class="icon-btn" title="刷新" :disabled="loading" @click="refresh">
          <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2">
            <path d="M21 12a9 9 0 1 1-3-6.7L21 8" />
            <polyline points="21 3 21 8 16 8" />
          </svg>
        </button>
      </template>

      <template #list>
        <div class="vdfs-list" role="listbox" aria-label="资源列表" @scroll.passive="onListScroll">
          <VdfsCard
            v-for="n in items"
            :key="n.path"
            :title="n.title || n.name"
            :subtitle="n.description"
            :status="cardStatus(n)"
            :status-title="n.status"
            :badge="badgeOf(n)"
            :badge-kind="badgeKindOf(n)"
            :tags="tagsOf(n)"
            :icon="iconOf(n)"
            :is-active="selectedId === n.path"
            @click="onItemClick(n)"
          />
          <!-- 有界列表的收尾：滚到底自动续页，也留一个显式入口 -->
          <div v-if="hasMore" class="list-more">
            <button
              v-if="!loadingMore"
              class="more-btn"
              :disabled="loading"
              @click="loadMore"
            >
              加载更早
            </button>
            <span v-else class="more-hint">加载中…</span>
          </div>
        </div>
      </template>

      <template #empty>
        <!-- 列表加载失败与「目录为空」是两件事：前者必须说出来，否则会被读成「没有数据」 -->
        <p v-if="loadError" class="prompt-error">{{ loadError }}</p>
        <template v-else>
          <p>{{ selectedName ? '此目录为空' : '暂无子目录' }}</p>
          <p v-if="canCreate" class="hint">点击右上角「新建」添加</p>
          <p v-else class="hint">该目录由系统管理</p>
        </template>
      </template>

      <template #detail>
        <!-- 新建：多于一种类型时先选类型。选完**直接进入该类型的详情页**
             （草稿态：无 id / 名字），与「选中一项」是同一条通道——名字要么在
             详情页里产生（如会话的首条消息），要么由后端生成。 -->
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
                @click="chooseType(t)"
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

          <!-- 内容来自本地文件（如 zip 整包导入）：内容在打开详情页之前就已齐备，
               没有「边看边填」的过程，因此选文件即完成（唯一不进详情页的新建形态） -->
          <template v-else>
            <h3 class="prompt-title">导入{{ createType.title || createType.ext }}</h3>
            <input
              type="file"
              class="prompt-input"
              :accept="createType.ext ? `.${createType.ext}` : undefined"
              @change="onTypedFile"
            />
            <p class="prompt-hint">写入地址：<code>{{ typedFilePreview }}</code></p>
            <p v-if="createType.description" class="prompt-hint">{{ createType.description }}</p>
            <p v-if="detailError" class="prompt-error">{{ detailError }}</p>
            <div class="prompt-actions">
              <button class="action-btn" :disabled="saving || !typedFile" @click="submitTypedFile">
                {{ saving ? '导入中…' : '导入' }}
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

        <!-- 重命名（内联；同一地址空间内移动） -->
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

        <!-- 详情：渲染器由节点 ext 决定（唯一分发点）。**草稿（新建态）也走这里**
             ——同一个 ext 用同一个渲染器，因此「点新建」与「选中一项」在交互上
             没有第二种形态（key 对草稿另取，保证连续新建时重挂载）。 -->
        <component
          :is="rendererComp"
          v-else-if="selectedNode && rendererComp"
          :key="selectedNode.path || `draft-${draftSeq}`"
          :node="selectedNode"
          :data="rendererData"
          :error="detailError"
          :field-errors="fieldErrors"
          :saving="saving"
          :testing="actionBusy"
          @save="onSave"
          @delete="onDelete"
          @action="onAction"
          @created="onCreated"
          @rename="startRename"
          @browse="browseInto"
        />

        <div v-else-if="loadingDetail" class="vdfs-placeholder">加载中…</div>
        <div v-else class="vdfs-placeholder">
          <p>← 选择一个资源查看/编辑</p>
        </div>
      </template>
    </Workbench>
  </div>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import Workbench from '@/components/common/Workbench.vue'
import VdfsCard from '@/components/common/VdfsCard.vue'
import { useVdfs } from '@/composables/useVdfs'
import { getVdfsRenderer, dirIconOf, resolveVdfsRenderer } from '@/registry/vdfsTypes'
// 装配渲染器组件（副作用导入：登记 ext → 组件；本控件是唯一消费方）
import '@/registry/vdfsRenderers'
import {
  VDFS_NEW_SOURCE_FILE,
  isVdfsDir,
  newFileNameOf,
  vdfsAccessOf,
  vdfsJoin,
  vdfsParent,
  type VdfsNewType,
  type VdfsNode,
} from '@/schemas/vdfs'
import { getVdfsIcon, getVdfsIconFor } from '@/registry/vdfsIcons'

const props = defineProps<{
  /** 绑定的 vdfs 数据地址（如 `.vdfs` 或 `.vdfs/session/<id>`）；变化 = 整体重载 */
  addr: string
}>()

const emit = defineEmits<{
  /** 钻入某目录的数据地址（点中栏目录 / 详情「浏览内部」）；呈现方式由宿主决定 */
  (e: 'open', addr: string): void
}>()

const {
  navItems,
  selectDir,
  selectedName,
  cwd,
  title,
  items,
  loading,
  loadError,
  hasMore,
  loadingMore,
  refresh,
  loadMore,
  select,
  selectedNode,
  selectedId,
  renderer,
  nodeText,
  formData,
  loadingDetail,
  saving,
  actionBusy,
  detailError,
  fieldErrors,
  saveFields,
  saveText,
  runAction,
  removeSelected,
  startNew,
  createTypedFile,
  creatableTypes,
  canCreate,
  draftSeq,
  renameSelected,
} = useVdfs({ addr: computed(() => props.addr) })

// ==================== 钻入（emit，宿主决定呈现） ====================

/**
 * 点中栏列表项：文件 → 选中（右栏详情）；目录 → 钻入其数据地址
 * （emit `open`，宿主通常 push 一个新地址页，同一控件承接）。
 */
function onItemClick(n: VdfsNode) {
  if (!isVdfsDir(n)) return void select(n)
  emit('open', n.path)
}

/**
 * 滚到底自动续页。
 *
 * 阈值取一屏的零头（200px）：**不等真正触底**，滚到快到底就开始取，
 * 用户基本感知不到等待；`loadMore` 内部有 `loadingMore` / `hasMore` 双闸门，
 * 滚动事件再密集也只会有一个在途请求。
 */
function onListScroll(e: Event) {
  const el = e.target as HTMLElement | null
  if (!el) return
  if (el.scrollHeight - el.scrollTop - el.clientHeight < 200) void loadMore()
}

/**
 * 详情「浏览内部」/ open-container：钻入容器目录的数据地址
 * （emit `open`）。会话节点本身是**叶子**（点击 = 聊天详情，语义不变）；
 * 其内部结构挂在**同名目录路径**之下（`<id>/子会话`、`<id>/工作目录`）。
 */
function browseInto() {
  const n = selectedNode.value
  if (!n) return
  emit('open', n.path)
}

// ==================== 详情装配 ====================
/** 当前渲染器组件（未登记 → null，视图回退占位） */
const rendererComp = computed(() => {
  if (!selectedNode.value) return null
  return getVdfsRenderer(resolveVdfsRenderer(selectedNode.value)) ?? null
})

/** 渲染器数据：form → 字段值对象；文本类（含 message 的正文）→ 文本；其余 → 不传 */
const rendererData = computed<unknown>(() => {
  const r = renderer.value
  if (r === 'form') return formData.value
  if (r === 'text' || r === 'json' || r === 'markdown' || r === 'message') return nodeText.value
  return null
})

function onSave(payload: unknown) {
  if (renderer.value === 'form') void saveFields((payload ?? {}) as Record<string, unknown>)
  else void saveText()
}

function onDelete() {
  void removeSelected()
}

/**
 * 节点动作（如「测试连接」）：标识由详情定义声明、由 provider 解释，
 * 控件只负责转发并呈现结果。
 */
function onAction(id: string) {
  void runAction(id)
}

// ==================== 新建（类型化；类型由节点声明，§5） ====================
//
// 两种形态，判据是**内容是否在打开详情页之前就已齐备**：
//
// - 缺省：点新建 = **进入该类型的详情页**（草稿态，无 id / 名字）——名字要么
//   在详情页里产生（如会话的首条消息），要么由后端生成（写目录自身，
//   见 `useVdfs.write`）；
// - `source = file`：内容就是那份本地文件，选文件即完成（进详情页无事可做）。
//
// 多于一种类型时先让用户选类型，选完立刻落到上面两条之一。

const creatingTyped = ref(false)
const createType = ref<VdfsNewType | null>(null)
/** `source = file` 的类型：待导入的本地文件 */
const typedFile = ref<File | null>(null)

/** 导入地址预览（目标名由**文件名**推导，故用户不填名） */
const typedFilePreview = computed(() => {
  const t = createType.value
  if (!t) return ''
  const f = typedFile.value
  if (!f) return vdfsJoin(cwd.value, `<文件名>${t.ext ? `.${t.ext}` : ''}`)
  return vdfsJoin(cwd.value, newFileNameOf(f.name, t.ext))
})

function onTypedFile(e: Event) {
  const input = e.target as HTMLInputElement
  typedFile.value = input.files?.[0] ?? null
  detailError.value = ''
}

async function submitTypedFile() {
  const t = createType.value
  const f = typedFile.value
  if (!t || !f) return
  if (await createTypedFile(t, f)) cancelTyped()
}

/** 选定类型 → 落到「进详情页」或「选文件」两条路之一 */
function chooseType(t: VdfsNewType) {
  if (t.source === VDFS_NEW_SOURCE_FILE) {
    createType.value = t
    return
  }
  // 进详情页 = 收起类型面板 + 选中一张草稿节点（与选中一项同一条通道）
  creatingTyped.value = false
  createType.value = null
  detailError.value = ''
  startNew(t)
}

function startTypedNew() {
  renaming.value = false
  typedFile.value = null
  detailError.value = ''
  const types = creatableTypes.value
  // 恰好一种类型：跳过类型选择，直接落到那一条路
  const only = types[0]
  if (types.length === 1 && only) {
    chooseType(only)
    return
  }
  creatingTyped.value = true
  createType.value = null
}
function cancelTyped() {
  creatingTyped.value = false
  createType.value = null
  typedFile.value = null
}

/**
 * 详情渲染器完成资源创建后上报新节点的 **id**（如新建会话落库）：刷新清单并选中它。
 *
 * 草稿态由机制进入、创建由渲染器发起（只有它知道怎么建），落点再交回机制统一
 * 处理——于是「新建 → 详情页 → 创建 → 选中」是一条闭合通道，本控件不需要认识
 * 任何具体资源类型。
 */
async function onCreated(id: string) {
  if (!id) return
  await refresh()
  const target = items.value.find((n) => n.path === vdfsJoin(cwd.value, id))
  if (target) void select(target)
}

// ==================== 重命名（内联交互态；S11 起不再有「新建目录」入口） ==========
const renaming = ref(false)
const draftName = ref('')

const renamePreview = computed(() =>
  selectedNode.value ? vdfsJoin(vdfsParent(selectedNode.value.path), draftName.value || '<名称>') : ''
)

function startRename() {
  if (!selectedNode.value) return
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

/** 标签：可写标记（访问位是唯一能力判据）+ 后端声明的类型特有标签 + 相对时间 */
function tagsOf(n: VdfsNode): Array<{ label: string; kind?: 'muted' | 'primary' }> {
  const out: Array<{ label: string; kind?: 'muted' | 'primary' }> = []
  if (vdfsAccessOf(n).write) out.push({ label: '可写', kind: 'primary' })
  // `meta_tags` 是后端决定的类型特有标签（VDFS 只透传），
  // 前端原样渲染、不含语义（如会话的工作目录名 / 消息数）
  const tags = n.meta_tags
  if (Array.isArray(tags)) {
    for (const label of tags) {
      if (typeof label === 'string' && label) out.push({ label, kind: 'muted' })
    }
  }
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

/**
 * 图标：目录用目录名映射的图标；文件按「kind + 项级扩展名」查项级图标，
 * 再回退 kind 级。全部是纯 UI 映射（VDFS 不下发图标）。
 * 项级标识直接读节点顶层的 `config_type`（后端 flatten 下发），缺省回落节点名。
 */
function iconOf(n: VdfsNode) {
  if (isVdfsDir(n)) return dirIconOf(n.name) ?? undefined
  const ext = typeof n.config_type === 'string' && n.config_type ? n.config_type : n.name
  return (
    getVdfsIconFor({ kind: n.kind, config_type: ext }) ??
    getVdfsIcon(n.kind) ??
    dirIconOf(n.kind) ??
    undefined
  )
}
</script>

<style scoped>
.vdfs-workbench {
  display: flex;
  width: 100%;
  height: 100vh;
  min-height: 0;
  overflow: hidden;
  background: var(--surface-page);
}

/* ============== 列表 ============== */
.vdfs-list {
  flex: 1;
  overflow-y: auto;
  padding: 0.25rem 0;
}

/* 有界列表的收尾（滚到底可见；也提供显式入口） */
.list-more {
  display: flex;
  justify-content: center;
  padding: 0.75rem 0 1rem;
}
.more-btn {
  padding: 0.35rem 1rem;
  border: 1px solid var(--border-default);
  border-radius: 999px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 0.8125rem;
  cursor: pointer;
}
.more-btn:hover:not(:disabled) {
  background: var(--surface-hover);
}
.more-btn:disabled {
  opacity: 0.5;
  cursor: default;
}
.more-hint {
  font-size: 0.8125rem;
  color: var(--text-muted);
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
/* 文件选择器（整包导入）沿用输入框的框体，但按原生控件排版 */
.prompt-input[type='file'] {
  width: 100%;
  padding: 0.35rem 0.5rem;
  font-size: 0.78rem;
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
