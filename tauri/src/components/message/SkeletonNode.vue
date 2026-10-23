<!--
  容器骨架渲染器 —— `type = skeleton` 的唯一形态（[T-155 B2a]，[ADR-070 §决策 1]）

  **一整轮助手响应**与**一次工具调用**在这一格里结构同构：一个可折叠头部 + 一条子流。
  前端对它们渲染完全一样，一律递归 `children`，不认 `turn` / `tool` 这些业务名。
  两者的差别全是**数据**：标题 / 图标 / 可用动作由后端在交互面填进 `title` / `icon` /
  `actions`，本组件照数据显示；工具参数就住在骨架自身的 `content` 里，容器把它当
  **首条正文**渲染（数据驱动，不为「工具」写特例）。

  折叠体（自上而下，各段**只在有内容时出现**）：
  1. **等待骨架**：仍在 streaming、且没有任何真实子节点 → 三点脉动 +「正在思考…」；
  2. **首条正文**：自身 `content` 非空（典型是工具参数 JSON）→ 高亮代码块；
  3. **子流**：`MessageChildren` 递归（`parent-type = skeleton`）——助手正文 / 思考折叠叶 /
     工具返回 / 子过程段都从这里长出去，深一层仍是同一个 SkeletonNode（分形复用）；
  4. **无响应兜底**：父骨架自述「这次没跑」却没有任何子节点时把话讲明白（历史数据用）；
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
  >
    <div class="skeleton-body">
      <!-- ① 等待骨架：streaming 且无真实子（合成的 typing 占位不算真实子） -->
      <SkeletonPending v-if="isPending" :text="`${agentName} 正在思考…`" />

      <!-- ② 首条正文：骨架自身的 content（工具参数 JSON）——有则显示，无（整轮）则跳过 -->
      <pre v-else-if="hasSelfContent" class="json req">{{ selfText }}</pre>

      <!-- ③ 子流：一律递归（助手正文 / 思考 / 工具返回 / 子过程段） -->
      <MessageChildren
        v-if="realChildren.length"
        :nodes="realChildren"
        :depth="depth ?? 0"
        parent-type="skeleton"
        :parent-failed="isFailed"
        @retry="emit('retry', $event)"
        @delete="emit('delete', $event)"
        @edit="emit('edit', $event)"
      />

      <!-- ④ 无响应兜底：历史数据 / 不守不变量的写入方 -->
      <div v-if="missingResultNote" class="skeleton-missing">
        <span class="sm-icon">↩</span>
        <span class="sm-text">{{ missingResultNote }}</span>
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
  NODE_KIND_TYPING,
  isUnsettledNodeStatus,
  type ChatNode,
} from '@/schemas/chat_node'
import { RESUME_KEY } from '@/composables/useChatConnection'
import { useMessageContent } from '@/composables/useMessageContent'
import {
  agentNameOf,
  canSupplyToolArgs,
  isAbortedStatus,
  isFailedStatus,
  isStreamingStatus,
  messageParentSessionId,
  messageTypeOf,
  missingResultNoteOf,
  type MessageFacets,
} from '@/registry/messageTypes'
import NodeShell from './NodeShell.vue'
import MessageChildren from './MessageChildren.vue'
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

const agentName = computed(() => agentNameOf(props.node, props.facets.subSession))

/** 真实子节点：剔掉合成的 typing 占位（它是派生节点、不是内容） */
const realChildren = computed<ChatNode[]>(() =>
  (props.node.children || []).filter((c) => messageTypeOf(c) !== NODE_KIND_TYPING),
)

const isFailed = computed(() => isFailedStatus(props.facets.status))
const isAborted = computed(() => isAbortedStatus(props.facets.status))
/** 需要向用户交代的非正常终态（失败 / 中止） */
const isUnsettled = computed(() => isUnsettledNodeStatus(props.facets.status))

const { text: selfText, errorText } = useMessageContent(
  () => props.node,
  () => props.facets,
)
/** 首条正文：骨架自身 content 非空（典型 = 工具参数 JSON）；整轮容器 content 为空 → 跳过 */
const hasSelfContent = computed(() => selfText.value.trim().length > 0)

/**
 * 等待态：streaming、既无真实子、自身也无正文 —— 才是「在等第一个 token」。
 *
 * 工具骨架的参数住在**自身 content**（首条正文），一到就该显示，不能被等待脉动盖掉——
 * 否则「运行中参数必须可见」这条会在结果子节点到达前失效（观感 = 「跑完才显示」）。
 */
const isPending = computed(
  () => isStreamingStatus(props.facets.status) && realChildren.value.length === 0 && !hasSelfContent.value,
)

/** 「有请求、无响应」的兜底（只在真的没有子节点时给；判定在 registry，本组件只画） */
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
