<!--
  待用户响应渲染器 —— `type = interaction` 的唯一形态（ask_user 提问 / 工具执行确认）

  两种子形态由数据契约判别键 `kind` 分派（`schemas/message_prompt.ts`）：
  · `question`：一个或多个问题，可单选 / 多选，含约定选项 `Other`（勾选后自由输入）；
  · `confirm` ：工具名 + 风险等级 + 可选审批摘要 + 参数 + 描述——始终可仅本次批准，
               服务端列出 `session` / `permanent` 时才显示对应的长期放行。

  授权范围为什么是平级按钮，而不是「批准」加一个「以后别再问」的复选框：
  `once` 记的是「用户批了**这一次**」（`authz.granted`，一次调用一笔）；
  `session` 改的是**会话的执行阈值**（同档及以下的工具在本会话里从此不再问）；
  `permanent` 改的是**当前审批卡绑定沙箱档案里的工具授权上限**，同一档案的后续
  会话共享，撤销在沙箱设置中进行。三笔事实的生命周期分别是「一次调用 / 一个会话 /
  一份沙箱档案」，做成复选框就把后两档藏在一次批准里——用户点的是「批准」，后果却是
  「以后都别再问」。分档也决定了误点的代价：越宽的档，按钮的视觉重量越轻。

  长期范围**只认服务端 `approval_scopes`**：旧卡缺省时只显示 `once`，未知值不猜测。
  永久授权只取当前卡的 `prompt.policy.profile_id`；绑定缺失或畸形时按钮禁用并给出
  刷新 / 重发提示，绝不回落到 `default`，也不把点击静默降级成仅本次批准。

  **已回答态**（`status` 不再是 `waiting_user_action`）不再隐藏表单，而是整体降透明度
  并禁用控件：用户事后回看会话时，仍能看到自己当初选了什么。

  提交都经 `RESUME_KEY` 注入的动作打到**父骨架**（`parent_id`）——提问与确认都是
  某个工具调用的一部分，锚点必须是父容器骨架而不是本节点自身。子会话里的提问还要
  带上 `parent_session_id`，否则恢复会打到父会话。「本会话放行」另走
  `ALLOW_FOR_SESSION_KEY`，「永久放行」另走 `ALLOW_FOREVER_KEY`：它们改的是
  阈值与授权清单，不是这条消息的恢复。
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
                aria-label="其他回答"
                placeholder="请输入…"
                :disabled="isAnswered || !isChosen(q.id, MESSAGE_PROMPT_OTHER)"
              />
            </label>
          </div>
        </div>
        <!-- 每个问题都必须有选择：此前空 answers 也能提交，后端收到的是一次"什么也没答"
             的恢复，用户看到的却是「已提交」——回答被吞了却不报错。 -->
        <p v-if="!isAnswered && unansweredCount > 0" class="up-hint">
          还有 {{ unansweredCount }} 个问题未作答（选了「其他」的还需填写内容）
        </p>
        <button
          class="up-submit"
          :disabled="isAnswered || !canSubmitQuestions"
          @click="submitQuestions"
        >
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
              >{{ riskLabel }}</span
            >
          </div>
          <template v-if="confirmApproval">
            <div class="up-confirm-action">{{ confirmApproval.action }}</div>
            <div class="up-confirm-target">
              <span>{{ confirmApproval.target_label || '目标' }}</span>
              <code>{{ approvalTarget }}</code>
            </div>
            <div class="up-confirm-effect">{{ effectLabel }}</div>
          </template>
          <div v-else class="up-confirm-desc">{{ confirmPrompt.description }}</div>
          <details v-if="confirmPrompt.args !== undefined" class="up-args">
            <summary>技术详情</summary>
            <pre>{{ JSON.stringify(confirmPrompt.args, null, 2) }}</pre>
          </details>
        </div>
        <div class="up-confirm-btns">
          <button class="up-approve" :disabled="isAnswered" @click="submitConfirm(true)">
            {{ isAnswered ? '已处理' : '仅本次批准' }}
          </button>
          <!-- 长期放行只在服务端明确列出 scope 且本次有可抬风险档时出现。 -->
          <button
            v-if="showSessionScope"
            class="up-allow-session"
            :disabled="isAnswered"
            @click="submitAllowForSession"
          >
            {{ isAnswered ? '已处理' : '本会话同档免问' }}
          </button>
          <!-- 永久授权按当前审批卡绑定的沙箱档案保存；无效绑定必须禁用，不能静默
               回落 default 或降级为仅本次批准。 -->
          <button
            v-if="showPermanentScope"
            class="up-allow-forever"
            :disabled="isAnswered || !!permanentBlockReason"
            @click="submitAllowForever"
          >
            {{ isAnswered ? '已处理' : '此工具永久免问同档' }}
          </button>
          <button class="up-reject" :disabled="isAnswered" @click="submitConfirm(false)">
            {{ isAnswered ? '已处理' : '拒绝' }}
          </button>
        </div>
        <p v-if="showPermanentScope && permanentBlockReason && !isAnswered" class="up-hint">
          {{ permanentBlockReason }}
        </p>
        <!-- 后果优先：按钮文字只说做什么，作用域在这里一次说清。 -->
        <p v-if="approvalReason" class="up-approval-reason">{{ approvalReason }}</p>
        <p class="up-scope-hint">
          仅批准只对这一次调用生效，不改变后续权限<template v-if="showSessionScope">；本会话同档免问仅持续到当前会话结束</template><template v-if="showPermanentScope">；此工具永久免问同档会保存到当前审批卡绑定的沙箱档案<template v-if="!confirmPolicy">（当前绑定无效，暂不可保存）</template>，跨会话生效，可在沙箱设置中撤销</template>。
        </p>
        <!-- 审批控制的是「让不让做」，不是「在隔离环境里做」；未选档使用 default，
             显式 none 才不限制，但两者都不提供进程、容器或网络隔离。 -->
        <p class="up-sandbox-hint">
          注意：沙箱审批不等于隔离，工具仍直接在选定的工作目录中执行；本产品不提供进程、容器或网络隔离。
        </p>
      </template>
    </div>

    <template v-for="name in ($slots.subtree ? ['subtree'] : [])" :key="name" #[name]>
      <slot name="subtree" />
    </template>
  </NodeShell>
</template>

<script setup lang="ts">
import { computed, inject, ref } from 'vue'
import {
  APPROVAL_SCOPE_PERMANENT,
  APPROVAL_SCOPE_SESSION,
  RESUME_ACTION_ANSWER,
  RESUME_ACTION_APPROVE,
  RESUME_ACTION_REJECT,
  approvalPolicyBindingOf,
  approvalReasonOf,
  approvalScopesOf,
  type ChatNode,
} from '@/schemas/chat_node'
import {
  MESSAGE_PROMPT_OTHER,
  MESSAGE_REJECT_REASON,
  type MessagePromptAnswer,
  type MessagePromptEffect,
  type MessagePromptQuestion,
} from '@/schemas/message_prompt'
import { RESUME_KEY, ALLOW_FOR_SESSION_KEY, ALLOW_FOREVER_KEY } from '@/composables/useChatConnection'
import { useToast } from '@/composables/useToast'
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
const { showToast } = useToast()
/** 「本会话放行」（审批卡第二档）；无人提供时按「没有这一档」处理 */
const allowForSession = inject(ALLOW_FOR_SESSION_KEY, null)
/** 「永久放行」（审批卡第三档）；同样可缺席（嵌入式只读视图） */
const allowForever = inject(ALLOW_FOREVER_KEY, null)

const prompt = computed(() => promptOf(props.node))
const questionPrompt = computed(() =>
  prompt.value?.kind === 'question' ? prompt.value : null,
)
const confirmPrompt = computed(() => (prompt.value?.kind === 'confirm' ? prompt.value : null))
const confirmApproval = computed(() => confirmPrompt.value?.approval)

const effectLabels: Record<MessagePromptEffect, string> = {
  file_modify: '会修改文件内容',
  file_delete: '会删除文件或目录',
  network_send: '会向外部服务发送数据',
  system_execute: '会在本机执行命令，具体影响取决于命令内容',
  other: '可能改变当前状态',
}
const riskLabels: Record<string, string> = {
  low: '低风险',
  medium: '中风险',
  high: '高风险',
}
const riskLabel = computed(() => riskLabels[confirmPrompt.value?.risk_level || ''] || '')
const effectLabel = computed(() => {
  const effect = confirmApproval.value?.effect
  return effect ? effectLabels[effect] : ''
})
const approvalTarget = computed(() => {
  const target = confirmApproval.value?.target
  if (typeof target === 'string') return target
  return target === undefined ? '' : JSON.stringify(target, null, 2)
})

/**
 * 本次调用可读的风险档（长期放行两档的必要条件）。
 *
 * 没有档位就没有「抬到哪一档」可言。校验走契约层词表，不在组件里手写风险字面量。
 */
const confirmRisk = computed<SessionRiskLevel | null>(() => {
  const raw = confirmPrompt.value?.risk_level
  return isSessionRiskLevel(raw) ? raw : null
})

/** 服务端明确列出的范围；旧卡 / 未知值不会补出任何长期授权。 */
const approvalScopes = computed(() => approvalScopesOf(confirmPrompt.value?.approval_scopes))
const approvalReason = computed(() => approvalReasonOf(confirmPrompt.value?.approval_reason))
const showSessionScope = computed(
  () => !!confirmRisk.value && approvalScopes.value.includes(APPROVAL_SCOPE_SESSION),
)
const showPermanentScope = computed(
  () => !!confirmRisk.value && approvalScopes.value.includes(APPROVAL_SCOPE_PERMANENT),
)

/** 当前卡绑定的沙箱档案；缺失 / 畸形时返回 null，绝不回落 default。 */
const confirmPolicy = computed(() => approvalPolicyBindingOf(confirmPrompt.value?.policy))

const PERMANENT_POLICY_HINT =
  '当前审批卡缺少有效的沙箱档案绑定，无法保存永久授权。请刷新审批卡或重新发起工具调用后再试；仍可仅本次批准。'
const permanentBlockReason = computed(() => {
  if (!showPermanentScope.value || isAnswered.value) return ''
  if (!confirmPolicy.value) return PERMANENT_POLICY_HINT
  if (!confirmPrompt.value?.tool_name) {
    return '当前审批卡缺少工具名，无法保存永久授权。请刷新审批卡或重新发起工具调用后再试；仍可仅本次批准。'
  }
  if (!allowForever) {
    return '当前视图未连接永久授权入口，无法保存永久授权。请回到会话审批卡重试；仍可仅本次批准。'
  }
  return ''
})

/** 已回答：状态不再是 waiting_user_action → 控件禁用并改为已提交态 */
const isAnswered = computed(() => !isWaitingStatus(props.facets.status))

/** 表单本地状态：questionId → 选中的 label 列表；`Other` 勾选时的自由文本 */
const selected = ref<Record<string, string[]>>({})
const customInput = ref<Record<string, string>>({})

/**
 * 尚未作答的问题数。
 *
 * 已回答态（控件禁用）不计；未回答时每个问题要至少选中一项，且勾了 `Other`
 * 必须填上内容——只勾不填等于交给后端一个空字符串，与「没答」同义。
 */
const unansweredCount = computed(() => {
  if (isAnswered.value) return 0
  return (questionPrompt.value?.questions ?? []).filter((q) => {
    const sel = selected.value[q.id] || []
    if (sel.length === 0) return true
    if (sel.includes(MESSAGE_PROMPT_OTHER) && !(customInput.value[q.id] || '').trim()) return true
    return false
  }).length
})
const canSubmitQuestions = computed(() => unansweredCount.value === 0)

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
 * 按钮在模板里靠 `showSessionScope` 显示，这里再挡一次是防「有按钮没接线」。
 */
async function submitAllowForSession() {
  if (!showSessionScope.value) return
  const risk = confirmRisk.value
  if (!risk || !allowForSession) return
  try {
    await allowForSession(risk)
  } catch (e) {
    // 抬阈值失败不拦本次批准（理由见函数文档），但必须让用户知道
    // 「本会话放行」没写进去——否则界面照常推进，用户误以为以后都不问了。
    showToast('error', `本次已批准，但「本会话放行」未生效：${e instanceof Error ? e.message : String(e)}`)
  }
  submitConfirm(true)
}

/**
 * 「永久放行」= 先把**这个工具在当前卡绑定档案里**的上限抬到本次风险档，再批准这一次。
 *
 * 与会话放行同一条两步纪律、同一个顺序理由：抬上限必须在恢复之前落定，否则后端的
 * 授权级联读到的还是旧上限。档案名只取 `prompt.policy.profile_id`，前端不选择、不翻译，
 * 也不从会话设置补 `default`；`revision` 由读侧守卫校验，`sandbox/grant` 请求只发
 * `{profile_id, tool, risk}`。
 *
 * 绑定缺失 / 畸形时**整次动作停止**：既不写授权，也不把「永久放行」静默降级成一次批准。
 * 写入已获服务端确认会失败时仍批准本次，并明确告知长期授权未生效。
 */
async function submitAllowForever() {
  const risk = confirmRisk.value
  const tool = confirmPrompt.value?.tool_name
  const policy = confirmPolicy.value
  if (!showPermanentScope.value || !risk || !tool) return
  if (!policy) {
    showToast('error', PERMANENT_POLICY_HINT)
    return
  }
  if (!allowForever) return
  try {
    await allowForever(policy.profile_id, tool, risk)
  } catch (e) {
    // 写失败不拦本次批准，但这是跨会话授权：必须如实告知没写进去，
    // 用户不能以为「永久放行」已生效。
    showToast('error', `本次已批准，但「永久放行」未生效：${e instanceof Error ? e.message : String(e)}`)
  }
  submitConfirm(true)
}

function submitQuestions() {
  const anchorId = parentSkeletonId()
  if (!anchorId || !resume) return
  // 兜底：按钮已经禁用，但 Enter 提交等路径不能绕过同一条判据
  if (!canSubmitQuestions.value) return
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
  background: var(--surface-selected);
  color: var(--text-muted);
  cursor: default;
}
.up-hint {
  margin: 0;
  font-size: 0.76rem;
  color: var(--color-tag-warn-fg);
}
.up-scope-hint {
  margin: 0;
  font-size: 0.72rem;
  line-height: 1.5;
  color: var(--text-muted);
}
.up-approval-reason {
  margin: 0;
  font-size: 0.76rem;
  line-height: 1.5;
  color: var(--text-primary);
}
.up-sandbox-hint {
  margin: 0;
  font-size: 0.72rem;
  line-height: 1.5;
  color: var(--color-tag-warn-fg);
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
.up-confirm-action {
  font-size: 0.92rem;
  font-weight: 600;
  color: var(--text-primary);
}
.up-confirm-target {
  display: flex;
  flex-wrap: wrap;
  gap: 0.35rem;
  align-items: baseline;
  font-size: 0.8rem;
  color: var(--text-secondary);
}
.up-confirm-target code {
  padding: 0.1rem 0.3rem;
  border-radius: 0.25rem;
  background: var(--color-code-bg);
  color: var(--color-code-fg);
  overflow-wrap: anywhere;
  white-space: pre-wrap;
}
.up-confirm-effect {
  font-size: 0.76rem;
  color: var(--color-tag-warn-fg);
}
.up-args {
  border-radius: 0.5rem;
  background: var(--color-code-bg);
  color: var(--color-code-fg);
  font-size: 0.74rem;
  overflow: hidden;
}
.up-args summary {
  padding: 0.35rem 0.6rem;
  color: var(--text-secondary);
  cursor: pointer;
}
.up-args pre {
  margin: 0;
  padding: 0 0.6rem 0.5rem;
  font-family: 'JetBrains Mono', ui-monospace, monospace;
  line-height: 1.5;
  overflow-x: auto;
  max-height: 15rem;
  white-space: pre;
}
.up-confirm-btns {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
}
.up-approve,
.up-allow-session,
.up-allow-forever,
.up-reject {
  flex: 1;
  min-height: 2rem;
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
/* 「本会话放行」与「永久放行」都比「批准执行」弱一档：它们放开的是一整档 / 一个
   工具的长期免问，比批一次更宽，不该和主按钮抢同一种视觉重量（也避免误点）。
   两档之间不再分视觉层级：它们的作用域不同（会话 / 跨会话），不是一个比另一个
   「更危险」的简单序——把宽度压成灰度只会诱导用户点那个看起来更轻的。 */
.up-allow-session,
.up-allow-forever {
  background: var(--color-option-bg);
  border-color: var(--color-option-border);
  color: var(--text-primary);
}
.up-approve:disabled,
.up-allow-session:disabled,
.up-allow-forever:disabled,
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
