<!--
  待用户响应渲染器 —— `type = interaction` 的唯一形态（ask_user 提问 / 工具执行确认）

  两种子形态由数据契约判别键 `kind` 分派（`schemas/message_prompt.ts`）：
  · `question`：一个或多个问题，可单选 / 多选，含约定选项 `Other`（勾选后自由输入）；
  · `confirm` ：工具名 + 风险等级 + 参数 + 描述，三键——批准本次 / 本会话放行 / 拒绝。

  三键为什么是三个平级按钮，而不是「批准」加一个「以后别再问」的复选框：
  `approve` 记的是「用户批了**这一次**」（`authz.granted`，一次调用一笔）；
  「本会话放行」改的是**会话的执行阈值**（同档及以下的工具从此不再问）。两笔
  事实的生命周期不同，做成复选框就把「以后别再问」藏在一次批准里——用户点的是
  「批准」，后果却是「这一档以后都不问了」。第三档「永久放行」没有落点（跨会话
  的授权清单不存在），故界面上只有这两档 + 拒绝。

  **已回答态**（`status` 不再是 `waiting_user_action`）不再隐藏表单，而是整体降透明度
  并禁用控件：用户事后回看会话时，仍能看到自己当初选了什么。

  提交都经 `RESUME_KEY` 注入的动作打到**父骨架**（`parent_id`）——提问与确认都是
  某个工具调用的一部分，锚点必须是父容器骨架而不是本节点自身。子会话里的提问还要
  带上 `parent_session_id`，否则恢复会打到父会话。「本会话放行」另走
  `ALLOW_FOR_SESSION_KEY`：它改的是会话阈值，不是这条消息的恢复。
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
    <div class="user-prompt" :class="{ answered: isAnswered }">
      <!-- 提问 -->
      <template v-if="questionPrompt">
        <div v-for="q in questionPrompt.questions" :key="q.id" class="up-question">
          <div v-if="q.header" class="up-header">{{ q.header }}</div>
          <div class="up-qtext">{{ q.question }}</div>
          <div class="up-options">
            <label
              v-for="opt in q.options"
              :key="opt.label"
              class="up-option"
              :class="{ active: isChosen(q.id, opt.label) }"
            >
              <input
                :type="q.multiSelect ? 'checkbox' : 'radio'"
                :name="q.id"
                :value="opt.label"
                :checked="isChosen(q.id, opt.label)"
                :disabled="isAnswered"
                @change="onToggleOption(q.id, !!q.multiSelect, opt.label)"
              />
              <span class="up-opt-label">{{ opt.label }}</span>
              <span v-if="opt.description" class="up-opt-desc">{{ opt.description }}</span>
            </label>
            <!-- `Other` 自由输入：与普通选项同一行，未勾选时输入框禁用 -->
            <label
              v-if="hasOtherOption(q)"
              class="up-option up-option-other"
              :class="{ active: isChosen(q.id, MESSAGE_PROMPT_OTHER) }"
            >
              <input
                type="checkbox"
                :name="q.id + '__other'"
                :checked="isChosen(q.id, MESSAGE_PROMPT_OTHER)"
                :disabled="isAnswered"
                @change="onToggleOption(q.id, true, MESSAGE_PROMPT_OTHER)"
              />
              <span class="up-opt-label">{{ MESSAGE_PROMPT_OTHER }}</span>
              <input
                v-model="customInput[q.id]"
                class="up-other-input"
                type="text"
                placeholder="请输入…"
                :disabled="isAnswered || !isChosen(q.id, MESSAGE_PROMPT_OTHER)"
              />
            </label>
          </div>
        </div>
        <button class="up-submit" :disabled="isAnswered" @click="submitQuestions">
          {{ isAnswered ? '已提交' : '提交' }}
        </button>
      </template>

      <!-- 工具确认 -->
      <template v-else-if="confirmPrompt">
        <div class="up-confirm">
          <div class="up-confirm-tool">
            <span class="up-tool-name">{{ confirmPrompt.tool_name }}</span>
            <span
              v-if="confirmPrompt.risk_level"
              class="up-risk"
              :class="'risk-' + confirmPrompt.risk_level"
              >{{ confirmPrompt.risk_level }}</span
            >
          </div>
          <div class="up-confirm-desc">{{ confirmPrompt.description }}</div>
          <pre v-if="confirmPrompt.args !== undefined" class="up-args">{{
            JSON.stringify(confirmPrompt.args, null, 2)
          }}</pre>
        </div>
        <div class="up-confirm-btns">
          <button class="up-approve" :disabled="isAnswered" @click="submitConfirm(true)">
            {{ isAnswered ? '已处理' : '批准执行' }}
          </button>
          <!-- 「本会话放行」只在这次调用确实带风险档时出现：没有档位就没有
               「抬到哪一档」可言（未声明风险的工具按 Medium 兜底，那种情况下
               抬档没有可读的因果）。点它 = 抬阈值 + 批准本次，两步见 submitAllowForSession。 -->
          <button
            v-if="confirmRisk"
            class="up-allow-session"
            :disabled="isAnswered"
            @click="submitAllowForSession"
          >
            {{ isAnswered ? '已处理' : '本会话放行' }}
          </button>
          <button class="up-reject" :disabled="isAnswered" @click="submitConfirm(false)">
            {{ isAnswered ? '已处理' : '拒绝' }}
          </button>
        </div>
      </template>
    </div>
  </NodeShell>
</template>

<script setup lang="ts">
import { computed, inject, ref } from 'vue'
import {
  RESUME_ACTION_ANSWER,
  RESUME_ACTION_APPROVE,
  RESUME_ACTION_REJECT,
  type ChatNode,
} from '@/schemas/chat_node'
import {
  MESSAGE_PROMPT_OTHER,
  MESSAGE_REJECT_REASON,
  type MessagePromptAnswer,
  type MessagePromptQuestion,
} from '@/schemas/message_prompt'
import { RESUME_KEY, ALLOW_FOR_SESSION_KEY } from '@/composables/useChatConnection'
import { isSessionRiskLevel, type SessionRiskLevel } from '@/schemas/session_meta'
import {
  isWaitingStatus,
  messageParentSessionId,
  promptOf,
  type MessageFacets,
} from '@/registry/messageTypes'
import NodeShell from './NodeShell.vue'

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

const resume = inject(RESUME_KEY)
/** 「本会话放行」（审批卡第三档）；无人提供时按「没有这一档」处理 */
const allowForSession = inject(ALLOW_FOR_SESSION_KEY, null)

const prompt = computed(() => promptOf(props.node))
const questionPrompt = computed(() =>
  prompt.value?.kind === 'question' ? prompt.value : null,
)
const confirmPrompt = computed(() => (prompt.value?.kind === 'confirm' ? prompt.value : null))

/**
 * 本次调用可读的风险档（审批卡第三档的出现条件）。
 *
 * 校验走契约层的词表（`isSessionRiskLevel`）而不是在组件里手写三个字面量：
 * `risk_level` 是后端产出的裸字符串，多一档时手写的判定会静默把它判成非法——
 * 表现是「这一档的审批卡少一个按钮」，没有任何东西会红。
 */
const confirmRisk = computed<SessionRiskLevel | null>(() => {
  const raw = confirmPrompt.value?.risk_level
  return isSessionRiskLevel(raw) ? raw : null
})

/** 已回答：状态不再是 waiting_user_action → 控件禁用并改为已提交态 */
const isAnswered = computed(() => !isWaitingStatus(props.facets.status))

/** 表单本地状态：questionId → 选中的 label 列表；`Other` 勾选时的自由文本 */
const selected = ref<Record<string, string[]>>({})
const customInput = ref<Record<string, string>>({})

function isChosen(questionId: string, label: string): boolean {
  return (selected.value[questionId] || []).includes(label)
}
function hasOtherOption(q: MessagePromptQuestion): boolean {
  return q.options.some((o) => o.label === MESSAGE_PROMPT_OTHER)
}

function onToggleOption(questionId: string, multiSelect: boolean, label: string) {
  const cur = selected.value[questionId] || []
  if (multiSelect) {
    // 多选：切换该 label；`Other` 与普通选项可共存
    const next = cur.includes(label) ? cur.filter((l) => l !== label) : [...cur, label]
    selected.value = { ...selected.value, [questionId]: next }
  } else {
    // 单选：直接替换为该 label
    selected.value = { ...selected.value, [questionId]: [label] }
  }
}

/** 恢复锚点 = 父骨架（提问/确认都是某个工具调用的一部分） */
function parentSkeletonId(): string | undefined {
  return props.node.parent_id || undefined
}

function submitConfirm(approved: boolean) {
  const anchorId = parentSkeletonId()
  if (!anchorId || !resume) return
  resume({
    targetId: anchorId,
    action: approved ? RESUME_ACTION_APPROVE : RESUME_ACTION_REJECT,
    reason: approved ? undefined : MESSAGE_REJECT_REASON,
    targetSessionId: messageParentSessionId(props.node),
  })
}

/**
 * 「本会话放行」= **先抬阈值，再批准这一次**。
 *
 * 两步是两笔独立的事实，顺序不能反：抬阈值先落定（会话 metadata + store 镜射），
 * 恢复载荷里的 `risk_level` 才读得到新档（`useChatConnection.resume` 从 store 取值）
 * ——否则本次恢复仍按旧阈值判，用户会觉得「点了放行又弹了一次」。
 *
 * 抬阈值失败**不拦批准**：那只是「以后不再问」没生效，本次批准仍然有效；拦住它
 * 等于让一次写失败把用户挡在门外。此时错误由写入口记日志（`useSessionOptionBar`
 * 同一条口径），用户看得见的是「还是问了」。
 *
 * 无人提供这一档（`allowForSession` 为 null，如嵌入式的只读视图）就当它不存在：
 * 按钮在模板里靠 `confirmRisk` 显示，这里再挡一次是防「有按钮没接线」。
 */
async function submitAllowForSession() {
  const risk = confirmRisk.value
  if (!risk) return
  if (allowForSession) {
    try {
      await allowForSession(risk)
    } catch {
      // 抬阈值失败不拦本次批准（理由见函数文档）
    }
  }
  submitConfirm(true)
}

function submitQuestions() {
  const anchorId = parentSkeletonId()
  if (!anchorId || !resume) return
  const answers: MessagePromptAnswer[] = (questionPrompt.value?.questions || []).map((q) => ({
    question_id: q.id,
    selected: selected.value[q.id] || [],
    custom_input: isChosen(q.id, MESSAGE_PROMPT_OTHER) ? customInput.value[q.id] || '' : null,
  }))
  resume({
    targetId: anchorId,
    action: RESUME_ACTION_ANSWER,
    answer: { answers },
    targetSessionId: messageParentSessionId(props.node),
  })
}
</script>

<style scoped>
/* ── 待用户响应（user_prompt 提问 / 工具确认）── */
.user-prompt {
  border: 1px solid var(--color-prompt-border);
  background: var(--color-prompt-bg);
  border-radius: 0.5rem;
  padding: 0.6rem 0.7rem;
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}
.user-prompt.answered {
  opacity: 0.7;
}
.up-question {
  display: flex;
  flex-direction: column;
  gap: 0.3rem;
}
.up-header {
  font-size: 0.78rem;
  font-weight: 600;
  color: var(--color-chip-tool-fg);
}
.up-qtext {
  font-size: 0.82rem;
  color: var(--text-primary);
}
.up-options {
  display: flex;
  flex-direction: column;
  gap: 0.25rem;
}
.up-option {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  padding: 0.3rem 0.45rem;
  border: 1px solid var(--color-option-border);
  border-radius: 0.375rem;
  background: var(--color-option-bg);
  font-size: 0.8rem;
  cursor: pointer;
}
.up-option.active {
  border-color: var(--color-option-active-border);
  background: var(--color-option-active-bg);
}
.up-opt-label {
  font-weight: 500;
  color: var(--text-primary);
}
.up-opt-desc {
  font-size: 0.74rem;
  color: var(--text-muted);
}
.up-option-other {
  gap: 0.35rem;
}
.up-other-input {
  flex: 1;
  border: 1px solid var(--color-option-border);
  border-radius: 0.25rem;
  padding: 0.2rem 0.4rem;
  font-size: 0.78rem;
}
.up-submit {
  align-self: flex-start;
  background: var(--accent);
  color: var(--text-on-accent);
  border: none;
  border-radius: 0.375rem;
  padding: 0.35rem 0.9rem;
  font-size: 0.8rem;
  cursor: pointer;
}
.up-submit:disabled {
  background: #94a3b8;
  cursor: default;
}
.up-confirm {
  display: flex;
  flex-direction: column;
  gap: 0.3rem;
}
.up-confirm-tool {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}
.up-tool-name {
  font-size: 0.85rem;
  font-weight: 600;
  color: var(--text-primary);
  font-family: 'JetBrains Mono', ui-monospace, monospace;
}
.up-risk {
  font-size: 0.68rem;
  padding: 0.05rem 0.4rem;
  border-radius: 62.4375rem;
  background: var(--color-chip-bg);
  color: var(--color-chip-fg);
}
/* 风险等级由动态 :class="'risk-' + level" 生成（risk-high / risk-medium / risk-low） */
.up-risk.risk-high {
  background: var(--color-tag-err-bg);
  color: var(--color-tag-err-fg);
}
.up-risk.risk-medium {
  background: var(--color-tag-warn-bg);
  color: var(--color-tag-warn-fg);
}
.up-confirm-desc {
  font-size: 0.8rem;
  color: var(--text-secondary);
}
.up-args {
  margin: 0;
  padding: 0.5rem 0.6rem;
  background: var(--color-code-bg);
  color: var(--color-code-fg);
  border-radius: 0.5rem;
  font-family: 'JetBrains Mono', ui-monospace, monospace;
  font-size: 0.74rem;
  line-height: 1.5;
  overflow-x: auto;
  max-height: 15rem;
  white-space: pre;
}
.up-confirm-btns {
  display: flex;
  gap: 0.5rem;
}
.up-approve,
.up-allow-session,
.up-reject {
  flex: 1;
  padding: 0.35rem;
  border-radius: 0.375rem;
  font-size: 0.8rem;
  cursor: pointer;
  border: 1px solid transparent;
}
.up-approve {
  background: var(--success-solid);
  color: var(--text-on-accent);
}
/* 「本会话放行」比「批准执行」弱一档：它放开的是一整档风险，比批一次更宽，
   不该和主按钮抢同一种视觉重量（也避免误点成「以后都别再问」）。 */
.up-allow-session {
  background: var(--color-option-bg);
  border-color: var(--color-option-border);
  color: var(--text-primary);
}
.up-approve:disabled,
.up-allow-session:disabled,
.up-reject:disabled {
  cursor: default;
  opacity: 0.7;
}
.up-reject {
  background: var(--color-option-bg);
  border-color: var(--color-error-border);
  color: var(--color-error-fg);
}
</style>
