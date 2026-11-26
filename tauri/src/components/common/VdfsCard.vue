<!--
  VdfsCard — 通用"列表项"卡片（VDFS 节点的一张卡）

  适用于工作台中栏列表：模型 / MCP / 技能 / 智能体 / 设置分区…
  一切由 `vdfs/list` 下发的节点，卡片本身不认识资源类型。

  视觉风格对齐会话卡片：
  - 状态点 + 标题行
  - 副标题（描述预览）
  - 元信息行（来源/类型/其他标签）
-->
<template>
  <div
    class="vdfs-card"
    :class="{ active: isActive, disabled }"
    role="option"
    :tabindex="disabled ? -1 : 0"
    :aria-selected="isActive"
    :aria-disabled="disabled"
    :aria-label="accessibleLabel"
    @click="$emit('click')"
    @keydown.enter="$emit('click')"
    @keydown.space.prevent="$emit('click')"
  >
    <!-- 状态条 + 类型图标 + 标题（状态点可由后端控制隐藏） -->
    <div class="card-header">
      <div v-if="showStatus" class="status-area">
        <span :class="['status-dot', statusClass]" :title="statusTitle" />
      </div>
      <component v-if="icon" :is="icon" class="card-icon" />
      <div class="title-row">
        <span class="title-text" :title="title">{{ title }}</span>
        <span v-if="badge" class="activity-text" :class="badgeClass">{{ badge }}</span>
      </div>
    </div>

    <!-- 副标题（描述） -->
    <div v-if="subtitle" class="card-preview">
      <span class="preview-text">{{ subtitle }}</span>
    </div>

    <!-- 元信息行 -->
    <div v-if="$slots.meta || tags.length" class="card-meta">
      <slot name="meta" />
      <template v-for="(tag, i) in tags" :key="i">
        <span class="tag" :class="`tag-${tag.kind || 'default'}`">{{ tag.label }}</span>
      </template>
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, type Component } from 'vue'

interface Tag {
  label: string
  kind?: 'default' | 'primary' | 'success' | 'warn' | 'info' | 'muted'
}

interface VdfsCardProps {
  title: string
  subtitle?: string
  status?: 'active' | 'working' | 'disabled' | 'warning' | 'error' | 'muted'
  statusTitle?: string
  badge?: string
  badgeKind?: 'default' | 'primary' | 'success' | 'warn' | 'info'
  isActive?: boolean
  disabled?: boolean
  tags?: Tag[]
  /** 类型图标组件（registry/vdfsIcons 的纯 UI 映射，如设置分区图标） */
  icon?: Component
  /** 是否显示状态点（由节点 status 决定；无状态的类型由调用方关闭） */
  showStatus?: boolean
}

const props = withDefaults(defineProps<VdfsCardProps>(), {
  status: 'active',
  statusTitle: '',
  badgeKind: 'default',
  isActive: false,
  disabled: false,
  tags: () => [],
  showStatus: true,
})

defineEmits<{
  (e: 'click'): void
}>()

const statusClass = computed(() => `status-${props.status}`)
const badgeClass = computed(() => `kind-${props.badgeKind}`)

/**
 * 可访问名称带上状态：状态点只有鼠标 `title`，读屏用户只看得到标题，
 * 听不到「运行中 / 失败 / 已禁用」。
 */
const accessibleLabel = computed(() => {
  const parts = [props.title]
  if (props.statusTitle) parts.push(props.statusTitle)
  else if (props.badge) parts.push(props.badge)
  return parts.join('，')
})
</script>

<style scoped>
.vdfs-card {
  margin: var(--space-1) var(--space-2);
  padding: var(--space-2) var(--space-3);
  background: var(--surface-overlay);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-lg);
  cursor: pointer;
  transition: background-color var(--motion-fast) var(--motion-ease),
    border-color var(--motion-fast) var(--motion-ease),
    box-shadow var(--motion-fast) var(--motion-ease);
  user-select: none;
}

/* hover 只给轻微的边框加深与阴影，不用 accent；选中态才是 accent 边框 +
   背景填充的强信号。两者同色时，鼠标停在哪一项上，看起来都像「已选中」，
   在「列表未选中但详情铺着新建草稿」的场景下尤其误导。 */
.vdfs-card:hover {
  border-color: var(--border-strong);
  box-shadow: var(--shadow-1);
}

.vdfs-card.active {
  border-color: var(--accent);
  background: var(--surface-selected);
}

.vdfs-card.disabled {
  opacity: 0.55;
}

/* 状态条 + 标题 */
.card-header {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  margin-bottom: var(--space-1);
}

.status-area {
  display: flex;
  align-items: center;
  flex-shrink: 0;
}

.status-dot {
  display: inline-block;
  width: 0.4375rem;
  height: 0.4375rem;
  border-radius: var(--radius-full);
  background: var(--text-disabled);
  flex-shrink: 0;
}
.status-dot.status-active { background: var(--success-solid); }
/* working：工作中（如会话处理中）——accent 色脉冲点 */
.status-dot.status-working {
  background: var(--accent);
  animation: status-pulse 1.4s var(--motion-ease, ease-in-out) infinite;
}
@keyframes status-pulse {
  0%, 100% { box-shadow: 0 0 0 0 transparent; opacity: 1; }
  50% { box-shadow: 0 0 0 0.25rem var(--accent-subtle-bg, rgba(0, 0, 0, 0.15)); opacity: 0.75; }
}
@media (prefers-reduced-motion: reduce) {
  .status-dot.status-working { animation: none; }
}
.status-dot.status-disabled { background: var(--border-strong); }
.status-dot.status-warning { background: var(--warning-solid); }
.status-dot.status-error { background: var(--danger-solid); }
.status-dot.status-muted { background: var(--text-disabled); }

/* 类型图标 */
.card-icon {
  /* 列表内图标 16px：图标组件不写死尺寸（见 `registry/vdfsIcons.svgIcon`），
     由本容器定，与侧栏的 20px 各自成立而互不影响。 */
  width: 1rem;
  height: 1rem;
  flex-shrink: 0;
  color: var(--text-secondary);
}

.title-row {
  flex: 1;
  display: flex;
  align-items: center;
  gap: var(--space-2);
  min-width: 0;
}

.title-text {
  font-size: var(--font-size-base);
  font-weight: var(--font-weight-semibold);
  color: var(--text-primary);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  flex: 1;
}

.activity-text {
  font-size: var(--font-size-xs);
  color: var(--text-muted);
  flex-shrink: 0;
}
.activity-text.kind-primary { color: var(--accent); }
.activity-text.kind-success { color: var(--success-solid); }
.activity-text.kind-warn { color: var(--warning-solid); }
.activity-text.kind-info { color: var(--info-solid); }

/* 副标题 */
.card-preview {
  margin: var(--space-1) 0;
  font-size: var(--font-size-xs);
  color: var(--text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

/* 元信息：保持单行。原先 flex-wrap:wrap 在窄屏（列表被压到 ~8rem）时把
   「来源 / 条数 / 时间」拆成三行，一张卡被撑得又高又碎。现在不换行、溢出省略，
   meta 是辅助信息，窄屏下截断优于逐行堆叠。 */
.card-meta {
  display: flex;
  align-items: center;
  gap: var(--space-2);
  flex-wrap: nowrap;
  overflow: hidden;
  margin-top: var(--space-1);
  font-size: var(--font-size-xs);
  color: var(--text-muted);
}

.card-meta > * {
  flex-shrink: 0;
}

.tag {
  padding: 0.1rem 0.4rem;
  background: var(--surface-sunken);
  border-radius: var(--radius-sm);
  /* 元信息已经压到 11px 的下限，别再小一号：0.65rem（10.4px）在浅色令牌的
     --text-muted 上偏细，读「3 条 / 1 小时前」要眯眼。用字体令牌与别处对齐。 */
  font-size: var(--font-size-xs);
}
.tag-primary { background: var(--accent-subtle-bg); color: var(--accent); }
.tag-success { background: var(--success-bg); color: var(--success-fg); }
.tag-warn    { background: var(--warning-bg); color: var(--warning-fg); }
.tag-info    { background: var(--info-bg); color: var(--info-fg); }
.tag-muted   { background: var(--surface-sunken); color: var(--text-muted); }

/* 描述给两行：列表卡片宽度受 Workbench 中栏约束（窄屏还会进一步收缩），
   单行 nowrap 会把「当前目录下共有 **27 个条目**（13 个目录…」这类有用信息
   砍成半句。两行截断既能读全意图，又不会让卡片无限变高。标题仍保持单行——
   标题是身份，被截断有 title 属性兜底；描述是辅助信息，两行足够。

   `white-space: normal` 不能省：父级 .card-preview 是单行的 nowrap，span 会继承；
   在 nowrap 下 -webkit-line-clamp 失效，实测变成「一行硬裁、连省略号都没有」，
   与上面这句两行的承诺不符。显式恢复换行才让 clamp 与末尾省略号真正生效。 */
.preview-text {
  font-size: var(--font-size-xs);
  color: var(--text-secondary);
  line-height: 1.4;
  overflow: hidden;
  white-space: normal;
  display: -webkit-box;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
  line-clamp: 2;
}
</style>
