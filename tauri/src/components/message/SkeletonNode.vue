<!--
  容器骨架渲染器 —— `type = skeleton` 的唯一形态（[T-155 B2a]，[ADR-070 §决策 1]）

  **一次工具调用**与**一段子过程**在这一格里结构同构：一个可折叠头部 + 一条子流。
  前端对它们渲染完全一样，不认 `turn` / `tool` 这些业务名。
  两者的差别全是**数据**：标题 / 图标 / 可用动作由后端在交互面填进 `title` / `icon` /
  `actions`，本组件照数据显示。容器自身正文（典型是工具调用的请求参数）由分派器
  提升为一条「请求」body 子节点（`requestViewOf`），与返回子节点同一套机制渲染——
  本组件不为它写任何特例。

  折叠体（自上而下，各段**只在有内容时出现**）：
  1. **等待骨架**：仍在 streaming、且没有任何可见子流（`childStreamOf`，含自身正文
     提升出的「请求」视图）→ 三点脉动 +「正在思考…」；
  2. **子流**：分派器经 `subtree` 插槽交来（[D-423]：递归判据 = 有无子，与形态无关）——
     「请求」视图 / 助手正文 / 思考折叠叶 / 工具返回 / 子过程段都从这里长出去，
     深一层仍是同一个 SkeletonNode（分形复用）。本组件只决定**落点在折叠体内**；
  3. **无响应兜底**：父骨架自述「这次没跑」却没有任何真实子节点时把话讲明白（历史数据用；
     **已释放态不出这条**——子节点是被本地摘掉的，不是「这次没跑」）；
  4. **折叠释放摘要**（[] 惰性重取）：后代已本地释放 ⇒ 一行 chip（多少条 / 什么类型 /
     还在跑几个），展开时经 `expand_skeleton` 按需重取（折叠态变化经 `NodeShell` 的
     `@collapse` 转给注入面，实现住 `useChatConnection`）；
  5. **交代条**：失败 / 中止 → `MessageErrorBox` + 重试（动词由后端 `actions` 决定，
     经 `messageRetryTargetOf` 在面板侧分派，本组件只发 `retry` 事件）；
  6. **补充参数**：`actions` 含 `supply` 时给一个就地补参数重跑的表单（纯数据判据）。

  层级只靠「缩进 + 左侧引导竖线」表达（见 `NodeShell.nested`），纵向间隔全由 `--msg-gap` 驱动。
-->
<template>
  <NodeShell
    :node="node"
    :facets="facets"
    :depth="depth"
    @retry="emit('retry', $event)"
    @delete="emit('delete', $event)"
    @edit="emit('edit', $event)"
    @collapse="onCollapse"
  >
    <div class="skeleton-body">
      <!-- ① 等待骨架：streaming 且无任何可见子流（合成的 typing 占位 / 请求视图都不算真实子） -->
      <SkeletonPending v-if="isPending" :text="`${agentName} 正在思考…`" />

      <!-- ② 子流：分派器按「有无可见子」交来（[D-423]），本组件只决定落点——
           放在折叠体内，因此收起容器会一并藏住「请求」视图与它的结果。 -->
      <slot name="subtree" />

      <!-- ④ 无响应兜底：历史数据 / 不守不变量的写入方。
           已释放态不出这条——子节点是被本地摘掉的，不是「这次没跑」。 -->
      <div v-if="!isReleased && missingResultNote" class="skeleton-missing">
        <span class="sm-icon">↩</span>
        <span class="sm-text">{{ missingResultNote }}</span>
      </div>

      <!-- ⑤ 折叠释放摘要（[] 惰性重取）：后代已在本地释放，只留计数与类型分布——
           展开时经 expand_skeleton 按需重取（走注入面，见 onCollapse）。 -->
      <div v-if="isReleased && releasedSummary" class="skeleton-released">
        <span class="sr-icon">🗇</span>
        <span class="sr-text">{{ releasedChip }}</span>
      </div>

      <!-- ⑤ 组级交代条：失败 / 中止都给重试入口 -->
      <MessageErrorBox
        v-if="isUnsettled"
        :text="errorText"
        :aborted="isAborted"
        retryable
        variant="skeleton"
        @retry="emit('retry', node.id)"
      />

      <!-- ⑥ 补充参数（actions 含 supply）：就地补参数重跑这一个工具 -->
      <div v-if="canSupply" class="skeleton-supply">
        <button class="supply-toggle" @click="showSupplyForm = !showSupplyForm">
          {{ showSupplyForm ? '收起' : '补充参数' }}
        </button>
        <div v-if="showSupplyForm" class="supply-form">
          <textarea
            v-model="supplyArgsText"
            class="supply-textarea"
            placeholder='{"key":"value"}  (与原参数浅合并)'
            rows="4"
          />
          <button class="supply-submit" @click="submitSupply">提交补充</button>
        </div>
      </div>
    </div>
  </NodeShell>
</template>

<script setup lang="ts">
import { computed, inject, ref } from 'vue'
import {
  RESUME_ACTION_SUPPLY,
  isUnsettledNodeStatus,
  type ChatNode,
} from '@/schemas/chat_node'
import { RESUME_KEY, SKELETON_LAZY_KEY } from '@/composables/useChatConnection'
import { useMessageContent } from '@/composables/useMessageContent'
import {
  agentNameOf,
  canSupplyToolArgs,
  childStreamOf,
  isAbortedStatus,
  isStreamingStatus,
  messageParentSessionId,
  messageTypeLabel,
  missingResultNoteOf,
  renderableChildrenOf,
  type MessageFacets,
} from '@/registry/messageTypes'
import NodeShell from './NodeShell.vue'
import MessageErrorBox from './MessageErrorBox.vue'
import SkeletonPending from './SkeletonPending.vue'

const props = defineProps<{
  node: ChatNode
  facets: MessageFacets
  depth?: number
}>()

const emit = defineEmits<{
  retry: [messageId: string]
  delete: [messageId: string]
  edit: [messageId: string]
}>()

/** 会话恢复入口（由面板提供）：supply / 子会话路由都走它 */
const resume = inject(RESUME_KEY)

/**
 * 骨架惰性重取面（由面板提供）：折叠态变化转发给它——释放 / 重取的实现住在
 * `useChatConnection`（它持有会话 id 与转写 store），本组件只当「折叠态变了」
 * 的传声筒。缺注入（组件单测直接挂载）时退化为无操作：chip 不显示、折叠
 * 不释放——界面照常可用，只是没有惰性重取。
 */
const skeletonLazy = inject(SKELETON_LAZY_KEY, null)

function onCollapse(open: boolean) {
  skeletonLazy?.onCollapse(props.node.id, open)
}

/** 本骨架的后代是否已被本地释放（折叠 chip 的显示判据） */
const isReleased = computed(() => skeletonLazy?.isReleased(props.node.id) ?? false)
const releasedSummary = computed(() => skeletonLazy?.summaryOf(props.node.id))

/**
 * 折叠 chip 文案：多少条 / 什么类型 / 还在跑几个（判据 4）。
 *
 * 类型分布经 `messageTypeLabel` 翻成人话（`skeleton` ⇒ 「组合」）——线格式词
 * 不直接摆给用户看。「还在跑」只在非零时出现：全零还写一栏等于公报噪音。
 */
const releasedChip = computed(() => {
  const s = releasedSummary.value
  if (!s) return ''
  const parts = [`已折叠释放 ${s.count} 项`]
  const types = Object.entries(s.byType)
    .sort((a, b) => b[1] - a[1])
    .map(([t, n]) => `${messageTypeLabel(t)} ${n}`)
  if (types.length) parts.push(types.join(' · '))
  if (s.running > 0) parts.push(`还在跑 ${s.running} 个`)
  return parts.join(' · ')
})

const agentName = computed(() => agentNameOf(props.node, props.facets.subSession))

/** 真实子节点：判据与分派器同一份（剔掉合成的 typing 占位） */
const realChildren = computed<ChatNode[]>(() => renderableChildrenOf(props.node))

const isAborted = computed(() => isAbortedStatus(props.facets.status))
/** 需要向用户交代的非正常终态（失败 / 中止） */
const isUnsettled = computed(() => isUnsettledNodeStatus(props.facets.status))

const { errorText } = useMessageContent(
  () => props.node,
  () => props.facets,
)

/**
 * 等待态：streaming 且**无任何可见子流**（`childStreamOf` = 真实子 + 自身正文
 * 提升出的「请求」视图，与分派器的子流来源同一份判据）——才是「在等第一个节点」。
 * 容器自身正文一到，请求视图立即顶掉等待脉动（否则「运行中参数必须可见」这条
 * 会在结果子节点到达前失效，观感 = 「跑完才显示」）。
 */
const isPending = computed(
  () => isStreamingStatus(props.facets.status) && childStreamOf(props.node).length === 0,
)

/** 「有请求、无响应」的兜底只看**真实子**（后端落地的子节点）：
 *  请求视图是派生节点，不能让它把「这次没跑」的交代顶掉。 */
const missingResultNote = computed<string | null>(() =>
  realChildren.value.length ? null : missingResultNoteOf(props.node),
)

// ── 补充参数表单（纯数据判据：actions 含 supply）──────────
const canSupply = computed(() => canSupplyToolArgs(props.facets))
const showSupplyForm = ref(false)
const supplyArgsText = ref('')

function submitSupply() {
  let parsed: unknown
  try {
    parsed = JSON.parse(supplyArgsText.value || '{}')
  } catch {
    return // JSON 解析失败：静默忽略
  }
  if (!resume) return
  resume({
    targetId: props.node.id,
    action: RESUME_ACTION_SUPPLY,
    args: parsed,
    // 子会话工具调用需路由回子会话（否则 resume 会打到父会话）
    targetSessionId: messageParentSessionId(props.node),
  })
  showSupplyForm.value = false
}
</script>

<style scoped>
.skeleton-body {
  display: flex;
  flex-direction: column;
  gap: var(--msg-gap, 0.6rem);
}
/* 无响应兜底：中性色而非错误色——「没轮到执行」不是故障。 */
.skeleton-missing {
  display: flex;
  align-items: baseline;
  gap: 0.35rem;
  font-size: 0.78rem;
  color: var(--text-muted);
}

/* ── 折叠释放摘要（[] 惰性重取）── */
.skeleton-released {
  display: flex;
  align-items: baseline;
  gap: 0.35rem;
  font-size: 0.78rem;
  color: var(--text-muted);
}
.sr-icon {
  flex-shrink: 0;
}
.sr-text {
  word-break: break-word;
}
.sm-icon {
  flex-shrink: 0;
}
.sm-text {
  word-break: break-word;
}

/* ── 补充参数 UI（actions 含 supply 时出现）── */
.skeleton-supply {
  display: flex;
  flex-direction: column;
  gap: 0.35rem;
}
.supply-toggle {
  align-self: flex-start;
  background: var(--color-supply-bg);
  border: 1px solid var(--color-prompt-border);
  color: var(--color-chip-tool-fg);
  border-radius: 0.375rem;
  padding: 0.2rem 0.6rem;
  font-size: 0.76rem;
  cursor: pointer;
}
.supply-toggle:hover {
  background: var(--color-prompt-bg);
}
.supply-form {
  display: flex;
  flex-direction: column;
  gap: 0.35rem;
  background: var(--color-msg-card);
  border: 1px solid var(--color-supply-border);
  border-radius: 0.5rem;
  padding: 0.5rem 0.6rem;
}
.supply-textarea {
  width: 100%;
  box-sizing: border-box;
  border: 1px solid var(--color-supply-border);
  border-radius: 0.375rem;
  padding: 0.35rem 0.45rem;
  font-family: 'JetBrains Mono', ui-monospace, monospace;
  font-size: 0.76rem;
  line-height: 1.5;
  resize: vertical;
  outline: none;
}
.supply-textarea:focus {
  border-color: var(--accent);
}
.supply-submit {
  align-self: flex-start;
  background: var(--accent);
  color: var(--text-on-accent);
  border: none;
  border-radius: 0.375rem;
  padding: 0.3rem 0.9rem;
  font-size: 0.78rem;
  cursor: pointer;
}
.supply-submit:hover {
  background: var(--accent-hover);
}
</style>
