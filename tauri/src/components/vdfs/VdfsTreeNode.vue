<!--
  VdfsTreeNode — 中栏「目录树」的递归节点（列表之外的第二种呈现形态）

  数据源是 useVdfs.treeNodes：后端 `vdfs/tree` 的扁平回执按 path 组装出的树。
  本组件只做两件事：

  - **目录**：点击就地展开 / 折叠（**默认收拢**，选中项所在那几层自动展开），
    不再 push 新地址页——中栏就是树，钻入 = 展开。这与扁平列表「点目录换地址页」
    是同一资源的不同呈现约定。
  - **文件**：点击 emit `activate`，由父组件走与列表项完全相同的选中逻辑
    （脱页确认 + useVdfs.select → 右栏详情）。

  展示映射（图标 / 徽标 / 状态 / 标签）复用 `registry/vdfsCards`，与 VdfsCard
  同源——同一个节点在列表与树里长得一样，只有层级缩进与展开箭头是树独有的。
  递归引用自己在渲染函数内（ESM 环在模块求值期不触碰），与 MessageChildren 同手法。
-->
<template>
  <div>
    <div
      class="tree-row"
      :class="{ active: selectedId === node.item.path }"
      role="button"
      tabindex="0"
      :aria-expanded="isDir ? expanded : undefined"
      @click="onActivate"
      @keydown.enter.prevent="onActivate"
      @keydown.space.prevent="onActivate"
    >
      <!-- 目录用展开箭头（纯 UI，不参与能力判定）；文件用占位保持左缘对齐 -->
      <span class="tree-toggle" :class="{ leaf: !isDir }">
        <svg v-if="isDir" viewBox="0 0 16 16" width="12" height="12" :class="{ open: expanded }">
          <path d="M6 4 L10 8 L6 12" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round" />
        </svg>
      </span>
      <component :is="icon" v-if="icon" class="tree-icon" />
      <span class="tree-title" :title="node.item.title || node.item.name">
        {{ node.item.title || node.item.name }}
      </span>
      <span v-if="badge" class="tree-badge">{{ badge }}</span>
    </div>

    <!-- 子层：仅目录展开时渲染；缩进由 CSS 变量逐层累加，左侧引导竖线表达层级 -->
    <div v-if="isDir && expanded && node.children.length" class="tree-children">
      <VdfsTreeNode
        v-for="child in node.children"
        :key="child.item.path"
        :node="child"
        :selected-id="selectedId"
        @activate="emit('activate', $event)"
      />
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, ref } from 'vue'
import {
  cardBadgeOf,
  cardIconOf,
} from '@/registry/vdfsCards'
import { vdfsAccessOf, type VdfsItem, type VdfsTreeNode } from '@/schemas/vdfs'

const props = defineProps<{
  node: VdfsTreeNode
  /** 右栏当前选中的条目地址（高亮用） */
  selectedId: string | null
}>()

const emit = defineEmits<{
  /** 请求激活条目：文件 → 右栏选中；目录 → 父组件决定（此处仅展开） */
  activate: [item: VdfsItem]
}>()

const isDir = computed(() => vdfsAccessOf(props.node.item).list)
const icon = computed(() => cardIconOf(props.node.item))
const badge = computed(() => cardBadgeOf(props.node.item))

/**
 * 展开态：**默认收拢**，点开才展开。
 *
 * 工作目录动辄成百上千项，全展开的第一屏几乎全是深层文件，顶层结构反而看不见——
 * 而树的用处正是「先看结构，再往下走」。唯一的例外是**当前选中项就在这棵子树里**
 * （重新挂载 / 换目录回来时的到达态）：那时把沿途展开，否则选中的文件藏在收起的
 * 目录里，用户会以为选择丢了。
 */
const expanded = ref(hasSelectedWithin(props.node, props.selectedId ?? ''))

/** 选中项是否在**子孙**里（自身不算——自己是文件时该由父层判断） */
function hasSelectedWithin(n: VdfsTreeNode, id: string): boolean {
  if (!id) return false
  return n.children.some((c) => c.item.path === id || hasSelectedWithin(c, id))
}

function onActivate() {
  if (isDir.value) {
    // 目录：就地展开 / 折叠，不钻入新页——树导航的核心约定
    expanded.value = !expanded.value
  } else {
    emit('activate', props.node.item)
  }
}
</script>

<style scoped>
/* 每层子容器缩进一次：层级由 DOM 嵌套自然表达，无需在 JS 里传递 / 计算深度 */
.tree-children {
  margin-left: 0.55rem;
  padding-left: 0.55rem;
  border-left: 1px solid var(--border-default);
}

.tree-row {
  display: flex;
  align-items: center;
  gap: 0.35rem;
  padding: 0.28rem 0.4rem;
  border-radius: 0.375rem;
  cursor: pointer;
  user-select: none;
  font-size: 0.82rem;
  color: var(--text-secondary);
}
.tree-row:hover {
  background: var(--surface-hover);
}
.tree-row.active {
  background: var(--accent-subtle-bg, var(--surface-hover));
  color: var(--accent);
}
.tree-row:focus-visible {
  outline: 2px solid var(--accent);
  outline-offset: -2px;
}

.tree-toggle {
  flex: 0 0 0.9rem;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  color: var(--text-muted);
  transition: transform 0.12s ease;
}
.tree-toggle svg.open {
  transform: rotate(90deg);
}
.tree-toggle.leaf {
  visibility: hidden;
}

.tree-icon {
  flex: 0 0 auto;
  width: 1rem;
  height: 1rem;
}

.tree-title {
  flex: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.tree-badge {
  flex: 0 0 auto;
  font-size: 0.68rem;
  color: var(--text-muted);
  font-variant-numeric: tabular-nums;
}
</style>
