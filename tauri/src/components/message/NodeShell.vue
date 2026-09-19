<!--
  统一折叠式节点外壳 —— 会话流里**所有内容节点**的共同外观与交互

  职责边界（只做外壳，不碰内容）：
  - 容器与层级：`.msg` 包装、`type-*` / `status-*` 修饰类、`nested` 缩进与引导竖线；
  - 折叠头部：图标 + 标题 + 心跳标 + 收起态摘要 + 状态标签（含运行中动效与秒数）
    + 悬停操作（编辑 / 就地重试 / 删除）；
  - 折叠交互：默认态与深层级策略查 `registry/messageTypes`，本组件只持有**交互态**；
  - 折叠体：**不关心内容**，由调用方经默认插槽填入（气泡 / Markdown / 工具卡 / 表单…）。

  这样「哪种节点长什么样」只有一份实现，个体渲染器只负责「折叠体里放什么」。
  头部是否渲染由 `facets.responseText` 决定：Turn 的直接正文子节点内联直排，
  重复一个「助手」行头属于冗余。

  判定（图标 / 标题 / 标签 / 色调 / 折叠策略）**全部**来自 `registry/messageTypes`，
  本组件不含任何「哪个类型显示什么」的判定。
-->
<template>
  <div class="msg" :class="[typeClass, statusClass, isUser ? 'user' : '', nested ? 'nested' : '']">
    <div v-if="!facets.responseText" class="node-head" :class="headClass" @click="toggle">
      <span class="node-icon">{{ icon }}</span>
      <span class="node-title">{{ title }}</span>
      <span v-if="isHeartbeat" class="node-heartbeat" title="系统心跳任务自动发送">♥ 心跳</span>
      <!-- 收起态展示单行摘要（深层级 / 子步骤默认收起时，让用户无需展开即知内容） -->
      <span v-if="!effectiveOpen && summaryPreview" class="node-preview">{{ summaryPreview }}</span>
      <span v-if="statusTag" class="node-tag" :class="tagClass">
        <span v-if="isRunningAction" class="tag-dots"><span /><span /><span /></span>{{ statusTag
        }}<span v-if="isRunningAction && runningDuration" class="tag-elapsed">{{ runningDuration }}</span>
      </span>
      <span v-if="showsLiveBadge" class="node-live">回复中…</span>
      <!-- 悬停操作：用户消息可编辑；失败工具可就地重试；仅 root 级节点可删除 -->
      <span class="node-actions" @click.stop>
        <button v-if="isUser" class="node-act" title="编辑" @click.stop="emit('edit', node.id)">✎</button>
        <button v-if="canRetry" class="node-act" title="重试此工具" @click.stop="emit('retry', node.id)">↻</button>
        <button v-if="!node.parent_id" class="node-act" title="删除" @click.stop="emit('delete', node.id)">🗑</button>
      </span>
    </div>

    <div v-if="effectiveOpen" class="node-body">
      <slot />
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, onScopeDispose, ref, watch } from 'vue'
import { CHAT_ROLE_USER, type ChatMessage } from '@/schemas/chat_message'
import { useMessageContent } from '@/composables/useMessageContent'
import { useRunningClock } from '@/composables/useRunningClock'
import {
  agentNameOf,
  canRetryTool,
  effectiveOpenOf,
  messageHeadModifier,
  messageIcon,
  messageIsHeartbeat,
  messageIsRunningAction,
  messageShowsLiveBadge,
  messageStartedAt,
  messageStatusTag,
  messageStatusTone,
  messageTitle,
  nextOpenOf,
  type MessageFacets,
} from '@/registry/messageTypes'

const props = defineProps<{
  node: ChatMessage
  /** 呈现判定入参（由分派器算好传入，避免同一节点被算两遍） */
  facets: MessageFacets
  /** 嵌套深度：0 = 主会话根级；>0 = 工具 / 子 agent 内部的子会话流层级 */
  depth?: number
}>()

const emit = defineEmits<{
  retry: [messageId: string]
  delete: [messageId: string]
  edit: [messageId: string]
}>()

const isUser = computed(() => props.facets.role === CHAT_ROLE_USER)
/** 分形层级：仅 depth ≥ 2 施加缩进 + 左侧引导竖线（depth=1 与主回合左缘齐平） */
const nested = computed(() => (props.depth ?? 0) > 1)
const typeClass = computed(() => `type-${props.facets.type}`)
const statusClass = computed(() => `status-${props.facets.status}`)

// ── 头部展示信息（全部查表，见文件头注释）──────────────────
const agentName = computed(() => agentNameOf(props.node, props.facets.subSession))
const icon = computed(() => messageIcon(props.facets))
const title = computed(() => messageTitle(props.facets, agentName.value))
const statusTag = computed(() => messageStatusTag(props.facets))
const tagClass = computed(() => messageStatusTone(props.facets))
const headClass = computed(() => messageHeadModifier(props.facets))
const showsLiveBadge = computed(() => messageShowsLiveBadge(props.facets))
const isHeartbeat = computed(() => messageIsHeartbeat(props.node))
const canRetry = computed(() => canRetryTool(props.facets))

// ── 折叠状态 ───────────────────────────────────────────────
//
// 折叠**策略**（默认态、深层级、用户覆盖的组合）在 `registry/messageTypes`——
// 它是「哪种消息默认展开」这条知识的唯一落点。本组件只持有**交互态**。
const userToggled = ref(false)
const userOpen = ref(true)

const effectiveOpen = computed(() =>
  effectiveOpenOf(props.facets, props.depth ?? 0, userToggled.value, userOpen.value),
)

function toggle() {
  // 基于「当前实际展示态」取反，而非固定初始值：否则默认收起节点首次点击会从
  // 初始 userOpen(true) 翻成 false，展示态不变（仍收起），表现为"第一次点击无反应"。
  // 该语义封装在 `nextOpenOf` 里并有单测锁死。
  userOpen.value = nextOpenOf(props.facets, props.depth ?? 0, userToggled.value, userOpen.value)
  userToggled.value = true
}

const { summaryPreview } = useMessageContent(
  () => props.node,
  () => props.facets,
)

// ── 运行中计时（共享秒级时钟）──────────────────────────────
//
// 「运行中」是一个**断言**，「已运行 47s」才是**判据**：工具执行可能持续几十秒到
// 几分钟（首次语义索引、长命令、子智能体），用户需要靠它区分"还在跑"与"卡住了"。
//
// 时钟本体是**模块级共享单例**（`useRunningClock`）：会话流是递归结构，
// 每实例一个定时器会让数量随会话长度线性增长。见该模块的注释。
const isRunningAction = computed(() => messageIsRunningAction(props.facets))
const { nowMs, acquire, release } = useRunningClock()
watch(
  isRunningAction,
  (running) => {
    if (running) acquire()
    else release()
  },
  { immediate: true },
)
onScopeDispose(() => {
  // 卸载兜底：漏 release 会让时钟永不停止（功能不错，只是白耗电）
  if (isRunningAction.value) release()
})

/** 运行中已持续的秒数；无锚点时返回 null（不显示时长，而不是编一个） */
const runningSeconds = computed(() => {
  if (!isRunningAction.value) return null
  const anchor = messageStartedAt(props.node)
  if (anchor === null) return null
  return Math.max(0, Math.floor((nowMs.value - anchor) / 1000))
})
/** 时长文案：60s 内用秒，之后用「分m秒s」（长任务最常见的就是分钟级） */
const runningDuration = computed(() => {
  const s = runningSeconds.value
  if (s === null) return ''
  if (s < 60) return `${s}s`
  return `${Math.floor(s / 60)}m${String(s % 60).padStart(2, '0')}s`
})
</script>

<style scoped>
.msg {
  width: 100%;
}
/* 分形层级：仅用「缩进 + 左侧引导竖线」表达嵌套深度（类邮件/论坛主题串），
   不再叠加任何外框，彻底避免「竖线 + 外框」的视觉冲突。
   注意：缩进只作用于 depth>=2 的层级（见 nested 判定）。depth=1 的子步骤（思考过程 /
   工具调用）保持与主回合头部（AI 头像）左边缘齐平 —— 这是「头像与思考过程左对齐」的关键：
   顶层 Turn 头部 icon 位于 msg 容器左缘（x=0），depth=1 子节点不再缩进，故内容左缘同样为 x=0，
   二者天然对齐；更深层级才施加统一缩进并绘制引导竖线。 */
.msg.nested {
  margin-left: var(--nest-indent, 0.7rem);
  padding-left: var(--nest-indent, 0.7rem);
  border-left: 0.125rem solid var(--color-nested-line);
}

/* 思考行流式动效：标题呼吸闪烁（单行态，不展开内容）。
   思考中 **与工具执行中** 共用（`headClass.thinking`）——两者都是"正在进行、
   内容尚未成形"的行，同一手法保证观感一致。 */
.node-head.thinking .node-title {
  animation: think-pulse 1.4s ease-in-out infinite;
}
@keyframes think-pulse {
  0%,
  100% {
    opacity: 0.55;
  }
  50% {
    opacity: 1;
  }
}

/* ── 统一节点头部（所有节点一致）── */
.node-head {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  cursor: pointer;
  user-select: none;
  padding: 0.16rem 0.3rem;
  margin: 0 -0.3rem;
  border-radius: 0.375rem;
  font-size: 0.8rem;
  transition: background 0.12s ease;
}
.node-head:hover {
  background: rgba(99, 102, 241, 0.06);
}
/* 用户消息整体右对齐；头像置于文字右侧（主流 IM 习惯），折叠箭头在最右。
   用 order 重排而非 row-reverse，避免箭头被推到最左。 */
.node-head.user {
  justify-content: flex-end;
}
.node-head.user .node-title {
  order: 1;
}
.node-head.user .node-icon {
  order: 2;
}
.node-icon {
  width: 1.125rem;
  height: 1.125rem;
  border-radius: 0.375rem;
  font-size: 0.62rem;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  background: var(--color-chip-bg);
  color: var(--color-chip-fg);
}
.node-head.user .node-icon {
  background: var(--color-chip-user-bg);
  color: var(--color-chip-user-fg);
}
.node-head.sub .node-icon {
  background: var(--color-chip-sub-bg);
  color: var(--color-chip-sub-fg);
}
.node-head.tool .node-icon {
  background: var(--color-chip-tool-bg);
  color: var(--color-chip-tool-fg);
}
.node-head.reasoning .node-icon {
  background: var(--color-chip-reasoning-bg);
  color: var(--color-chip-reasoning-fg);
}
.node-title {
  font-weight: 500;
  font-size: 0.8rem;
  color: var(--text-secondary);
  flex-shrink: 1;
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.node-head.sub .node-title {
  color: var(--color-chip-sub-fg);
}
/* 收起态单行摘要（标题之后、状态标签之前，省略号截断） */
.node-preview {
  flex: 1;
  min-width: 0;
  font-size: 0.74rem;
  color: var(--text-muted);
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.node-live {
  font-size: 0.72rem;
  color: var(--color-chip-tool-fg);
}
.node-head.sub .node-live {
  color: var(--color-chip-sub-fg);
}

/* ── 悬停操作（编辑 / 重试 / 删除）──
   默认隐藏，鼠标悬停整条消息时显示在头部右侧。
   用 @click.stop 阻止冒泡触发头部折叠。 */
.node-actions {
  display: none;
  align-items: center;
  gap: 0.15rem;
  margin-left: 0.25rem;
  flex-shrink: 0;
}
.msg:hover > .node-head .node-actions {
  display: inline-flex;
}
.node-tag {
  font-size: 0.68rem;
  padding: 0.05rem 0.4rem;
  border-radius: 62.4375rem;
}
.node-tag.sub {
  background: var(--color-tag-sub-bg);
  color: var(--color-tag-sub-fg);
}
/* 工具调用「运行中」：主色 + 三点脉动 + 已运行时长。
   三点与 `.turn-pending-dots` 同一手法（同节奏、同三点，仅尺寸缩到标签内），
   使"正在跑"在会话流里无论出现在等待骨架还是工具行，观感都是同一件事。
   时长是**判据**而非装饰：用户靠它区分"还在跑"与"卡住了"。 */
.node-tag.run {
  display: inline-flex;
  align-items: center;
  gap: 0.25rem;
  background: var(--accent-subtle-bg);
  color: var(--accent);
}
.tag-dots {
  display: inline-flex;
  gap: 0.15rem;
}
.tag-dots span {
  width: 0.25rem;
  height: 0.25rem;
  border-radius: 50%;
  background: currentColor;
  animation: turn-pulse 1.2s infinite ease-in-out;
}
.tag-dots span:nth-child(2) {
  animation-delay: 0.2s;
}
.tag-dots span:nth-child(3) {
  animation-delay: 0.4s;
}
@keyframes turn-pulse {
  0%,
  80%,
  100% {
    opacity: 0.25;
    transform: scale(0.85);
  }
  40% {
    opacity: 1;
    transform: scale(1);
  }
}
/* 时长用等宽数字，避免每秒跳动时标签宽度抖动 */
.tag-elapsed {
  font-variant-numeric: tabular-nums;
  opacity: 0.75;
}
.node-tag.warn {
  background: var(--color-tag-warn-bg);
  color: var(--color-tag-warn-fg);
}
.node-tag.err {
  background: var(--color-tag-err-bg);
  color: var(--color-tag-err-fg);
}
.node-heartbeat {
  display: inline-flex;
  align-items: center;
  font-size: 0.7rem;
  font-weight: 500;
  color: #ef4444;
  background: rgba(239, 68, 68, 0.12);
  border-radius: 0.25rem;
  padding: 0 0.35rem;
  margin-left: 0.25rem;
  flex-shrink: 0;
}

/* ── 折叠体 ──
   兄弟元素（思考过程 / 工具调用 / 文本回复）之间、以及头部到首个子元素的纵向间隔，
   全部引用统一变量 --msg-gap，确保「助手 ↔ 思考过程」与「思考过程 ↔ 工具调用」等
   所有间隔完全一致（机制化保障，而非散落硬编码）。 */
.node-body {
  display: flex;
  flex-direction: column;
  gap: var(--msg-gap, 0.6rem);
  padding: var(--msg-gap, 0.6rem) 0 0;
}

/* ── 响应流排版（现代扁平 agent 风格）──
   非用户响应节点（正文 / 思考过程 / 工具调用）统一无框扁平：
   直接落在聊天背景上，仅由「节点头（图标 + 名称 + 折叠箭头）」分组，
   去掉背景卡片与外描边，消除「卡片内边距 + 条目间距」双层空白叠加造成的稀疏感。
   代码块（.json / markdown pre）属于内容块，保留底色以保证可读性；
   交互类卡片（user_prompt 提问 / supply 补充参数）保留淡表面以维持操作可辨识性。 */
.msg.type-text:not(.user),
.msg.type-reasoning,
.msg.type-tool_call {
  padding: 0.12rem var(--card-pad-x, 0.5rem);
}
</style>
