<!--
  Workbench — 「侧边栏 + 列表 + 详情」三栏工作台容器（全项目统一命名/唯一实现）

  页面容器，只做插槽装配，自身无数据逻辑。三栏自上而下相接：

  - **侧边栏**（窄条图标导航，可选——不传 railItems 即无侧边栏的两栏形态）：
    类别集合一律由后端下发（VDFS 目录节点），前端只做 UI 映射；`rail-header` /
    `rail-footer` 由宿主注入（返回键 / logo / 系统目录入口等宿主件）；
  - **中栏**：panel-header（标题 + header-actions）+ list + 空态 / 加载态；
  - **右栏**：detail。

  插槽只有**消费方真的会用**的那些：`rail-header` / `rail-footer` / `header-actions`
  / `list` / `empty` / `detail`。此前还转发过 `content` / `meta` 两个无人使用的插槽，
  以及 `back` / `backTitle` 两个恒为缺省的 prop（连同它们的默认返回键），已删除——
  不可达的分支不是灵活性，是维护税。

  侧栏的 `.nav-btn` / `.nav-count` / `.nav-label` 是**全局**样式
  （styles/controls.css）：`rail-header` 插槽内容属宿主作用域，只有全局类才能命中。

  VDFS 的三栏页面 = VdfsWorkbench 控件（components/vdfs）+ useVdfs 数据逻辑
  （绑定数据地址，自取左栏/中栏/详情）。规范：docs/design/vdfs.md。

  **列表为空且右栏正铺着新建详情**时中栏整栏收起（判据见 `showList`）。
-->
<template>
  <div class="workbench">
    <!-- 第一栏：侧边栏 -->
    <nav v-if="railItems" class="side-nav">
      <slot name="rail-header" />

      <div class="nav-items">
        <button
          v-for="it in railItems"
          :key="it.key"
          class="nav-btn"
          :class="{ active: it.active }"
          :aria-label="it.label"
          :title="it.description || it.label"
          @click="$emit('rail-select', it.key)"
        >
          <component :is="it.icon" v-if="it.icon" />
          <!-- 未登记图标的**目录**兜底：必须是文件夹，不能是文件——
               侧栏项一律是目录（`<根>` 的挂载点），用文件图标是语义错配。
               尺寸同样交给 CSS（`.nav-btn svg`）。 -->
          <svg
            v-else
            viewBox="0 0 24 24"
            fill="none"
            stroke="currentColor"
            stroke-width="1.75"
            stroke-linecap="round"
            stroke-linejoin="round"
          >
            <path d="M22 19a2 2 0 0 1-2 2H4a2 2 0 0 1-2-2V5a2 2 0 0 1 2-2h5l2 3h9a2 2 0 0 1 2 2z" />
          </svg>
          <span v-if="it.count" class="nav-count">{{ it.count }}</span>
          <span class="nav-label">{{ it.label }}</span>
        </button>
      </div>

      <slot name="rail-footer" />
    </nav>

    <!-- 第二栏：列表 -->
    <!-- 列表**为空且非加载/非空态**时整栏收起：此时中栏能给的只有空白，
         留着它只是把详情挤窄。判据取"真的没有可选项且有详情可看"——见
         `showList`。空态（`empty` 插槽）与加载骨架**不收起**：前者是内容
         （「此目录为空 / 没有匹配」必须说出来），后者是过渡，收起都会闪。 -->
    <aside v-if="showList" class="workbench-list" :style="{ width: `${listWidth}px` }">
      <header class="panel-header">
        <h3 class="panel-title">{{ title }}</h3>
        <div class="header-actions">
          <slot name="header-actions" />
        </div>
      </header>

      <slot name="list" />

      <div v-if="!hasListContent && $slots.empty" class="empty-state">
        <slot name="empty" />
      </div>
      <!-- 加载态用骨架卡而非纯文字：形状与列表内容一致，数据到达是"填充"
           而不是"重排"（纯文字态与内容态高度不等，一到就跳） -->
      <div v-else-if="!hasListContent && loading" class="list-skeleton" aria-busy="true">
        <VdfsCardSkeleton v-for="i in SKELETON_COUNT" :key="i" :title-width="skeletonTitleWidth(i)" />
      </div>
    </aside>

    <!-- 第三栏：详情 -->
    <section class="workbench-detail">
      <slot name="detail" />
    </section>
  </div>
</template>

<script setup lang="ts">
import type { Component } from 'vue'
import { computed } from 'vue'
import VdfsCardSkeleton from './VdfsCardSkeleton.vue'

/**
 * 骨架卡的数量与宽度变化。
 *
 * 5 张足以铺满中栏一屏（`min-width` 13.75rem 下一张卡约 3.5rem 高），
 * 再多只是徒增动画开销。宽度按固定序列错开——等宽的骨架排在一起像表格，
 * 错开才读得出「这是一列内容」。
 */
const SKELETON_COUNT = 5
const SKELETON_WIDTHS = ['58%', '72%', '46%', '66%', '54%']
const skeletonTitleWidth = (i: number): string => SKELETON_WIDTHS[(i - 1) % SKELETON_WIDTHS.length]

/**
 * 侧栏导航项（宿主与容器之间的纯 UI 契约）。
 *
 * 它是**呈现投影**而非协议形状：由数据层（如 useVdfs 的 navItems）产出，
 * 容器只负责画。`icon` 是已解析的组件（图标映射属 registry 的纯 UI 映射）。
 */
export interface WorkbenchRailItem {
  /** 类别键（provider kind / 容器子类别 kind / VDFS 挂载名） */
  key: string
  label: string
  /** 类别图标（registry/vdfsIcons 注册表映射；缺省回退通用文件图标） */
  icon?: Component | null
  /** 语义说明（作 tooltip；VDFS 导航由后端下发 description） */
  description?: string
  /** 计数角标（0/undefined 不显示） */
  count?: number
  active?: boolean
}

const props = withDefaults(
  defineProps<{
    /** 侧边栏类别项（后端下发的 VDFS 挂载点清单；不传 = 本页无侧边栏） */
    railItems?: WorkbenchRailItem[]
    /** 中栏标题（列表加载后即为当前目录的自述） */
    title?: string
    /** 中栏宽度（px） */
    listWidth?: number
    /**
     * 中栏列表是否已有内容（控制空态/加载骨架的显隐，并作为「有无可选项」的事实）。
     */
    hasListContent?: boolean
    /** 中栏加载中 */
    loading?: boolean
    /**
     * 右栏是否有可看内容（已选中一项 / 详情加载中）。
     *
     * 缺省 `false`（不收栏）——激进的那一支必须由宿主显式选择：只有宿主知道
     * 右栏渲染的是真内容还是「← 选择一个资源」的占位符。
     */
    hasDetail?: boolean
    /**
     * 选中的是否为**草稿（新建态）**。
     *
     * 与 `canCreate` 一起构成收栏判据：草稿已占右栏时，中栏那句「暂无子目录／
     * 点击右上角新建」是重复信息，收起它换来更宽的详情。非草稿（用户在看已有
     * 条目）或不可新建时保留中栏——那两处的空态解释必须留着。
     */
    hasDraft?: boolean
    /** 当前目录可新建（空态文案与收栏判据共用） */
    canCreate?: boolean
  }>(),
  {
    railItems: undefined,
    title: '',
    listWidth: 260,
    hasListContent: false,
    loading: false,
    hasDetail: false,
    hasDraft: false,
    canCreate: false,
  }
)

/**
 * 中栏是否该显示。**只有一种情形会收起**：列表没有可选项，且右栏正显示一张
 * **新建详情**（`hasDraft && canCreate`）——空目录里能做的只有新建，而新建详情
 * 已经铺在右栏，中栏那句「暂无子目录／点击右上角新建」只是把详情挤窄。
 *
 * 其余一切情形都保留中栏：有可选项（中栏是主导航）、加载中（收起会把"正在取"
 * 变成"什么都没有"，数据到了整栏弹回）、右栏空（收起 = 整屏空白）、以及
 * **非草稿的空目录**（用户在看已有条目或在浏览目录，那句「此目录为空」
 * 「该目录由系统管理」是他此刻唯一的解释）。
 *
 * ⚠️ 别改回「宿主没提供 `empty` 插槽才收起」：`VdfsWorkbench` **无条件**声明该
 * 插槽（空态文案是它的正常内容），那条判据在本项目**恒为假**、收起永不发生。
 * 该不该让位取决于右栏是不是那张新建详情，与宿主声明了哪些插槽无关。
 */
const showList = computed(
  () =>
    props.hasListContent ||
    props.loading ||
    !props.hasDetail ||
    !(props.hasDraft && props.canCreate)
)

defineEmits<{
  (e: 'rail-select', key: string): void
}>()
</script>

<style scoped>
.workbench {
  display: flex;
  width: 100%;
  height: 100%;
  min-height: 0;
  overflow: hidden;
  background: var(--surface-page);
}

/* ============== 第一栏：侧边栏 ============== */
.side-nav {
  width: var(--sidebar-width);
  background: var(--surface-panel);
  border-right: 1px solid var(--border-default);
  display: flex;
  flex-direction: column;
  flex-shrink: 0;
  z-index: 10;
}

.nav-items {
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 0.25rem;
  flex: 1;
  padding: 0.5rem 0.375rem;
  overflow-y: auto;
}

/* ============== 第二栏：列表 ============== */
.workbench-list {
  flex: 0 0 auto;
  min-width: 13.75rem;
  max-width: 22.5rem;
  display: flex;
  flex-direction: column;
  background: var(--surface-panel);
  border-right: 1px solid var(--border-default);
  overflow: hidden;
}

.panel-header {
  display: flex;
  align-items: center;
  gap: 0.25rem;
  padding: 0.5rem 0.75rem;
  border-bottom: 1px solid var(--border-default);
  flex-shrink: 0;
}

/* 撑开中间，把右侧动作推到边上；min-width:0 + 省略号防长标题溢出 */
.panel-title {
  flex: 1 1 auto;
  min-width: 0;
  font-size: 0.85rem;
  font-weight: 600;
  color: var(--text-secondary);
  margin: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.header-actions {
  display: flex;
  gap: 0.25rem;
  /* 标题可压缩（flex 1 + 省略号），动作行不参与压缩 */
  flex-shrink: 0;
}

.empty-state {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  color: var(--text-muted);
  font-size: 0.85rem;
  gap: 0.3rem;
  padding: 1rem;
  text-align: center;
}

.empty-state :deep(.hint) {
  font-size: 0.75rem;
  opacity: 0.7;
}

/* 加载骨架：与 .vdfs-list 同内边距，卡片横坐标与真实列表一致 */
.list-skeleton {
  flex: 1;
  overflow: hidden;
  padding: 0.25rem 0;
}

/* ============== 第三栏：详情 ============== */
/* 详情区用**毛玻璃**而非纯色面板：它是三栏里最"深"的一层，背后是页面底与
   已经被切换掉的列表内容，透出一点背景能让层级关系自明（而不是三块等重的白板）。

   ⚠️ 两个必要条件，缺一毛玻璃就等于白设：
   1. **父层必须有非不透明背景**（`.workbench` 的 `--surface-page` 已满足）；
      若整条链路都是不透明色，`backdrop-filter` 无从"模糊"任何东西。
   2. **`background` 必须带 alpha**（`--surface-glass` 是 rgba）——不透明底会把
      模糊结果整个盖住，看起来和纯色一模一样，这类"写了没用"最难发现。

   不支持 `backdrop-filter` 的环境退回不透明面板色（`@supports` 兜底）：
   那种环境上半透明只会显得脏。 */
.workbench-detail {
  flex: 1 1 auto;
  min-width: 0;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: var(--surface-panel);
}

@supports (backdrop-filter: blur(1px)) or (-webkit-backdrop-filter: blur(1px)) {
  .workbench-detail {
    background: var(--surface-glass);
    -webkit-backdrop-filter: blur(var(--glass-blur));
    backdrop-filter: blur(var(--glass-blur));
  }
}
</style>
