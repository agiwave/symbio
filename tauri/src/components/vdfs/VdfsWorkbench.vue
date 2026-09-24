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
    :has-list-content="items.length > 0"
    :loading="loading"
    @rail-select="selectDir"
  >
    <template #rail-header><slot name="rail-header" /></template>
    <template #rail-footer><slot name="rail-footer" /></template>

    <template #header-actions>
      <!-- 新建（节点声明了可接受的新建类型时可见；类型由后端下发，前端不硬编码）。
           点一下**直接进入该类型的详情页**（草稿态）——没有「选方式」这一步：
           整包导入是详情页上的一条动作，不是第二种新建入口（见 ADR-029）。 -->
      <button
        v-if="canCreate"
        class="icon-btn"
        :title="`新建 ${creatableType?.title ?? ''}`"
        :disabled="loading || saving"
        @click="startNew()"
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
        <!-- 徽标只给目录（子项数）。⚠️ 文件不给徽标：`ext` 是**渲染器键**
             （`form` / `session` 之类），属机制细节，不该出现在给用户看的列表里。 -->
        <VdfsCard
          v-for="n in items"
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

      <div v-else-if="loadingDetail" class="vdfs-placeholder">加载中…</div>
      <div v-else class="vdfs-placeholder">
        <p>← 选择一个资源查看/编辑</p>
      </div>
    </template>
  </Workbench>
</template>

<script setup lang="ts">
import { computed } from 'vue'
import Workbench from '@/components/common/Workbench.vue'
import VdfsCard from '@/components/common/VdfsCard.vue'
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
  isVdfsDir,
  vdfsJoin,
  type VdfsNode,
} from '@/schemas/vdfs'

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
 * 其内部结构挂在**同名目录路径**之下（`<id>/subsession`、`<id>/workdir`）。
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
</style>
