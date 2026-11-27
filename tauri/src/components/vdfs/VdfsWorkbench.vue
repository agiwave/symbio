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
  - **宿主件**：`rail-header`（左上角，如返回键）、`rail-footer`
    （左下角，如系统目录入口）由宿主注入，控件不关心宿主是谁——首页绑
    `<根>`、会话/智能体内部页绑 `<根>/session/<id>`，对控件毫无区别。
-->
<template>
  <Workbench
    :rail-items="navItems"
    :title="title"
    :has-list-content="filteredItems.length > 0 || (treeMode && treeNodes.length > 0)"
    :has-detail="hasDetail"
    :has-draft="isDraftSelected"
    :can-create="canCreate"
    :loading="loading"
    @rail-select="onRailSelect"
  >
    <template #rail-header><slot name="rail-header" /></template>
    <template #rail-footer><slot name="rail-footer" /></template>

    <template #header-actions>
      <!-- 新建（节点声明了可接受的新建类型时可见；类型由后端下发，前端不硬编码）。
           点一下**直接进入该类型的详情页**（草稿态）——没有「选方式」这一步：
           整包导入是详情页上的一条动作，不是第二种新建入口。

           视觉上它是本栏**唯一的主操作**（填充主题色），与「刷新」这类次要图标
           按钮明确分开：此前两者同为透明 icon-btn，用户要读 tooltip 才知道哪个
           能建东西；而「没内容时该做什么」正是最需要一眼看出的时刻。 -->
      <button
        v-if="canCreate"
        class="new-btn"
        :title="`新建 ${creatableType?.title ?? ''}`"
        :aria-label="`新建 ${creatableType?.title ?? ''}`"
        :disabled="loading || saving || actionBusy"
        @click="startNewFromButton()"
      >
        <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2.25" stroke-linecap="round">
          <line x1="12" y1="5" x2="12" y2="19" />
          <line x1="5" y1="12" x2="19" y2="12" />
        </svg>
      </button>
      <button
        class="icon-btn refresh-btn"
        :class="{ 'is-loading': loading }"
        :title="loading ? '正在刷新' : '刷新'"
        :aria-label="loading ? '正在刷新' : '刷新'"
        :disabled="loading"
        @click="refresh"
      >
        <svg viewBox="0 0 24 24" width="16" height="16" fill="none" stroke="currentColor" stroke-width="2">
          <path d="M21 12a9 9 0 1 1-3-6.7L21 8" />
          <polyline points="21 3 21 8 16 8" />
        </svg>
      </button>
      <span class="sr-only" role="status">{{ loading ? '正在刷新资源列表' : '' }}</span>
    </template>

    <template #list>
      <!-- 筛选框：**服务端声明**了才出现（`cwdNode.search`）——与「新建」看
           `new_type` 同一条通道，本控件不自判「条目够不够多」。
           目录条目少（服务端认定不值得检索）、或空目录时都不给：一个筛不出东西的
           搜索框只会让用户以为"可以搜库"。
           筛选本身是纯客户端投影（见 useVdfs 的说明），不发请求、不动分页游标。 -->
      <div v-if="searchable && !treeMode" class="vdfs-filter">
        <input
          v-model="filter"
          class="vdfs-filter-input"
          type="search"
          placeholder="筛选当前目录…"
          spellcheck="false"
          aria-label="筛选当前目录条目"
          @keydown.esc="filter = ''"
        />
        <button
          v-if="filter"
          class="filter-clear"
          type="button"
          title="清除筛选"
          aria-label="清除筛选"
          @click="filter = ''"
        >
          ×
        </button>
      </div>

      <div class="vdfs-list-region">
        <div
          v-overlay-scrollbar
          ref="listRef"
          class="vdfs-list"
          :role="treeMode ? 'tree' : 'listbox'"
          :aria-label="treeMode ? '资源目录树' : '资源列表'"
          :aria-busy="loading"
          @scroll.passive="onListScroll"
          @focusin="onListFocusIn"
        >
          <!-- 目录树形态：当前目录声明 `t` 位时，中栏改渲染可展开的树。
               点目录就地展开 / 折叠，点文件走与列表项相同的 onItemClick（右栏出详情）。 -->
          <template v-if="treeMode">
            <VdfsTreeNode
              v-for="(n, index) in treeNodes"
              :key="n.item.path"
              :node="n"
              :selected-id="selectedId"
              :focus-path="focusedPath"
              :tab-stop="isTreeTabStop(n.item.path, index === 0)"
              @activate="onItemClick"
              @load-children="loadTreeChildren"
            />
          </template>

          <!-- 扁平列表形态（未声明 `t` 位的目录）。
               徽标只给目录（子项数）。⚠️ 文件不给徽标：`ext` 是**渲染器键**
               （`form` / `session` 之类），属机制细节，不该出现在给用户看的列表里。 -->
          <template v-else-if="!treeMode">
            <VdfsCard
              v-for="(n, index) in filteredItems"
              :key="n.path"
              :title="n.title || n.name"
              :subtitle="n.description"
              :status="cardStatusOf(n)"
              :status-title="cardStatusTextOf(n)"
              :show-status="Boolean(n.status)"
              :badge="cardBadgeOf(n)"
              badge-kind="primary"
              :tags="cardTagsOf(n, now)"
              :icon="cardIconOf(n)"
              :is-active="selectedId === n.path"
              :tab-stop="isListTabStop(n.path, index === 0)"
              :data-path="n.path"
              @click="onItemClick(n)"
            />
          </template>
        </div>

        <!-- 续页控件放在 listbox/tree 外：复合组件里只允许选项，不允许混入按钮。 -->
        <div v-if="hasMore" class="list-more">
          <!-- 续页失败不能把入口藏起来：保留按钮就是保留重试，错误行说明为什么没动。 -->
          <p v-if="loadMoreError" class="list-more-error" role="alert">{{ loadMoreError }}</p>
          <button
            v-if="!loadingMore"
            class="more-btn"
            :disabled="loading"
            @click="loadMore"
          >
            {{ loadMoreError ? '重试' : '加载更早' }}
          </button>
          <span v-else class="more-hint">加载中…</span>
        </div>
      </div>
    </template>

    <template #empty>
      <!-- 列表加载失败与「目录为空」是两件事：前者必须说出来，否则会被读成「没有数据」 -->
      <template v-if="loadError">
        <p class="prompt-error">{{ loadError }}</p>
        <button class="empty-action" type="button" :disabled="loading" @click="refresh">重试</button>
      </template>
      <!-- 「筛掉了」与「本来就没有」也是两件事：前者要告诉用户还有更早的页可捞 -->
      <template v-else-if="filterEmpty">
        <p>没有匹配「{{ activeFilter }}」的条目</p>
        <p v-if="filterTruncated" class="hint">还有更早的条目未加载，可能在其中</p>
        <button class="empty-action" type="button" @click="filter = ''">清除筛选</button>
      </template>
      <!-- 空态即引导：目录自己声明了可新建的类型（`new_type`）时，直接给一个
           **可执行**的入口，而不是让用户去找右上角那个图标（「没内容时该做什么」
           正是最需要一眼看出的时刻）。文案里的类型名来自**类型自己的 title**
           （后端下发），因此新增一类资源这里零改动——不存在任何类型清单。 -->
      <template v-else>
        <p>{{ selectedName ? '此目录为空' : '暂无子目录' }}</p>
        <button
          v-if="canCreate"
          class="empty-action primary"
          type="button"
          :disabled="loading || saving || actionBusy"
          @click="startNewFromButton()"
        >
          新建第一个{{ creatableType?.title ?? '条目' }}
        </button>
        <p v-else class="hint">该目录由系统管理</p>
      </template>
    </template>

    <template #detail>
      <div
        v-if="testResult && !loadingDetail && !detailLoadError"
        class="vdfs-action-result is-success"
        role="status"
      >
        <span aria-hidden="true">✓</span>
        <p>{{ testResult.message }}</p>
      </div>
      <!-- 详情：渲染器由节点 ext 决定（唯一分发点）。**草稿（新建态）也走这里**
           ——同一个 ext 用同一个渲染器，因此「点新建」与「选中一项」在交互上
           没有第二种形态（key 对草稿另取，保证连续新建时重挂载）。

           机制动作（删除）由**本控件**算一次后注入所有渲染器：它们是
           「页面对任何已落盘可写节点都能做的默认动作」，不该由每个渲染器各算一遍
           （见 useVdfs.mechanismActions）。渲染器只声明**它自己特有**的动作。 -->
      <!-- 详情加载中：骨架必须优先于真实表单 / 正文。选中真实节点后 vdfs/read
           可能仍在途，此时挂载表单会把尚未读到的 null / 空串显示成默认可编辑值。 -->
      <div v-if="loadingDetail" class="vdfs-detail-skeleton" aria-busy="true">
        <SkeletonBlock :width="'38%'" :height="0.95" />
        <SkeletonBlock v-for="i in DETAIL_SKELETON_ROWS" :key="i" :lines="1" :height="0.85" />
      </div>

      <!-- 读取失败独立于保存 / 动作失败：前者卸载未就绪详情并给出重试，后者仍把
           错误传给当前渲染器，保留表单与用户输入。 -->
      <div v-else-if="detailLoadError" class="vdfs-detail-error" role="alert">
        <p>{{ detailLoadError }}</p>
        <button class="more-btn" type="button" @click="retryDetail">重试详情</button>
      </div>

      <component
        :is="rendererComp"
        v-else-if="selectedNode && rendererComp"
        ref="rendererRef"
        :key="selectedNode.path || `draft-${draftSeq}`"
        :node="selectedNode"
        :data="rendererData"
        :error="detailError"
        :field-errors="fieldErrors"
        :saving="saving"
        :testing="actionBusy"
        :mechanism-actions="mechanismActions"
        :mechanism-busy="mechanismBusyId"
        @save="onSave"
        @delete="onDelete"
        @action="onAction"
        @created="onCreated"
        @browse="browseInto"
        @open-node="onOpenNode"
        @dirty-change="onDetailDirty"
      />

      <div v-else class="vdfs-placeholder">
        <!-- 深链指向的条目没找到时，占位符必须给出理由：否则「← 选择一个资源」
             读起来像是用户自己没选，而不是地址里的那一项失效了。 -->
        <p v-if="focusNotice" class="placeholder-notice">{{ focusNotice }}</p>
        <p v-else>← 选择一个资源查看/编辑</p>
      </div>
    </template>
  </Workbench>

  <!-- 删除确认：目录是递归删除，节点内容不可撤销。与消息删除同一套确认件。 -->
  <ConfirmDialog
    :visible="pendingDelete"
    title="删除资源"
    :message="deleteConfirmMessage"
    :confirm-text="deleteTargetIsDir ? '递归删除' : '删除'"
    danger
    icon="🗑"
    icon-kind="danger"
    @confirm="confirmDelete"
    @cancel="cancelDelete"
  />

  <!-- 未保存离开确认：详情表单/文本改到一半就切走会静默丢内容，这里兜住。
       confirm 时执行被拦下的导航，cancel 时留在当前详情。 -->
  <ConfirmDialog
    :visible="pendingLeave"
    title="放弃未保存的修改？"
    message="当前资源有尚未保存的修改，离开后这些修改将丢失。"
    confirm-text="放弃并离开"
    cancel-text="继续编辑"
    danger
    icon="⚠"
    icon-kind="danger"
    @confirm="confirmLeave"
    @cancel="cancelLeave"
  />
</template>

<script setup lang="ts">
import { computed, nextTick, onBeforeUnmount, onMounted, ref, shallowRef, watch } from 'vue'
import { vOverlayScrollbar } from '@/directives/overlayScrollbar'
import Workbench from '@/components/common/Workbench.vue'
import VdfsCard from '@/components/common/VdfsCard.vue'
import VdfsTreeNode from './VdfsTreeNode.vue'
import ConfirmDialog from '@/components/common/ConfirmDialog.vue'
import SkeletonBlock from '@/components/common/SkeletonBlock.vue'
import { useHotkeys } from '@/composables/useHotkeys'
import { useVdfs } from '@/composables/useVdfs'
import {
  getVdfsRenderer,
  isFormRenderer,
  isSchemaDrivenExt,
  isSessionExt,
  rendererPayloadIsText,
  rendererReadsNodeSchema,
  rendererReadsNodeText,
  resolveVdfsRenderer,
} from '@/registry/vdfsTypes'
// 装配渲染器组件（副作用导入：登记 ext → 组件；本控件是唯一消费方）
import '@/registry/vdfsRenderers'
// 列表卡片的呈现映射（状态点 / 文案 / 徽标 / 标签 / 图标）
import {
  cardBadgeOf,
  cardIconOf,
  cardStatusOf,
  cardStatusTextOf,
  cardTagsOf,
} from '@/registry/vdfsCards'
import {
  isVdfsDir,
  isVdfsDraft,
  vdfsBase,
  vdfsJoin,
  vdfsParent,
  type VdfsItem,
  type VdfsTreeNode as VdfsTreeNodeData,
} from '@/schemas/vdfs'
import { vdfsRoot } from '@/schemas/vdfsRoot'
import { useSessionsStore } from '@/stores/sessions'
import { NEW_RESOURCE_EVENT } from '@/utils/newResource'

const props = defineProps<{
  /** 绑定的 vdfs 数据地址（如 `<根>` 或 `<根>/session/<id>`）；变化 = 整体重载 */
  addr: string
  /**
   * 到达后要顺带选中的**末段名**（`addr` 的直接子项；如 `?select=main.rs`）。
   *
   * 「打开文件」没有独立地址页，只有「父目录 + 选中项」这一种形态；本控件不认识
   * 名字的含义，选中后仍走同一条详情通道（渲染器按 ext 决定）。
   */
  select?: string | null
}>()

const emit = defineEmits<{
  /**
   * 打开一个数据地址；`select` = 该目录下要顺带选中的末段名（可选）。
   *
   * 钻入目录（点中栏目录 / 详情「浏览内部」）只给 `addr`；打开**文件**没有自己的
   * 地址页（三栏的右栏才是详情），于是给「父目录 + `select`」，宿主据此 push
   * 一个「父目录地址页 + 选中项」。
   */
  (e: 'open', addr: string, select?: string): void
}>()

const {
  navItems,
  selectDir,
  selectedName,
  pinnedAtAddr,
  cwd,
  cwdNode,
  cwdAddr,
  title,
  items,
  treeMode,
  treeNodes,
  loadTreeChildren,
  filteredItems,
  searchable,
  filter,
  activeFilter,
  filterEmpty,
  filterTruncated,
  loading,
  loadError,
  hasMore,
  loadingMore,
  loadMoreError,
  refresh: refreshVdfs,
  loadMore,
  select,
  atMountLevel,
  selectedNode,
  selectedId,
  focusNotice,
  renderer,
  nodeText,
  formData,
  loadingDetail,
  detailLoadError,
  retryDetail,
  saving,
  actionBusy,
  detailError,
  testResult,
  clearTestResult,
  fieldErrors,
  mechanismActions,
  mechanismBusyId,
  saveFields,
  saveText,
  write,
  runAction,
  runPackAction,
  removeSelected,
  startNew,
  creatableType,
  canCreate,
  draftSeq,
} = useVdfs({
  addr: computed(() => props.addr),
  select: computed(() => props.select ?? null),
})

// ==================== 当前空间声明 ====================
//
// 「能新建会话的目录」就是**会话挂载目录**——判据与 `services/vdfsScheme`
// 认出根挂载用的是同一个（`new_type.ext === 'session'`），因此这里不需要任何
// 段名字面量。会话挂载**不止一份**（根空间 / 各子智能体空间各一棵子树），
// 所以「我在哪个空间」是个随目录变化的事实，而不是常量。
//
// 为什么由工作台声明：**新建态（草稿）没有地址**——草稿节点的 `path` 是空串，
// 拿不到任何「住在哪个空间」的信息。而它是从**当前目录**进的（`startNew`），
// 当前目录就是那个空间。于是把这条事实交给 store（清单读哪份、新建写哪份、
// 发送落哪个 inbox 都按它寻址），草稿态的懒创建才不会建到根空间去。
//
// 选中一个**已存在**的会话时不必依赖它：那条路径的地址里本来就带着挂载目录
// （`Session.vue` 传 `node.path` → `store.selectSession` 解析）。
//
// ⚠️ 判据必须三条同时成立，缺一条都会把**不是挂载目录**的地址声明成会话空间：
// 1. 自述声明 `new_type.ext === 'session'`（判据与 `services/vdfsScheme` 同源）；
// 2. 它是**目录**（访问位含 `l`）——会话节点自己的 ext 也是 `session`，但它是叶子
//    （`rw`）。少了这条，选中一个会话就会把 `<挂载目录>/<id>` 当成"空间"，
//    清单、新建、转写订阅全部按一个不是挂载目录的地址寻址；
// 3. 自述与当前目录**同源**（`cwdAddr === cwd`）——换目录时 `cwdNode` 一步滞后，
//    少了这条会把上一栏的类型套到这一栏（进入会话后切到 `message` / `subsession`
//    就把它们声明成了会话空间）。
const sessionsStore = useSessionsStore()
watch(
  () => {
    const node = cwdNode.value
    if (!node || !isSessionExt(node.new_type?.ext)) return ''
    if (cwdAddr.value !== cwd.value) return ''
    return isVdfsDir(node) ? cwd.value : ''
  },
  (dir) => {
    if (dir) sessionsStore.setSessionSpace(dir)
  },
  { immediate: true },
)

// ==================== 空目录自动备好一张草稿 ====================
//
// 只有**已加载完成、确实为空**的可新建目录才自动铺草稿（新用户引导）；已有条目的
// 目录默认保持「列表 + 空详情」，草稿只由用户点新建 / Ctrl+N / 命令面板打开。
//
// 自述与条目必须同源：一次 list 回执同时更新 items、cwdNode、cwdAddr；只有
// `cwdAddr === cwd` 才认这份自述，且 loading / loadError 期间不把空数组当结果。
// 类型仍只来自 `creatableType`（它内部也核同源），不出现任何资源类型名。
//
// 深链在 list 之后还要 select / stat 落位，因此仅看 selectedNode 不够；pinnedAtAddr
// 覆盖整个到达态，避免慢 stat 时被草稿抢走。每个目录只自动开一次，清空选中或删掉
// 条目不会让草稿「复活」。schema 驱动的选择器页（如插件市场）同样不自动开。
let autoDraftDone = false
/** undefined = 没有待处理的明确新建；null = 当前页；字符串 = 目标目录。 */
let pendingNewTarget: string | null | undefined
const autoDraftContext = computed(() => ({
  dir: cwd.value,
  selfAddr: cwdAddr.value,
  itemCount: items.value.length,
  loading: loading.value,
  loadError: loadError.value,
  selected: selectedNode.value,
  deepLinked: pinnedAtAddr.value,
  type: creatableType.value,
}))
watch(
  autoDraftContext,
  (ctx, prev) => {
    if (prev && prev.dir !== ctx.dir) autoDraftDone = false
    if (pendingNewTarget !== undefined) return
    if (autoDraftDone || ctx.loading || ctx.loadError || ctx.selfAddr !== ctx.dir) return
    if (!ctx.type || isSchemaDrivenExt(ctx.type.node_ext ?? ctx.type.ext)) return
    if (ctx.itemCount > 0 || ctx.selected || ctx.deepLinked) return
    autoDraftDone = true
    startNew()
  },
  { immediate: true },
)

// ==================== 未保存离开拦截 ====================
// 详情渲染器（表单 / 文本）在编辑时上抛 dirty。切换节点/目录、钻入、新建这些
// 「离开当前详情」的动作先过一道闸：有改动就弹确认，避免静默丢弃编辑现场。
const detailDirty = ref(false)
const pendingLeave = ref(false)
let queuedLeave: (() => void) | null = null
let leaveResolver: ((allowed: boolean) => void) | null = null

function onDetailDirty(dirty: boolean) {
  detailDirty.value = dirty
  if (dirty) clearTestResult()
}

function resetLeave() {
  queuedLeave = null
  pendingLeave.value = false
  const resolve = leaveResolver
  leaveResolver = null
  resolve?.(false)
}

// 兜底：任何路径导致的详情节点切换（含保存后 select、自动草稿、后端收敛）都清掉
// 上一节点的 dirty，避免把旧编辑态带进新详情、在下次离开时误弹确认。
watch(
  () => selectedNode.value?.path ?? selectedNode.value?.id,
  () => {
    detailDirty.value = false
    resetLeave()
  },
)

/** 离开当前详情前先过闸；无改动直接执行，有改动则暂存动作并弹确认 */
function guardLeave(action: () => void) {
  if (!detailDirty.value) {
    action()
    return
  }
  queuedLeave = action
  pendingLeave.value = true
}

/** 路由宿主用它拦截浏览器 Back/Forward；控件只提供确认，不关心路由实现。 */
function confirmRouteLeave(): Promise<boolean> {
  if (!detailDirty.value) return Promise.resolve(true)
  pendingLeave.value = true
  return new Promise((resolve) => {
    leaveResolver = resolve
  })
}

function confirmLeave() {
  pendingLeave.value = false
  detailDirty.value = false
  const action = queuedLeave
  queuedLeave = null
  const resolve = leaveResolver
  leaveResolver = null
  action?.()
  resolve?.(true)
}

function cancelLeave() {
  resetLeave()
}

/** 用户点「新建」：也是离开当前详情，过闸；自动备草稿不经过这里 */
function startNewFromButton() {
  if (loading.value || saving.value || actionBusy.value) return
  guardLeave(() => startNew())
}

/**
 * 全局「新建」请求（Ctrl+N / 命令面板）。
 *
 * 那两个入口只 push 到类别目录；当目录里已经选中一项时，路由原地不动。
 * 已有条目的目录不再自动铺草稿，因此这里让当前工作台直接执行与按钮同一条
 * 路径（含未保存离开闸）。请求带地址时只认自己的目录，避免跳转途中别页抢先
 * 铺出错误的草稿。
 */
function flushPendingNewResource() {
  if (pendingNewTarget === undefined) return
  const target = pendingNewTarget
  if (target && target !== cwd.value) return
  if (loading.value || loadError.value || cwdAddr.value !== cwd.value) return
  pendingNewTarget = undefined
  if (canCreate.value) startNewFromButton()
}

watch(
  [cwd, cwdAddr, canCreate, loading, loadError],
  flushPendingNewResource,
  { immediate: true },
)

function onNewResourceRequest(e: Event) {
  pendingNewTarget = (e as CustomEvent<string | undefined>).detail ?? null
  flushPendingNewResource()
}

function onBeforeUnload(event: BeforeUnloadEvent) {
  if (!detailDirty.value) return
  event.preventDefault()
  event.returnValue = true
}

const now = ref(Date.now())
let relativeClock: number | undefined

function syncRelativeClock() {
  now.value = Date.now()
}

onMounted(() => {
  window.addEventListener(NEW_RESOURCE_EVENT, onNewResourceRequest)
  window.addEventListener('beforeunload', onBeforeUnload)
  document.addEventListener('visibilitychange', syncRelativeClock)
  relativeClock = window.setInterval(syncRelativeClock, 60_000)
})
onBeforeUnmount(() => {
  window.removeEventListener(NEW_RESOURCE_EVENT, onNewResourceRequest)
  window.removeEventListener('beforeunload', onBeforeUnload)
  document.removeEventListener('visibilitychange', syncRelativeClock)
  if (relativeClock !== undefined) window.clearInterval(relativeClock)
  pendingNewTarget = undefined
  resetLeave()
})

// ==================== 钻入（emit，宿主决定呈现） ====================

/**
 * 点左栏某项。
 *
 * 两种形态，由**地址结构**决定（`useVdfs.atMountLevel`，前端不持有类别清单）：
 *
 * - **挂载层**（绑定地址是 `<根>` 的直接子节点，如 `<根>/session`）：左栏是
 *   **类别清单**（当前挂载高亮）——点一下 = 换到那个类别的地址页；
 * - 其余：左栏是绑定地址的子目录——点一下就地切换当前目录，不产生新页面。
 *
 * 差别只在「这一栏的东西住在谁的下面」：换类别必须换地址（类别不是当前目录的
 * 子目录，没有可重置的选中态），而子目录切换是同一个地址内的浏览。
 */
function onRailSelect(key: string) {
  if (atMountLevel.value) return guardLeave(() => emit('open', vdfsJoin(vdfsRoot(), key)))
  return guardLeave(() => void selectDir(key))
}

/**
 * 点中栏列表项：文件 → 选中（右栏详情）；目录 → 钻入其数据地址
 * （emit `open`，宿主通常 push 一个新地址页，同一控件承接）。
 */
function onItemClick(n: VdfsItem) {
  // 点当前已选项不算离开（只是重读同一详情），无需确认
  if (!isVdfsDir(n)) {
    if (selectedId.value === n.id) return void select(n)
    return guardLeave(() => void select(n))
  }
  guardLeave(() => emit('open', n.path))
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

// ==================== 键盘（绑定声明在这里，机制在 useHotkeys） ====================

const listRef = ref<HTMLElement | null>(null)
const focusedPath = ref('')

async function refresh(event?: Event) {
  const trigger = event?.currentTarget as HTMLElement | null
  const restoreFocus = Boolean(trigger && document.activeElement === trigger)
  try {
    await refreshVdfs()
  } finally {
    if (restoreFocus && trigger?.isConnected && document.activeElement === document.body) {
      await nextTick()
      trigger.focus({ preventScroll: true })
    }
  }
}

/**
 * 焦点是否落在这份列表里。
 *
 * ↑↓ 是**列表局部的键**，不是全局的：焦点在详情栏、在聊天输入框时把它抢走会直接
 * 破坏滚动/输入。判据因此是「焦点在本控件的列表容器内」，而不是「本页是 VDFS 页」。
 */
function listHasFocus(): boolean {
  const active = document.activeElement
  return Boolean(active && listRef.value?.contains(active))
}

function onListFocusIn(event: FocusEvent) {
  const target = (event.target as HTMLElement | null)?.closest<HTMLElement>('[data-path]')
  if (target?.dataset.path) focusedPath.value = target.dataset.path
}

const listPaths = computed(() => {
  const paths = new Set<string>()
  if (treeMode.value) {
    const visit = (nodes: VdfsTreeNodeData[]) => {
      for (const node of nodes) {
        paths.add(node.item.path)
        visit(node.children)
      }
    }
    visit(treeNodes.value)
  } else {
    for (const node of filteredItems.value) paths.add(node.path)
  }
  return paths
})

function hasListPath(path: string): boolean {
  return listPaths.value.has(path)
}

/** roving tabindex：整份长列表只保留一个 Tab 停靠点。 */
function isListTabStop(path: string, first: boolean): boolean {
  if (focusedPath.value && hasListPath(focusedPath.value)) return focusedPath.value === path
  if (selectedId.value && hasListPath(selectedId.value)) return selectedId.value === path
  return first
}

function isTreeTabStop(path: string, first: boolean): boolean {
  return isListTabStop(path, first)
}

watch(filteredItems, (nodes) => {
  if (treeMode.value || !focusedPath.value) return
  if (!nodes.some((node) => node.path === focusedPath.value)) focusedPath.value = ''
})

watch(treeNodes, () => {
  if (!treeMode.value || !focusedPath.value) return
  if (!listPaths.value.has(focusedPath.value)) focusedPath.value = ''
})

watch([cwd, treeMode], () => {
  focusedPath.value = ''
})

watch(selectedId, (id) => {
  if (id && !focusedPath.value) focusedPath.value = id
})

/**
 * 焦点在卡片之间移动（不改变选中）。
 *
 * 只挪焦点、不开详情，是 listbox 的常规分工：移动是浏览，`Enter` 才是决定。
 * 焦点越界时**停住**不绕回：长列表里绕回会把人甩到看不见的位置。
 */
function focusCardAt(index: number) {
  const selector = treeMode.value ? '.tree-node' : '.vdfs-card'
  const cards = Array.from(listRef.value?.querySelectorAll<HTMLElement>(selector) ?? [])
  if (cards.length === 0) return
  const target = cards[Math.min(cards.length - 1, Math.max(0, index))]
  focusedPath.value = target.dataset.path ?? ''
  target.focus()
}

function moveCardFocus(step: number) {
  const selector = treeMode.value ? '.tree-node' : '.vdfs-card'
  const cards = Array.from(listRef.value?.querySelectorAll<HTMLElement>(selector) ?? [])
  if (cards.length === 0) return
  const active = document.activeElement as HTMLElement | null
  const at = active ? cards.indexOf(active) : -1
  const next = at < 0 ? (step > 0 ? 0 : cards.length - 1) : Math.min(cards.length - 1, Math.max(0, at + step))
  focusCardAt(next)
}

useHotkeys('vdfs-workbench', [
  {
    id: 'rail-category',
    keys: 'mod+digit',
    group: '导航',
    label: '切到第 N 个类别（左栏顺序）',
    // 第 N 个类别不存在 ⇒ 这个数字还给浏览器（Ctrl+7 是切标签页）
    when: (e) => Number(e.key) >= 1 && Number(e.key) <= navItems.value.length,
    run: (e) => {
      const it = navItems.value[Number(e.key) - 1]
      if (it) onRailSelect(it.key)
    },
  },
  {
    id: 'list-down',
    keys: 'down',
    group: '资源列表',
    label: '下一条',
    when: listHasFocus,
    run: () => moveCardFocus(1),
  },
  {
    id: 'list-up',
    keys: 'up',
    group: '资源列表',
    label: '上一条',
    when: listHasFocus,
    run: () => moveCardFocus(-1),
  },
  {
    id: 'list-home',
    keys: 'home',
    group: '资源列表',
    label: '第一项',
    when: listHasFocus,
    run: () => focusCardAt(0),
  },
  {
    id: 'list-end',
    keys: 'end',
    group: '资源列表',
    label: '最后一项',
    when: listHasFocus,
    run: () => focusCardAt(Number.MAX_SAFE_INTEGER),
  },
])

/**
 * 详情「浏览内部」/ open-container：钻入容器目录的数据地址
 * （emit `open`）。会话节点本身是**叶子**（点击 = 聊天详情，语义不变）；
 * 其内部结构挂在**同名目录路径**之下（`<id>/subsession`、`<id>/workdir`）。
 */
function browseInto() {
  const n = selectedNode.value
  if (!n) return
  guardLeave(() => emit('open', n.path))
}

/**
 * 详情请求打开**另一个节点**（如会话变更面板里的一行）。
 *
 * 文件没有自己的地址页：三栏的右栏才是详情，所以「打开文件」= 打开它所在的
 * 父目录页 + 顺带选中它。拆分用地址代数（`vdfsParent` / `vdfsBase`），本控件
 * 仍然不认识路径里任何一段的含义。
 */
function onOpenNode(addr: string) {
  if (!addr) return
  guardLeave(() => emit('open', vdfsParent(addr), vdfsBase(addr)))
}

// ==================== 详情装配 ====================

/** 详情骨架的字段行数（表单详情一般是 4-6 行，取中间偏少，避免满载时反而更长） */
const DETAIL_SKELETON_ROWS = 5

/**
 * 右栏是否有可看的东西（供容器判「列表为空时能否收起中栏」）：有选中项且有渲染器，
 * 或详情正在加载。**「已选中但渲染器未登记」不算**——那种情况右栏最终落在占位符
 * 上，保留中栏反而给了用户换一个选项的余地。
 */
const hasDetail = computed(() => {
  if (loadingDetail.value || detailLoadError.value) return true
  return Boolean(selectedNode.value && rendererComp.value)
})

/**
 * 右栏那张详情是不是**新建草稿**——收栏判据的最后一环（见
 * `common/Workbench.vue` 的 `showList`）。
 *
 * 判据取 `isVdfsDraft`（**没有 path**），与资源类型无关：模型 / 技能 / 会话的
 * 草稿都该让中栏，而那些类型各有自己的 `ext`，按类型判必然漏。
 */
const isDraftSelected = computed(() => isVdfsDraft(selectedNode.value))

/** 当前渲染器组件（未登记 → null，视图回退占位） */
const rendererComp = computed(() => {
  if (!selectedNode.value) return null
  return getVdfsRenderer(resolveVdfsRenderer(selectedNode.value)) ?? null
})

/**
 * 渲染器数据：form → 字段值对象；文本类（含 message 的正文）→ 文本；
 * `schema` 类（插件市场）→ 节点自述里的候选；其余 → 不传。
 *
 * 市场是一张草稿页（没有落盘、没有正文），候选由后端随 `new_type.schema` 下发，
 * 因此取 `selectedNode.schema` 而不是 `nodeText`。外观面板的正文是
 * `ui_prefs` 下发的 JSON 配置文本，组件自行 parse。
 */
const rendererData = computed<unknown>(() => {
  const r = renderer.value
  if (isFormRenderer(r)) return formData.value
  if (r && rendererReadsNodeSchema(r)) return selectedNode.value?.schema
  if (r && rendererReadsNodeText(r)) return nodeText.value
  return null
})

const rendererRef = ref<{ markSaved?: () => void; currentTitle?: () => string } | null>(null)

function onSave(values: unknown, actionPayload?: Record<string, unknown>) {
  const r = renderer.value
  const sourceNode = selectedNode.value
  const sourceDraftSeq = draftSeq.value
  const done = isFormRenderer(r)
    ? actionPayload
      ? runAction('save', { ...actionPayload, fields: values })
      : saveFields((values ?? {}) as Record<string, unknown>)
    : r && rendererReadsNodeSchema(r)
      ? // 市场这类选择器页：候选在 `schema`，保存载荷是用户选中的项；写回当前
        // 目录由 `write` 按草稿规则完成（无需读正文，见 useVdfs.write）
      write(values as Record<string, unknown>)
      : r && rendererPayloadIsText(r)
        ? // 外观面板：载荷已是偏好配置的 JSON 文本，原样写回
        write(String(values ?? ''))
        : saveText()
  // 保存成功后 renderer 重新读入同一节点，身份门闩不会重跑预填 ⇒ 表单基线仍停在旧值，
  // dirty 会误留。显式把新值树立为基线，dirty 归零，避免保存后又弹「放弃修改」。
  void done.then((ok) => {
    if (!ok || !sourceNode) return
    const current = selectedNode.value
    const sameSelection = isVdfsDraft(sourceNode)
      ? current === sourceNode && draftSeq.value === sourceDraftSeq
      : current?.path === sourceNode.path
    if (!sameSelection) return
    rendererRef.value?.markSaved?.()
    detailDirty.value = false
  })
}

/** 删除确认：确认前不动数据（目录是递归删除，节点内容不可撤销） */
const pendingDelete = ref(false)
const deleteTarget = shallowRef<VdfsItem | null>(null)
const deleteTargetIsDir = computed(() => Boolean(deleteTarget.value && isVdfsDir(deleteTarget.value)))

const deleteConfirmMessage = computed(() => {
  const node = deleteTarget.value
  if (!node) return ''
  const name = node.title || node.name
  return deleteTargetIsDir.value
    ? `将递归删除目录「${name}」及其全部内容，此操作无法撤销。`
    : `将删除「${name}」，此操作无法撤销。`
})

function onDelete() {
  const node = selectedNode.value
  if (!node) return
  const currentTitle = rendererRef.value?.currentTitle?.()
  deleteTarget.value = { ...node, title: currentTitle || node.title }
  pendingDelete.value = true
}

function confirmDelete() {
  pendingDelete.value = false
  const target = deleteTarget.value
  deleteTarget.value = null
  if (target) void removeSelected(target)
}

function cancelDelete() {
  pendingDelete.value = false
  deleteTarget.value = null
}

/** 转发动作标识、声明载荷与可选文件；文件编码归 useVdfs。 */
function onAction(id: string, payload?: unknown, file?: File) {
  if (file) void runPackAction(id, file, payload)
  else if (payload === undefined) void runAction(id)
  else void runAction(id, payload)
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

defineExpose({ confirmRouteLeave })

// ==================== 列表项展示 ====================
//
// 卡片上那几个可见元素取什么值（状态点 / 状态文案 / 徽标 / 标签 / 图标），
// 一律查 `registry/vdfsCards` —— 那五条映射每条都带着「不许回退」的约定，
// 规则与实现放在同一处才钉得住，也才测得动。本控件不再持有它们的任何一份副本。
//
// 注：**状态点画不画**由节点自己声明（`status` 为空串 = 该资源没有运行态），
// 判据在模板里就是 `:show-status="Boolean(n.status)"`，不进注册表。
</script>

<style scoped>
/* ============== 新建（本栏唯一主操作） ============== */
/* 填充主题色：与同排的次要图标按钮（透明底）拉开层级。
   尺寸沿用 `.icon-btn` 的 2rem 方形，故两者基线对齐、不撑高 header。 */
.new-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 2rem;
  height: 2rem;
  border: none;
  border-radius: var(--radius-md);
  background: var(--accent);
  color: var(--text-on-accent);
  cursor: pointer;
  transition: background-color var(--motion-fast) var(--motion-ease);
}
.new-btn:hover:not(:disabled) {
  background: var(--accent-hover);
}
.new-btn:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

.refresh-btn.is-loading svg {
  animation: refresh-spin 0.9s linear infinite;
}
.refresh-btn.is-loading:disabled {
  opacity: 0.75;
  cursor: progress;
}
@keyframes refresh-spin {
  to { transform: rotate(360deg); }
}

/* ============== 筛选 ============== */
.vdfs-filter {
  position: relative;
  display: flex;
  align-items: center;
  padding: 0.4rem 0.5rem 0.2rem;
  flex-shrink: 0;
}

.vdfs-filter-input {
  box-sizing: border-box;
  width: 100%;
  min-height: 2rem;
  padding: 0.3rem 1.6rem 0.3rem 0.6rem;
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: var(--surface-sunken);
  color: var(--text-primary);
  font-size: 0.78rem;
  font-family: inherit;
}
.vdfs-filter-input:focus {
  outline: none;
  border-color: var(--accent);
}
.vdfs-filter-input::placeholder {
  color: var(--text-muted);
}
/* 去掉 type=search 的原生清除叉：自绘的 × 才有焦点态与主题色 */
.vdfs-filter-input::-webkit-search-cancel-button {
  display: none;
}

.filter-clear {
  position: absolute;
  right: 0.65rem;
  width: 1.5rem;
  height: 1.5rem;
  display: flex;
  align-items: center;
  justify-content: center;
  border: none;
  border-radius: var(--radius-full);
  background: transparent;
  color: var(--text-muted);
  font-size: 0.9rem;
  line-height: 1;
  cursor: pointer;
}
.filter-clear:hover {
  background: var(--surface-hover);
  color: var(--text-primary);
}

/* 空态里的动作：恢复路径（清除筛选）与引导入口（新建）共用一个原子。
   不能只有一个说明而没有出口。 */
.empty-action {
  min-height: 2rem;
  margin-top: 0.5rem;
  padding: 0.3rem 0.85rem;
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text-secondary);
  font-size: 0.78rem;
  cursor: pointer;
}
.empty-action:hover:not(:disabled) {
  background: var(--surface-hover);
  color: var(--text-primary);
}

/* 主操作（新建第一个 X）：与本栏右上角那个「新建」同色，
   两处说的是同一件事，不应有两种视觉层级 */
.empty-action.primary {
  border-color: var(--accent);
  background: var(--accent);
  color: var(--text-on-accent);
}
.empty-action.primary:hover:not(:disabled) {
  background: var(--accent-hover);
  border-color: var(--accent-hover);
  color: var(--text-on-accent);
}
.empty-action:disabled {
  opacity: 0.5;
  cursor: not-allowed;
}

/* ============== 列表 ============== */
.vdfs-list-region {
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
}
.vdfs-list {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 0.25rem 0;
}

.vdfs-action-result {
  flex-shrink: 0;
  display: flex;
  align-items: flex-start;
  gap: 0.5rem;
  margin: 0.5rem 1rem 0;
  padding: 0.55rem 0.75rem;
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  font-size: 0.78rem;
  line-height: var(--line-height-normal);
}
.vdfs-action-result p {
  margin: 0;
  overflow-wrap: anywhere;
}
.vdfs-action-result.is-success {
  border-color: var(--accent);
  color: var(--accent);
  background: var(--accent-subtle-bg);
}

/* 详情读取失败与有界列表的续页入口 */
.vdfs-detail-error {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.65rem;
  padding: 1rem;
  color: var(--danger-fg);
  font-size: 0.78rem;
  text-align: center;
}
.vdfs-detail-error p {
  margin: 0;
}
.list-more {
  display: flex;
  flex-direction: column;
  align-items: center;
  flex-shrink: 0;
  padding: 0.75rem 0 1rem;
}
.more-btn {
  min-height: 2rem;
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
.list-more-error {
  margin: 0 0 0.35rem;
  font-size: 0.72rem;
  color: var(--danger-fg);
  text-align: center;
}

.more-hint {
  font-size: 0.8125rem;
  color: var(--text-muted);
}

/* 空态里的失败说明（列表加载失败 ≠ 目录为空，见模板） */
.prompt-error {
  margin: 0;
  font-size: 0.78rem;
  color: var(--danger-fg);
}

/* ============== 占位 ============== */
.placeholder-notice {
  max-width: 22rem;
  text-align: center;
  color: var(--color-tag-warn-fg);
  line-height: 1.5;
}

.vdfs-placeholder {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  color: var(--text-muted);
  font-size: 0.85rem;
}

/* 详情骨架：内边距与 DetailShell 默认形态对齐（0.75rem 1rem），
   故骨架与真实详情的内容起点一致，加载完成时不横向跳位 */
.vdfs-detail-skeleton {
  display: flex;
  flex-direction: column;
  gap: 0.85rem;
  padding: 0.75rem 1rem;
}

@media (pointer: coarse) {
  .new-btn {
    width: 2.75rem;
    height: 2.75rem;
  }

  .vdfs-filter-input {
    min-height: 2.75rem;
    padding-right: 2.4rem;
    font-size: 1rem;
  }

  .filter-clear {
    right: 0.35rem;
    width: 2.75rem;
    height: 2.75rem;
  }

  .empty-action,
  .more-btn {
    min-width: 2.75rem;
    min-height: 2.75rem;
  }
}
</style>
