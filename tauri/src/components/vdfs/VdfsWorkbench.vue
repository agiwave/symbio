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
    `<根>`、会话/智能体内部页绑 `<根>/session/<id>`，对控件毫无区别。
-->
<template>
  <Workbench
    :rail-items="navItems"
    :title="title"
    :has-list-content="filteredItems.length > 0"
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
           整包导入是详情页上的一条动作，不是第二种新建入口（见 ADR-029）。

           视觉上它是本栏**唯一的主操作**（填充主题色），与「刷新」这类次要图标
           按钮明确分开：此前两者同为透明 icon-btn，用户要读 tooltip 才知道哪个
           能建东西；而「没内容时该做什么」正是最需要一眼看出的时刻。 -->
      <button
        v-if="canCreate"
        class="new-btn"
        :title="`新建 ${creatableType?.title ?? ''}`"
        :aria-label="`新建 ${creatableType?.title ?? ''}`"
        :disabled="loading || saving"
        @click="startNew()"
      >
        <svg viewBox="0 0 24 24" width="15" height="15" fill="none" stroke="currentColor" stroke-width="2.25" stroke-linecap="round">
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
      <!-- 筛选框：只在**当前目录有内容**时出现。
           空目录给一个搜索框是噪音——没有东西可筛，它只会让用户以为"可以搜库"。
           筛选是纯客户端投影（见 useVdfs 的说明），不发请求、不动分页游标。 -->
      <div v-if="items.length > 0" class="vdfs-filter">
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

      <div class="vdfs-list" role="listbox" aria-label="资源列表" @scroll.passive="onListScroll">
        <!-- 徽标只给目录（子项数）。⚠️ 文件不给徽标：`ext` 是**渲染器键**
             （`form` / `session` 之类），属机制细节，不该出现在给用户看的列表里。 -->
        <VdfsCard
          v-for="n in filteredItems"
          :key="n.path"
          :title="n.title || n.name"
          :subtitle="n.description"
          :status="cardStatusOf(n)"
          :status-title="cardStatusTextOf(n)"
          :show-status="Boolean(n.status)"
          :badge="cardBadgeOf(n)"
          badge-kind="primary"
          :tags="cardTagsOf(n)"
          :icon="cardIconOf(n)"
          :is-active="selectedId === n.path"
          @click="onItemClick(n)"
        />
        <!-- 有界列表的收尾：滚到底自动续页，也留一个显式入口。
             ⚠️ 筛选态下**必须保留**：命中可能落在更早的页里，去掉这个入口
             就会把「还没找到」误报成「不存在」。 -->
        <div v-if="hasMore && filteredItems.length > 0" class="list-more">
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
          :disabled="loading || saving"
          @click="startNew()"
        >
          新建第一个{{ creatableType?.title ?? '条目' }}
        </button>
        <p v-else class="hint">该目录由系统管理</p>
      </template>
    </template>

    <template #detail>
      <!-- 详情：渲染器由节点 ext 决定（唯一分发点）。**草稿（新建态）也走这里**
           ——同一个 ext 用同一个渲染器，因此「点新建」与「选中一项」在交互上
           没有第二种形态（key 对草稿另取，保证连续新建时重挂载）。

           机制动作（删除）由**本控件**算一次后注入所有渲染器：它们是
           「页面对任何已落盘可写节点都能做的默认动作」，不该由每个渲染器各算一遍
           （见 useVdfs.mechanismActions）。渲染器只声明**它自己特有**的动作。 -->
      <component
        :is="rendererComp"
        v-if="selectedNode && rendererComp"
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
      />

      <!-- 详情加载中：骨架按"标题 + 若干字段行"的形状占位，与表单详情同构，
           数据到达时是填充而不是重排 -->
      <div v-else-if="loadingDetail" class="vdfs-detail-skeleton" aria-busy="true">
        <SkeletonBlock :width="'38%'" :height="0.95" />
        <SkeletonBlock v-for="i in DETAIL_SKELETON_ROWS" :key="i" :lines="1" :height="0.85" />
      </div>
      <div v-else class="vdfs-placeholder">
        <p>← 选择一个资源查看/编辑</p>
      </div>
    </template>
  </Workbench>
</template>

<script setup lang="ts">
import { computed, watch } from 'vue'
import Workbench from '@/components/common/Workbench.vue'
import VdfsCard from '@/components/common/VdfsCard.vue'
import SkeletonBlock from '@/components/common/SkeletonBlock.vue'
import { useVdfs } from '@/composables/useVdfs'
import { getVdfsRenderer, isTextualRenderer, resolveVdfsRenderer } from '@/registry/vdfsTypes'
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
  VDFS_EXT_SESSION,
  isVdfsDir,
  isVdfsDraft,
  vdfsJoin,
  type VdfsItem,
} from '@/schemas/vdfs'
import { vdfsRoot } from '@/schemas/vdfsRoot'
import { useSessionsStore } from '@/stores/sessions'

const props = defineProps<{
  /** 绑定的 vdfs 数据地址（如 `<根>` 或 `<根>/session/<id>`）；变化 = 整体重载 */
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
  cwdNode,
  title,
  items,
  filteredItems,
  filter,
  activeFilter,
  filterEmpty,
  filterTruncated,
  loading,
  loadError,
  hasMore,
  loadingMore,
  refresh,
  loadMore,
  select,
  atMountLevel,
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
  mechanismActions,
  mechanismBusyId,
  saveFields,
  saveText,
  runAction,
  runPackAction,
  removeSelected,
  startNew,
  creatableType,
  canCreate,
  draftSeq,
} = useVdfs({ addr: computed(() => props.addr) })

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
const sessionsStore = useSessionsStore()
watch(
  () => (cwdNode.value?.new_type?.ext === VDFS_EXT_SESSION ? cwd.value : ''),
  (dir) => {
    if (dir) sessionsStore.setSessionSpace(dir)
  },
  { immediate: true },
)

// ==================== 进入「可新建」的目录时自动备好一张草稿 ====================
//
// 空目录里唯一能做的事就是新建，而新建详情已经铺在右栏 ⇒ 中栏整栏收起（判据在
// `common/Workbench.vue` 的 `showList`）。这里只负责**把那张草稿备好**。
//
// ⚠️ 监听源是**刚读回来的目录自述**（`cwdNode`），**不是**目录地址（`cwd`）。换目录
// 时 `cwd` 立刻变、`cwdNode` 要等 `refresh()` 回来才更新，中间那一拍 `creatableType`
// 还是**上一栏**的——拿它开草稿，就把上一类的详情页开到了这一栏上（用户报的「模型
// 页显示智能体详情、会话页显示模型详情」，一步滞后）。草稿的类型只能来自当前目录
// 自己的自述。
//
// ⚠️ 判据**不得出现类型名**：曾写成 `creatableType.ext === 'session'`，结果只有会话
// 页有草稿、其余类型永远收不了栏。类型只由目录自述 `new_type` 决定。
//
// 两条边界：① 用户还没选中任何东西（不抢深链进来的选择）；② 每个目录只开一次
// （否则清空选中 / 删掉刚选中的项之后草稿会"复活"）。
let autoDraftDone = false
watch(
  () => cwdNode.value,
  (node) => {
    if (autoDraftDone || !node?.new_type) return
    if (selectedNode.value) return
    autoDraftDone = true
    startNew()
  },
  { immediate: true },
)

// 换目录时重置「已自动开过」的标记：新目录是新的上下文，理应各自开一份。
watch(
  () => cwd.value,
  () => {
    autoDraftDone = false
  },
)

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
  if (atMountLevel.value) return void emit('open', vdfsJoin(vdfsRoot(), key))
  return void selectDir(key)
}

/**
 * 点中栏列表项：文件 → 选中（右栏详情）；目录 → 钻入其数据地址
 * （emit `open`，宿主通常 push 一个新地址页，同一控件承接）。
 */
function onItemClick(n: VdfsItem) {
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
 * 其内部结构挂在**同名目录路径**之下（`<id>/subsession`、`<id>/workdir`）。
 */
function browseInto() {
  const n = selectedNode.value
  if (!n) return
  emit('open', n.path)
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
  if (loadingDetail.value) return true
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

/** 渲染器数据：form → 字段值对象；文本类（含 message 的正文）→ 文本；其余 → 不传 */
const rendererData = computed<unknown>(() => {
  const r = renderer.value
  if (r === 'form') return formData.value
  if (isTextualRenderer(r)) return nodeText.value
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
 * 节点动作（如「测试连接」「导出」「导入整包」）：标识由详情定义声明、由
 * provider 解释，控件只负责转发并呈现结果。
 *
 * `file` 只在动作声明了 `pack` 时带上（渲染器取到本地文件后上抛）——载荷怎么
 * 编码是机制层的事，控件把它交给 `runPackAction`。于是「导入」（取文件）与
 * 「导出」（给文件）在控件这一层恰是一对逆操作，两边都不认识对方的动作名。
 */
function onAction(id: string, file?: File) {
  if (file) void runPackAction(id, file)
  else void runAction(id)
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
   尺寸沿用 `.icon-btn` 的 1.625rem 方形，故两者基线对齐、不撑高 header。 */
.new-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 1.625rem;
  height: 1.625rem;
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

/* ============== 筛选 ============== */
.vdfs-filter {
  position: relative;
  display: flex;
  align-items: center;
  padding: 0.4rem 0.5rem 0.2rem;
  flex-shrink: 0;
}

.vdfs-filter-input {
  width: 100%;
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
  right: 0.85rem;
  width: 1.1rem;
  height: 1.1rem;
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

/* 空态里的失败说明（列表加载失败 ≠ 目录为空，见模板） */
.prompt-error {
  margin: 0;
  font-size: 0.78rem;
  color: var(--danger-fg);
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

/* 详情骨架：内边距与 DetailShell 默认形态对齐（0.75rem 1rem），
   故骨架与真实详情的内容起点一致，加载完成时不横向跳位 */
.vdfs-detail-skeleton {
  display: flex;
  flex-direction: column;
  gap: 0.85rem;
  padding: 0.75rem 1rem;
}
</style>
