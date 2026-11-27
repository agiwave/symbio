<template>
  <div class="model-chat-panel">
    <!-- 编辑单条消息的浮层。外壳走 `BaseModal`：ESC 可关、焦点被陷阱锁在框内、
         遮罩底色与层级走 token（原实现写死 `z-index: 100` + `rgba(0,0,0,0.45)`，
         层级低于其它浮层且不跟随主题）。 -->
    <BaseModal :visible="!!editing" panel-class="edit-box" :dismissable="!savingEdit" @close="closeEdit">
      <template v-if="editing">
        <div class="edit-title">{{ editing.retryAfterSave ? '修改并重试' : '编辑消息' }}</div>
        <textarea
          v-model="editing.content"
          class="edit-area"
          aria-label="编辑消息内容"
          :disabled="savingEdit"
          :placeholder="editing.isJson ? 'JSON 内容' : '消息内容'"
        />
        <div v-if="editing.attachments.length > 0" class="edit-attachments">
          <div v-for="(attachment, index) in editing.attachments" :key="index" class="edit-attachment">
            <img :src="attachment.image_url.url" :alt="`附件 ${index + 1}`" />
            <button
              type="button"
              class="edit-attachment-remove"
              :title="`移除附件 ${index + 1}`"
              :aria-label="`移除附件 ${index + 1}`"
              @click="removeEditAttachment(index)"
            >×</button>
          </div>
        </div>
        <p v-if="editing.retryAfterSave" class="edit-retry-note">
          保存后会保留旧对话并另开一支；已执行的外部操作不会撤销。
        </p>
        <div class="edit-btns">
          <button class="edit-save" :disabled="savingEdit" @click="saveEdit">
            {{ savingEdit ? '处理中…' : editing.retryAfterSave ? '保存并重试' : '保存' }}
          </button>
          <button class="edit-cancel" :disabled="savingEdit" @click="closeEdit">取消</button>
        </div>
      </template>
    </BaseModal>

    <!-- 删除确认：删除一条消息会连同其后所有消息一起截断（store 的级联口径与后端
         同源），这是一次不可撤销的写操作，不能一点就删。单条也确认。 -->
    <ConfirmDialog
      :visible="!!pendingDelete"
      title="删除消息"
      :message="pendingDeleteMessage"
      :confirm-text="pendingDelete && pendingDelete.count > 1 ? '删除这些消息' : '删除'"
      danger
      icon="🗑"
      icon-kind="danger"
      @confirm="confirmDelete"
      @cancel="pendingDelete = null"
    />

    <!-- 消息区：恒为单列——一个会话线程一个显示区域（呈现合流定案：
         会话里没有第二条线程，收件箱 FIFO + 单写者 + 单转写只允许一条线；
         完整转发直排：正文 / 思考 / 工具调用与轮次过程在同一列按时间轴展开）。 -->
    <div class="chat-body">
      <section class="chat-column">
        <div class="column-body" ref="messagesRef" @scroll="onScroll">
          <div v-if="messageTree.length === 0" class="empty-chat">
            <p>开始与 AI 助手对话</p>
          </div>

          <!-- 渲染窗口的**显式入口**：更早的轮次没铺出来时，这里给一条按钮。

               为什么不自动续拉（滚到顶就加载）：那样「往上翻」会变成一场无限加载，
               用户不知道有没有头；一条按钮同时回答「还有更早的」和「还剩多少条」。
               条数说的是**根节点**（一轮响应的根），不是消息条数——窗口按根切，
               判据见 `composables/useTranscriptWindow`。 -->
          <div v-if="hiddenCount > 0" class="earlier-row">
            <button class="earlier-btn" type="button" @click="handleShowEarlier">
              显示更早的消息（还有 {{ hiddenCount }} 条）
            </button>
          </div>

          <MessageNode
            v-for="node in renderedRoots"
            :key="node.id"
            :node="node"
            :depth="0"
            @retry="handleRetry"
            @delete="handleDelete"
            @edit="handleEdit"
          />

          <!-- 等待提示的**第二个来源**：会话在跑，而流里没有任何在途节点。
               常规情形下根级骨架节点已到，等待态由 SkeletonNode 挂在骨架容器里；
               这条只在「骨架节点的变更还没到 / 丢了一次」时兜底——否则用户点完
               发送会看到「什么都没发生」。两者互斥，不会同时出现两条。 -->
          <SkeletonPending v-if="showTyping" />

          <!-- 停止反馈与等待骨架分开：错误走 toast 的 assertive 通道，停止过程 / 成功
               只走持久 polite 区域，且不订阅消息状态帧，避免每个终态帧重复播报。 -->
          <div
            v-if="stopNotice"
            class="stop-notice"
            :class="`stop-notice--${stopNotice.tone}`"
            aria-hidden="true"
          >{{ stopNotice.text }}</div>
          <div class="stop-live" role="status" aria-live="polite" aria-atomic="true">
            {{ stopNotice?.text }}
          </div>

          <!-- 排队中的用户消息（「待发送」气泡）。

               发送方不止前端自己：CLI / telegram / 心跳 / 别的进程都往同一个收件箱
               写（`<会话地址>/inbox/<消息 id>`）。队列里的消息要等会话空闲才被
               取出、落库、发权威帧——那之前转写里没有它，界面上应当显示「它在排队」，
               而不是「什么都没发生」。落库后同一条消息由转写帧接手（同一个 id），
               本块随之消失，不会出现两份。 -->
          <InboxPending :entries="inboxEntries" />

          <!-- 会话级错误条：本地传输错误或现代失败轮由同一份转写派生；
               有节点内恢复入口时不重复显示。位置在流内末尾，重试入口紧跟本轮结果。 -->
          <MessageErrorBox
            v-if="sessionError"
            :text="sessionError"
            :retryable="!!lastFailedSend || !!roundRetryAnchor"
            :modifiable="!!roundRetryAnchor && !lastFailedSend"
            :disabled="sessionRetryBusy"
            :dismissible="canDismissSessionError"
            variant="skeleton"
            @retry="handleSessionRetry"
            @modify="handleSessionModify"
            @dismiss="sessionsStore.setSessionError(props.sessionId, null)"
          />
        </div>

        <!-- 上翻阅读时的回到最新入口：流还在尾部增长，没有它只能自己滚回底部。
             位置在消息区右下、输入区之上，不遮挡正文；只是「回到最新」，不改变阅读位置记录。 -->
        <button
          v-if="!isUserAtBottom"
          class="scroll-bottom"
          type="button"
          title="回到最新消息"
          aria-label="回到最新消息"
          @click="jumpToBottom"
        >
          ↓ 最新
        </button>
      </section>
    </div>

    <!-- 输入控制区域：水位 + 输入框 + 选项行由 ChatComposer 唯一装配（不再在此各拼一半） -->
    <div class="chat-controls">
      <ChatComposer
        ref="composerRef"
        v-model="inputText"
        v-model:attached-images="attachedImages"
        :is-loading="isLoading || isStopping"
        :session-id="props.sessionId"
        :definition="optionDefinition"
        :values="optionValues"
        :scope="optionScope"
        :meter="optionItem?.context_watermark"
        @submit="handleSendOrAbort"
      />
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, nextTick, watch, computed, onMounted, onBeforeUnmount, provide } from 'vue'
import { useChatConnection, RESUME_KEY, SKELETON_LAZY_KEY, SWITCH_BRANCH_KEY, ALLOW_FOR_SESSION_KEY, ALLOW_FOREVER_KEY, type ResumePayload } from '@/composables/useChatConnection'
import { useChatScroll } from '@/composables/useChatScroll'
import { useTranscriptWindow, TRANSCRIPT_WINDOW_INITIAL } from '@/composables/useTranscriptWindow'
import {
  type ChatNode,
  type MessageContent,
  type ContentPart,
  PRINCIPAL_KIND_PERSON,
  PRINCIPAL_USER,
  messageTextOf,
  isUnsettledNodeStatus,
} from '@/schemas/chat_node'
import type { ImageAttachment } from '@/types'
import type { DetailDefinition } from '@/schemas/vdfs'
import type { SessionRiskLevel } from '@/schemas/session_meta'
import { logger } from '@/utils/logger'
import { useHotkeys } from '@/composables/useHotkeys'
import { useSessionsStore } from '@/stores/sessions'
import { grantToolForever } from '@/services/sessionGrant'
import { useSessionUiStateStore } from '@/stores/sessionUiState'
import { needsTypingRow } from '@/stores/sessionLive'
import { dropUnselectedBranches } from '@/stores/sessionTranscript'
import { CHAT_ROLE_USER } from '@/schemas/chat_node'
// 消息级判定与业务规则全部来自 registry（本组件不解释消息词表，也不读后端 meta 字段）
import {
  latestRoundRetryAnchorOf,
  messageRetryTargetOf,
  messageRoleOf,
  messageStatusOf,
} from '@/registry/messageTypes'
import { messageErrorTextOf } from '@/composables/useMessageContent'

import MessageNode from './MessageNode.vue'
import MessageErrorBox from './message/MessageErrorBox.vue'
import SkeletonPending from './message/SkeletonPending.vue'
import InboxPending from './message/InboxPending.vue'
import ChatComposer from './chat/ChatComposer.vue'
import BaseModal from './common/BaseModal.vue'
import ConfirmDialog from './common/ConfirmDialog.vue'
import { useToast } from '@/composables/useToast'

// Props（多会话缩略窗口架构下，ModelChatPanel 只接收会话身份）
const props = defineProps<{
  /** 会话 id（**地址末段**）：store 里所有以会话为键的状态（消息 / 运行态）都用它 */
  sessionId: string
  /**
   * 会话**完整地址** = `<挂载目录>/<id>`——跨进程寻址的唯一依据。
   *
   * 与 `sessionId` 并存而不是二选一：前者是 store 的键（字典查表），后者是
   * **地址**（发言写哪个 inbox、中止打在哪个节点、恢复落在哪条消息）。两个空间
   * 里可以有同名 id，只带 id 就等于把「住在哪个空间」丢掉。
   */
  sessionAddr: string
  onSendComplete?: () => void
  /** 内部 useChatConnection 的 isLoading 变化回调（用于跨组件状态同步） */
  onLoadingChange?: (loading: boolean) => void
}>()

// --- 状态管理 ---
const messagesRef = ref<HTMLElement | null>(null)
/** 输入区（ChatComposer）：发送后复位高度、排队首条消息回填文本与附件 */
const composerRef = ref<{
  resetHeight: () => void
  transferImages: (images: ImageAttachment[]) => void
  restoreImages: (images: ImageAttachment[]) => boolean
} | null>(null)

// 会话级纯 UI 状态（按 sessionId 记忆）：本面板是按地址**重挂载**的
// （`ChatMainPanel` 的 `:key`），切走再切回时组件是全新的——草稿与滚动位置
// 因此不能存在组件里，否则「切一下再回来，刚才写的东西没了」。
const uiState = useSessionUiStateStore()

/** 输入框初值 = 上次离开时这一会话留下的草稿（无记录 = 空串） */
const inputText = ref(uiState.draftOf(props.sessionId))

// 草稿随输入实时落进按会话的状态（空串 = 清除）。
// 不在这里做防抖：键盘输入本来就该即时生效，而写入是一次浅对象替换。
watch(inputText, (text) => uiState.setDraft(props.sessionId, text))
const attachedImages = ref<ImageAttachment[]>([])
/** 已从输入区接手、但发送尚未有终态的 URL；卸载时也由面板收尾。 */
const claimedImageUrls = new Set<string>()
let disposed = false

function claimImages(images: ImageAttachment[]): void {
  for (const image of images) {
    if (image.thumbnailUrl) claimedImageUrls.add(image.thumbnailUrl)
  }
  composerRef.value?.transferImages(images)
}

function releaseClaimedImages(images?: ImageAttachment[]): void {
  const urls = images
    ? images.map((image) => image.thumbnailUrl).filter((url): url is string => !!url)
    : [...claimedImageUrls]
  for (const url of urls) {
    if (!claimedImageUrls.delete(url)) continue
    URL.revokeObjectURL(url)
  }
}

/** 失败时优先还给仍挂载的输入区；切换 / 卸载后没有接收方，就由面板释放。 */
function returnClaimedImages(images: ImageAttachment[]): void {
  const urls = images
    .map((image) => image.thumbnailUrl)
    .filter((url): url is string => !!url && claimedImageUrls.delete(url))
  if (!disposed && composerRef.value?.restoreImages(images)) return
  for (const url of urls) URL.revokeObjectURL(url)
}

// 编辑单条消息的浮层状态。附件保留 wire 中的 image_url 分片，保存时原样带回；
// 删除恢复时再转成输入区已有的 ImageAttachment 形状，不改消息 payload 协议。
type ImageContentPart = Extract<ContentPart, { type: 'image_url' }>
const editing = ref<{
  id: string
  content: string
  isJson: boolean
  attachments: ImageContentPart[]
  retryAfterSave: boolean
} | null>(null)
const savingEdit = ref(false)
const retryingFailedSend = ref(false)
const roundResumePendingId = ref<string | null>(null)
const roundResumeVersion = ref(0)

function imagePartsOf(content: MessageContent | undefined): ImageContentPart[] {
  if (!Array.isArray(content)) return []
  return content.filter((part): part is ImageContentPart => part.type === 'image_url')
}

/** 现有协议把图片存为 data URL；删除回填只负责这一种真实可恢复的形状。 */
function restoreImagesOf(content: MessageContent | undefined): {
  images: ImageAttachment[]
  unsupported: number
} {
  const images: ImageAttachment[] = []
  let unsupported = 0
  imagePartsOf(content).forEach((part, index) => {
    const match = /^data:([^;,]+);base64,([\s\S]*)$/.exec(part.image_url.url)
    if (!match) {
      unsupported += 1
      return
    }
    images.push({
      base64: match[2],
      mimeType: match[1],
      fileName: `图片 ${index + 1}`,
      thumbnailUrl: part.image_url.url,
    })
  })
  return { images, unsupported }
}

function removeEditAttachment(index: number) {
  editing.value?.attachments.splice(index, 1)
}

// --- Composables ---
const { isUserAtBottom, scrollToBottom, scrollToGap, gapFromBottom, smartScroll, handleScroll } = useChatScroll(messagesRef)

/**
 * 滚动事件：先按 `useChatScroll` 的规则重算「是否贴底」，再把位置记进
 * **按会话**的 UI 状态（切回时还原）。
 *
 * 记的是**离底的距离**而不是 `scrollTop`——本面板的 DOM 起点会随渲染窗口变，
 * 从顶部量的数在下次挂载对不上同一条消息（换算只有一份实现：`gapFromBottom`）。
 *
 * 两个职责合成一个 handler，是因为 DOM 上只能挂一个 `@scroll`；拆成两个监听
 * 只会让「哪次滚动被记下」变得不确定。
 */
function onScroll() {
  handleScroll()
  const gap = gapFromBottom()
  if (gap !== null) uiState.setScrollGap(props.sessionId, gap)
}

const chat = useChatConnection({
  sessionId: props.sessionId,
  sessionAddr: props.sessionAddr,
  onSendComplete: props.onSendComplete,
})

// 向下级组件（MessageNode 内联交互卡提问 / 失败工具骨架重试/补充）提供
// 会话恢复接口（retry_subtree/retry/approve/reject/supply/answer 统一入口）。
// target_id 使用后端声明的用户轮锚、工具骨架或其它稳定节点；子会话工具调用的
// 恢复路由仍由 targetSessionId 指回子会话。会话参数由后端按 session.metadata 解析。
provide(RESUME_KEY, (payload: ResumePayload) => chat.resume(payload))

// 骨架惰性重取面（[] 前端半）：折叠态变化 → 释放 / 重取子树。与 RESUME_KEY 同一条
// 理由用注入而非 props——相遇点（NodeShell 的折叠开关）在组件树深层，而「哪个会话」
// 只有这里的 useChatConnection 闭包知道。
provide(SKELETON_LAZY_KEY, chat.skeletonLazy)

// 分支切换面：切换器住在 `NodeShell`（所有渲染器共用的外壳），而「哪个会话、哪个地址」
// 只有这里的 useChatConnection 闭包知道——与 RESUME_KEY / SKELETON_LAZY_KEY 同一条理由。
provide(SWITCH_BRANCH_KEY, (mid: string, to: string) => {
  void chat.switchBranch(mid, to)
})


const { isLoading, isStopping, stopNotice } = chat
const messageTree = chat.messageTree

/**
 * 渲染窗口：首屏只铺最近的若干条根节点，更早的按需展开。
 *
 * 转写是整份装载的（权威副本在存储），本窗口**不改数据**、只改这一帧交出去多少
 * 棵子树；为什么按根节点切、为什么起点只往左移都在 `useTranscriptWindow` 的模块头。
 * 上次的条数按会话还原——它与离底的距离是一对，只还原位置不还原窗口，
 * 往上翻过的位置会因为内容不够而落到窗口顶部。
 */
const transcriptWindow = useTranscriptWindow(
  messageTree,
  uiState.windowSizeOf(props.sessionId) ?? TRANSCRIPT_WINDOW_INITIAL,
)
const { renderedRoots, hiddenCount } = transcriptWindow

/**
 * 展开更早的轮次，并**把视线留在原来那一屏上**。
 *
 * 往上铺内容等于在视口上方插入高度，浏览器仍按原 `scrollTop` 显示——于是内容整体
 * 下移、用户看到的东西被推走。补上这次插入的高度差（`新高度 - 旧高度`）才是
 * 「接着往下看」。差量在展开**前**量、在 DOM 更新**后**补，顺序不可颠倒。
 *
 * 一条都没铺出来（已经到顶）时不动滚动位置：那次补偿挪的是凭空的高度 0，
 * 表现就是「按钮还在，点一下界面抖一下」。
 */
async function handleShowEarlier() {
  const el = messagesRef.value
  const beforeHeight = el?.scrollHeight ?? 0
  const beforeTop = el?.scrollTop ?? 0
  const added = transcriptWindow.showEarlier()
  if (added === 0) return
  uiState.setWindowSize(props.sessionId, transcriptWindow.renderedCount.value)
  await nextTick()
  if (!el) return
  el.scrollTop = beforeTop + (el.scrollHeight - beforeHeight)
}

// Esc 停止生成。绑定声明在这里而不是外壳里，因为「这一轮还在跑吗」是本组件
// 经 `useChatConnection` 才有的状态；全局表只负责判键与让路。
// `allowInInput` 是那条守卫必须的例外：要按停止的时候，焦点正落在输入框里。
useHotkeys('model-chat-panel', [
  {
    id: 'abort',
    keys: 'esc',
    group: '会话',
    label: '停止生成',
    allowInInput: true,
    when: () => isLoading.value || isStopping.value,
    run: () => void chat.abort(),
  },
])

// 全局 store 引用：错误状态兜底 / 消息计数
const sessionsStore = useSessionsStore()

// 「本会话放行」：审批卡上的另一档（与「批准执行」并列）。它只做一件事——把
// **本会话**的执行阈值抬到这次调用的风险档，与 `resume(approve)` 是两笔独立的
// 事实（见 `ALLOW_FOR_SESSION_KEY` 的文档）。会话 id 取自 props：本面板就是
// 「一个会话的视图」，不让渲染器传 id——那会出现「两个来源」，而错的那一个
// 恰好是子会话 id。
provide(ALLOW_FOR_SESSION_KEY, async (risk: SessionRiskLevel) => {
  await sessionsStore.raiseSessionRiskLevel(props.sessionId, risk)
})

// 「永久放行」：把某个工具的授权上限抬到这次的风险档（跨会话生效，存在会话插件的
// 全局配置里）。与上一档的分工见 `ALLOW_FOREVER_KEY` 的文档：一个改本会话的阈值，
// 一个改这份部署的授权清单。`tool` 是能力名，由审批卡原样回传。
provide(ALLOW_FOREVER_KEY, async (tool: string, risk: SessionRiskLevel) => {
  await grantToolForever(props.sessionId, tool, risk)
})

// --- 会话选项栏的三个入参（定义 / 值 / 条件作用域） ---
//
// 本面板与会话树**解耦**（只有 `sessionId`），拿不到会话节点；而选项的**定义**
// 随节点下发（`node.schema`）。两端在 store 的会话清单里相遇：清单来自
// `<根>/session` 的 `vdfs/list`，每项都带着自己的 `schema` 与 `metadata`
// （见 `services/session.listSessions`）。于是这里零额外请求就能拿全三样东西
// —— 上下文水位同理（`context_watermark` 也是那一项的节点属性），共用 `optionItem`。
const optionItem = computed(() => sessionsStore.list.find((s) => s.id === props.sessionId) ?? null)

/** 选项定义（后端产出，前端不解释内容） */
const optionDefinition = computed<DetailDefinition | null>(
  () => (optionItem.value?.schema as DetailDefinition | undefined) ?? null
)

/** 当前字段值 = 会话 metadata（键 = 定义里的字段 key） */
const optionValues = computed<Record<string, unknown>>(
  () => (optionItem.value?.metadata ?? {}) as Record<string, unknown>,
)

/** 条件作用域里的节点级事实（如「已有历史」= `message_count ≠ 0`） */
const optionScope = computed<Record<string, unknown>>(() => ({
  message_count: optionItem.value?.message_count,
}))

// --- 计算属性 ---
  // 会话级错误条（"错误是状态、不是节点"的兜底展示）。
  //
  // 两个来源：会话节点的 `attributes.error`（服务端权威）与 send / resume 的本地乐观
  // 错误。已有可恢复动作的失败节点时由节点承载；现代失败轮只有 body 节点，
  // 因此从同一份转写补出流内错误条，避免“有失败事实但没有重试入口”。
  //
  // 呈现位置是**流尾**（模板里紧跟消息列表，与「等待骨架」同排），不是页面顶部：
  // 它是「这一轮的结果」，属于会话流的时间轴。
  //
  // 这个判定放在**渲染时**（从节点表算出）而不是事件到达时：事件时判定隐含
  // "错误事件到达的那一刻恰好能看到失败节点"，而两条通道的先后无法保证。
  // 从节点表派生则没有这个问题——同一份节点表必然得到同一个结论
  // （`018-node-state-streaming.md` §5.2「派生是纯函数」）。
  const roundRetryAnchor = computed(() =>
    latestRoundRetryAnchorOf(dropUnselectedBranches(sessionsStore.getSessionMessages(props.sessionId))),
  )
  const sessionRetryBusy = computed(() =>
    retryingFailedSend.value
    || (!!roundRetryAnchor.value && roundResumePendingId.value === roundRetryAnchor.value.id),
  )
  const canDismissSessionError = computed(() =>
    !roundRetryAnchor.value && !!sessionsStore.getSessionError(props.sessionId),
  )
  watch(roundRetryAnchor, (anchor) => {
    roundResumeVersion.value += 1
    if (roundResumePendingId.value && anchor?.id !== roundResumePendingId.value) {
      roundResumePendingId.value = null
    }
  })
  const sessionError = computed(() => {
    const err = sessionsStore.getSessionError(props.sessionId)
    const messages = dropUnselectedBranches(sessionsStore.getSessionMessages(props.sessionId))
    const anchor = roundRetryAnchor.value
    const roundMessages = anchor ? messages.slice(messages.indexOf(anchor) + 1) : messages
    const hasInlineRetry = roundMessages.some((message) =>
      messageRoleOf(message) !== CHAT_ROLE_USER
      && isUnsettledNodeStatus(messageStatusOf(message))
      && messageRetryTargetOf(message, messages) !== null,
    )
    if (hasInlineRetry) return null
    if (err) return err
    if (!anchor) return null
    const failure = [...roundMessages].reverse().find((message) =>
      messageRoleOf(message) !== CHAT_ROLE_USER
      && isUnsettledNodeStatus(messageStatusOf(message)),
    )
    return failure ? messageErrorTextOf(failure) : '这一轮未完成，请重试。'
  })

  /**
   * 末尾等待提示（判定在 `stores/sessionLive.needsTypingRow`）。
   *
   * 会话节点说「在跑」（含 send 后的乐观置位）而流里没有任何在途节点
   * ⇒ 骨架节点的变更还没到，此处补一条骨架。
   *
   * ## 它在**发出后那一瞬**是常态，不是兜底
   *
   * 发言只是往会话收件箱写一条（见 `useChatConnection.send`），消息要等后端空闲时
   * 取出、落库、才发权威帧。因此"已置 working 而流里还没有自己的消息"是**预期**
   * 状态——用户看到的是"已发出，等待处理"。随后骨架节点（或用户消息帧本身）
   * 到达，本行自然消失。
   */
  const showTyping = computed(() =>
    needsTypingRow(
      sessionsStore.isSessionWorking(props.sessionId),
      sessionsStore.getSessionMessages(props.sessionId),
    ),
  )

  /**
   * 排队中的消息（「待发送」气泡的数据源）。
   *
   * 自己从 store 取，不靠 props 传：队列是**会话级**状态，而本面板按地址重挂载
   * （`ChatMainPanel` 的 `:key`），来回切换不该让队列显示与否变得不确定——
   * 只要 store 里还有，切回来就还在。空数组时组件自行不渲染。
   */
  const inboxEntries = computed(() => sessionsStore.getInboxItems(props.sessionId))

  /** 会话级错误重试：未入队失败重发原消息；已落库失败对用户轮锚执行建支重试。 */
  function handleSessionRetry() {
    if (sessionRetryBusy.value) return
    sessionsStore.setSessionError(props.sessionId, null)

    // 优先重发**刚才没入队的那一条**。此前这里只回看转写里最后一条 user 消息：
    // 而发送失败意味着它根本没落库，于是“重试”会把上一轮已经成功发送的内容
    // 再发一遍——用户以为在重试失败的消息，实际又跑了一轮旧对话。
    const failed = lastFailedSend.value
    if (failed) {
      lastFailedSend.value = null
      retryingFailedSend.value = true
      // 失败时内容已经还给输入区；重试成功后再留着就等于同一内容可以发两次。
      // 只在输入区未被用户改过时清空（改过就说明他正在写别的）。
      const failedText = messageTextOf(failed.message.content)
      if (failedText && inputText.value === failedText) inputText.value = ''
      // 图片随这条失败消息重试，从输入区摘下来并交接所有权；用户新加的图留在原处。
      const retryImages = failed.images.filter((image) => attachedImages.value.includes(image))
      if (retryImages.length) {
        attachedImages.value = attachedImages.value.filter((image) => !retryImages.includes(image))
        claimImages(retryImages)
      }
      void (async () => {
        try {
          const ok = await dispatchSend(failed.message, retryImages)
          if (ok) releaseClaimedImages(retryImages)
          else returnClaimedImages(retryImages)
        } finally {
          retryingFailedSend.value = false
        }
      })()
      return
    }

    const anchor = roundRetryAnchor.value
    if (!anchor) return
    sessionsStore.setSessionError(props.sessionId, null)
    const pendingId = anchor.id
    const pendingVersion = ++roundResumeVersion.value
    roundResumePendingId.value = pendingId
    void chat.resume({ targetId: pendingId, action: 'retry_subtree' }).then((accepted) => {
      if (!accepted && roundResumeVersion.value === pendingVersion) roundResumePendingId.value = null
    })
  }

  /** 已落库的失败轮：编辑用户轮锚，保存成功后继续同一套建支重试。 */
  function handleSessionModify() {
    if (sessionRetryBusy.value) return
    const anchor = roundRetryAnchor.value
    if (!anchor) return
    handleEdit(anchor.id, true)
  }

  // --- 方法 ---
onMounted(() => {
  // 还原上次的阅读位置：有记录就回到「离底同样远」的地方（并重算「是否贴底」——否则
  // 下一次流式追加会把它拽回底部），无记录才贴底。
  const savedGap = uiState.scrollGapOf(props.sessionId)
  nextTick(() => {
    if (savedGap == null) {
      scrollToBottom()
      return
    }
    if (!messagesRef.value) return
    scrollToGap(savedGap)
    handleScroll()
  })

  // 懒创建闭环：新建会话流程中排队的首条消息（文本 + 可选图片附件；
  // 用户在"新建详情"输入、创建完成后经机制选中切到本会话），挂载即注入并发送。
  // 注意：图片的 thumbnailUrl 是 object URL，所有权随载荷转移，此处**不可 revoke**。
  const pending = sessionsStore.consumePendingFirstMessage(props.sessionId)
  if (pending) {
    inputText.value = pending.text
    if (pending.images?.length) composerRef.value?.restoreImages(pending.images)
    void handleSend()
  }
})

onBeforeUnmount(() => {
  disposed = true
  // 发送中的附件已离开输入区，由面板负责；未发送的仍由 ChatInputArea 自己释放。
  releaseClaimedImages()
  if (scrollRaf !== null) {
    cancelAnimationFrame(scrollRaf)
    scrollRaf = null
  }
})

// Handle send or abort logic
function handleSendOrAbort() {
  if (isStopping.value) {
    // 请求 / 权威终态帧到达前的重复点击统一由 composable 的同步 latch 拦下。
    void chat.abort()
  } else if (isLoading.value) {
    // While loading, only allow abort (no queuing new messages)
    void chat.abort()
  } else if (inputText.value.trim() || attachedImages.value.length > 0) {
    void handleSend()
  }
}

/** 最近一条**没入队成功**的消息：错误条的「重试」据此发对那一条 */
const lastFailedSend = ref<{ message: ChatNode; images: ImageAttachment[] } | null>(null)

/** 发送并记录失败目标（成功时清空，避免错误条重发旧内容） */
async function dispatchSend(msg: ChatNode, images: ImageAttachment[] = []): Promise<boolean> {
  let ok = false
  try {
    ok = await chat.send(msg)
  } catch (err) {
    logger.error('ModelChatPanel', '发送消息失败', err)
  }
  lastFailedSend.value = ok ? null : { message: msg, images }
  return ok
}

// 核心发送逻辑
async function handleSend() {
  const text = inputText.value.trim()
  const images = [...attachedImages.value]
  if (!text && images.length === 0) return

  // 构建消息内容
  let content: MessageContent
  if (images.length > 0) {
    const parts: ContentPart[] = []
    if (text) parts.push({ type: 'text', text })
    images.forEach(img => parts.push({
      type: 'image_url',
      image_url: { url: `data:${img.mimeType};base64,${img.base64}` }
    }))
    content = parts
  } else {
    content = text
  }

  const userMessage = {
    id: crypto.randomUUID(),
    content,
    timestamp: Date.now()
  }

  // 先把附件所有权交给面板，再清空预览。这样发送期间切换会话不会把 URL
  // 丢给已卸载的输入区；失败时由 returnClaimedImages 原样接回。
  claimImages(images)
  inputText.value = ''
  attachedImages.value = []
  composerRef.value?.resetHeight()

  nextTick(() => scrollToBottom())

  const chatMessage: ChatNode = {
    id: userMessage.id,
    principal: { kind: PRINCIPAL_KIND_PERSON, id: PRINCIPAL_USER },
    content: userMessage.content,
    timestamp: userMessage.timestamp
  }

  const ok = await dispatchSend(chatMessage, images)
  if (ok) {
    // 入队成功，正文与图片所有权到此结束；集合保证只 revoke 一次。
    releaseClaimedImages(images)
    return
  }

  // 失败：内容已经不在输入框里了，必须还给用户——他刚刚打的字不能只留在
  // 一条错误提示里。图片同理；若面板已卸载，则由本层释放而不是挂回死组件。
  returnClaimedImages(images)
  if (disposed) return
  const restoredToInput = !inputText.value
  if (restoredToInput) inputText.value = text
  // 提示必须自带重试入口：此前只说「可重试」，按钮却在别处（错误条 / 流末尾），
  // 用户此刻的注意力在 toast 上，找不到就以为只能重新打一遍。
  showToast(
    'error',
    restoredToInput ? '消息未发出，内容已保留' : '消息未发出',
    undefined,
    { label: '重试', run: () => void handleSessionRetry() },
  )
}

/** 回到最新：上翻阅读时流还在长，没有这个入口用户只能自己一路滚回底部 */
function jumpToBottom() {
  // 用户主动跳转 → 平滑滚动；自动跟随仍走瞬时（见 useChatScroll）。
  scrollToBottom(true)
  handleScroll()
}


/**
 * 重试入口：分派规则在 `registry/messageTypes.messageRetryTargetOf`——
 * 「工具调用失败 → 单工具重试」「整轮失败 → 用户轮锚另开一支」是消息级规则，
 * 不属于面板。本面板只负责取节点 + 发恢复请求。
 */
function handleRetry(messageId: string) {
  const messages = sessionsStore.getSessionMessages(props.sessionId)
  const msg = messages.find(m => m.id === messageId)
  if (!msg) return
  const target = messageRetryTargetOf(msg, messages)
  if (!target) return
  void chat.resume({ targetId: target.targetId, action: target.action })
}

const { showToast } = useToast()

/** 待确认的删除（打开确认框后才有值；确认前不动本地/后端任何数据） */
interface PendingDelete {
  id: string
  isUser: boolean
  userText: string
  /** 删除成功后回填到输入区的附件（data URL → 输入区形状）。 */
  images: ImageAttachment[]
  /** 非 data URL 的附件无法转回现有输入协议，必须明确告知。 */
  unrecoverableAttachments: number
  /** 本次会被删除的条数（目标 + 其后全部） */
  count: number
}
const pendingDelete = ref<PendingDelete | null>(null)

const pendingDeleteMessage = computed(() => {
  const p = pendingDelete.value
  if (!p) return ''
  if (p.count > 1) {
    return `会删除这条消息以及它之后的 ${p.count - 1} 条消息（共 ${p.count} 条），此操作无法撤销。`
  }
  return '此操作无法撤销。'
})

/** 删除入口：先取消息（回填与条数都要用），再弹确认 */
function handleDelete(messageId: string) {
  const msgs = sessionsStore.getSessionMessages(props.sessionId)
  const idx = msgs.findIndex(m => m.id === messageId)
  const msg = idx >= 0 ? msgs[idx] : undefined
  const isUserMsg = msg ? messageRoleOf(msg) === CHAT_ROLE_USER : false
  // 取值走契约层唯一实现，不在此另写形状判定
  const userText = isUserMsg ? messageTextOf(msg?.content) : ''
  const restored = isUserMsg ? restoreImagesOf(msg?.content) : { images: [], unsupported: 0 }
  pendingDelete.value = {
    id: messageId,
    isUser: isUserMsg,
    userText,
    images: restored.images,
    unrecoverableAttachments: restored.unsupported,
    count: idx >= 0 ? msgs.length - idx : 1,
  }
}

/** 确认后执行删除（由 store 调后端持久化）；若为 user 消息，删除后将其内容回填输入框 */
async function confirmDelete() {
  const p = pendingDelete.value
  pendingDelete.value = null
  if (!p) return
  try {
    await sessionsStore.deleteMessage(props.sessionId, p.id)
    // 用户消息的正文与附件一并回填，方便原样修改后重发（不自动发送）。
    if (p.isUser) {
      if (p.userText) {
        const cur = inputText.value ?? ''
        const sep = cur.trim().length > 0 ? '\n\n' : ''
        inputText.value = cur + sep + p.userText
      }
      if (p.images.length > 0) composerRef.value?.restoreImages(p.images)
      if (p.unrecoverableAttachments > 0) {
        showToast('error', `删除完成，但 ${p.unrecoverableAttachments} 个附件无法恢复：仅支持内嵌 base64 图片`)
      }
    }
  } catch (e) {
    logger.error('ModelChatPanel', 'deleteMessage 失败', e)
    // store 已把消息放回本地；这里必须说明“为什么闪了一下又回来”
    showToast('error', `删除失败：${e instanceof Error ? e.message : String(e)}`)
  }
}

/** 打开编辑浮层；整轮恢复可要求保存成功后立即另开一支。 */
function handleEdit(messageId: string, retryAfterSave = false) {
  const msg = sessionsStore.getSessionMessages(props.sessionId).find(m => m.id === messageId)
  if (!msg) return
  const c = msg.content
  const isJson = typeof c === 'string' && /^[\\[{]/.test(c.trim())
  const text = messageTextOf(c)
  // 文本可编辑，现有图片分片同时展示并允许逐张移除。
  editing.value = { id: messageId, content: text, isJson, attachments: imagePartsOf(c), retryAfterSave }
}

function closeEdit() {
  if (savingEdit.value) return
  editing.value = null
}

/**
 * 保存编辑：更新 store + 持久化到后端。
 *
 * `updateMessage` 是「先本地应用、再写后端」：写失败时本地已经是新内容，
 * 若不回滚就会界面与后端分叉（关掉浮层后用户以为改成了）。失败时把原消息
 * 重新落回本地（`applyTranscriptMessage`，与实时流同一条路径），并保留编辑态
 * 让用户能重试。
 */
async function saveEdit() {
  const edit = editing.value
  if (!edit || savingEdit.value) return
  const { id, content, attachments, retryAfterSave } = edit
  const msg = sessionsStore.getSessionMessages(props.sessionId).find(m => m.id === id)
  if (!msg) {
    editing.value = null
    return
  }
  // 文本 + 保留的 image_url 分片仍是原来的 ChatNode.content 协议；全部移除后
  // 回落纯文本，不会留下空图片数组或把图片 URL 拼进正文。
  const newContent: MessageContent = attachments.length > 0
    ? [{ type: 'text', text: content }, ...attachments]
    : content
  savingEdit.value = true
  try {
    await sessionsStore.updateMessage(props.sessionId, {
      ...msg,
      content: newContent,
    } as ChatNode)
  } catch (e) {
    sessionsStore.applyTranscriptMessage(props.sessionId, msg)
    savingEdit.value = false
    logger.error('ModelChatPanel', 'updateMessage 失败', e)
    showToast('error', `保存失败：${e instanceof Error ? e.message : String(e)}`)
    return
  }
  if (retryAfterSave && !await chat.resume({ targetId: id, action: 'retry_subtree' })) {
    savingEdit.value = false
    return
  }
  editing.value = null
  savingEdit.value = false
  nextTick(() => scrollToBottom())
}

// 智能滚动 —— **只 watch 版本号**，不 deep watch 消息树。
//
// `messageTree` 是一棵带 `children` 自引用的树，deep watch 会在每个流式 token
// 上遍历整棵树（O(n²)），而且监听器本身也会被树里任何一处改动唤醒。改成
// 订阅 store 的 `transcriptVersion`（一个数字，只在 `commitMessages` 提交时 +1），
// 再用 rAF 把一帧内的多次提交合并成一次滚动——滚动是**视觉效果**，
// 一帧一次就够，不必逐 token 同步。
let scrollRaf: number | null = null
watch(
  () => sessionsStore.transcriptVersion,
  () => {
    if (scrollRaf !== null) return
    scrollRaf = requestAnimationFrame(() => {
      scrollRaf = null
      smartScroll()
    })
  }
)

// 会话级错误条也要被滚进视野。它**不经过消息提交**（错误是会话状态，不是节点），
// `transcriptVersion` 因此不会变，上面那个 watcher 管不到它。
watch(sessionError, (now) => {
  if (!now) return
  void nextTick(() => smartScroll())
})
</script>

<style scoped>
.model-chat-panel {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 0;
  overflow: hidden;
  background: var(--color-chat-bg);
}

/* 消息区：单列的滚动容器（完整时间轴直排 的呈现合流定案）。
   间距、缩进、卡片内边距这套纵向节奏在列容器上定义一次
   （下游 MessageNode 经 CSS 继承复用）。 */
.chat-body {
  flex: 1;
  min-height: 0;
  display: flex;
  overflow: hidden;
}

.chat-column {
  position: relative;
  display: flex;
  flex-direction: column;
  flex: 1 1 0;
  min-width: 0;
  min-height: 0;
}

/* 回到最新：贴在消息区右下角，与输入区隔开一点，避免盖住最后一条气泡 */
.scroll-bottom {
  position: absolute;
  right: 1rem;
  bottom: 0.75rem;
  z-index: 2;
  display: inline-flex;
  align-items: center;
  gap: 0.25rem;
  padding: 0.3rem 0.7rem;
  border: 1px solid var(--border-default);
  border-radius: 999px;
  background: var(--surface-overlay);
  color: var(--text-secondary);
  box-shadow: var(--shadow-1);
  font-size: 0.75rem;
  cursor: pointer;
}
.scroll-bottom:hover {
  color: var(--text-primary);
  background: var(--surface-hover);
}

.column-body {
  flex: 1;
  min-height: 0;
  /* 滚动容器本身是 flex item：关掉 min-content 下限，长 nowrap 文本沿 flex 链
     顶宽时，这里能跟着列容器收缩而不是把 .msg 一起撑出右缘。 */
  min-width: 0;
  overflow-y: auto;
  /* 关掉浏览器的滚动锚定：它会在上方内容插入时自己挪 `scrollTop`，而展开更早的轮次
     时面板**已经**按高度差补过一次（`handleShowEarlier`）。两层各补一次就是补两倍，
     表现为「点一下往下窜一大截」。滚动位置由本面板一处负责。
     代价说清：其余「上方高度变了」的情形也不再由浏览器兜底——但会话流里剩下的那些
     变化都是**用户自己点的**（展开一张卡片：卡片在它正看着的位置之下，卡片本身不动），
     而流式增长恒在**尾部**（窗口保证最新一条总在 DOM 里），不在视口上方。 */
  overflow-anchor: none;
  padding: 1rem;
  display: flex;
  flex-direction: column;
  /* ── 统一纵向节奏（机制化保障一致性）──
     所有响应元素之间的纵向间隔都引用同一变量，下游 MessageNode 经 CSS 继承复用，
     杜绝散落的 0.75/0.5/0.4rem 多重间距导致的「忽大忽小」。 */
  --msg-gap: 0.4rem;         /* 统一纵向间隔：顶层节点 / 骨架子元素 / 工具子元素 全部共用（紧凑） */
  --nest-indent: 0.7rem;      /* 嵌套缩进步长（仅 depth>=2 的层级使用，见 MessageNode） */
  --card-pad-x: 0.5rem;       /* 卡片型节点统一左右内边距 */
  gap: var(--msg-gap);
}

.empty-chat {
  text-align: center;
  color: var(--text-muted);
  padding: 3rem 1rem;
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

/* 「显示更早的消息」——渲染窗口的收尾入口，形状与中栏列表的 `.more-btn` 同款胶囊。
   位置在流的**最上方**而不是 sticky：想往上翻的人本来就会滚到这里，
   而钉在顶部等于常年盖住第一条消息，还会把「往上还有多少」变成一条常驻横幅。 */
.earlier-row {
  display: flex;
  justify-content: center;
  padding: 0.25rem 0 0.5rem;
}
.earlier-btn {
  min-height: 2rem;
  padding: 0.35rem 1rem;
  border: 1px solid var(--border-default);
  border-radius: 999px;
  background: transparent;
  color: var(--text-secondary);
  font-size: 0.8125rem;
  cursor: pointer;
}
.earlier-btn:hover {
  background: var(--surface-hover);
}

.empty-chat p {
  margin: 0;
}

/* 停止过程 / 结果留在流尾，直到下一次发言或恢复；不随 token 更新。 */
.stop-notice {
  display: flex;
  align-items: center;
  padding: 0.4rem 0.65rem;
  border: 1px solid var(--border-default);
  border-radius: 0.5rem;
  background: var(--color-option-bg);
  color: var(--text-secondary);
  font-size: 0.8rem;
}
.stop-notice--success {
  color: var(--success-fg);
}
.stop-notice--error {
  color: var(--color-error-fg);
  border-color: var(--color-error-border);
  background: var(--color-error-bg);
}
/* 读屏专用区域始终挂载：内容从空变为一句终态，才能被 polite 通道可靠播报。 */
.stop-live {
  position: absolute;
  width: 1px;
  height: 1px;
  overflow: hidden;
  clip: rect(0 0 0 0);
  clip-path: inset(50%);
  white-space: nowrap;
}

.chat-controls {
  padding: 0.75rem 1rem;
  border-top: 1px solid var(--border-default);
  background: var(--surface-panel);
  flex-shrink: 0;
}

/* 注：会话级错误条不再在此处持有样式——它现在与容器级错误条、工具级错误条
   共用同一个组件（`message/MessageErrorBox.vue`）。三处各自维护一份 `.error-box`
   的写法正是它被抽出来要消除的东西（改一处不改另一处不会报错，只会观感不一致）。 */

/* ── 编辑单条消息的浮层（edit-box …）────────────────────────
   遮罩（定位 / 主题化底色 / 层级）与面板底色、圆角、阴影均由 `BaseModal` 提供。 */

:global(.modal-panel.edit-box) {
  width: min(34rem, 92vw);
  border: 1px solid var(--border-default);
  padding: var(--space-4);
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.edit-title {
  font-size: var(--font-size-base);
  font-weight: var(--font-weight-medium);
  color: var(--text-primary);
}

.edit-area {
  min-height: 8rem;
  padding: var(--space-2) var(--space-3);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: var(--surface-sunken);
  color: var(--text-primary);
  font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: var(--font-size-sm);
  line-height: 1.5;
  resize: vertical;
}

.edit-area:focus {
  outline: none;
  border-color: var(--accent);
}

.edit-retry-note {
  margin: 0;
  color: var(--text-secondary);
  font-size: var(--font-size-sm);
  line-height: 1.5;
}

.edit-attachments {
  display: flex;
  flex-wrap: wrap;
  gap: var(--space-2);
}
.edit-attachment {
  position: relative;
  width: 4.5rem;
  height: 4.5rem;
}
.edit-attachment img {
  width: 100%;
  height: 100%;
  object-fit: cover;
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: var(--surface-sunken);
}
.edit-attachment-remove {
  position: absolute;
  top: -0.35rem;
  right: -0.35rem;
  width: 1.5rem;
  height: 1.5rem;
  display: grid;
  place-items: center;
  padding: 0;
  border: 1px solid var(--border-default);
  border-radius: 999px;
  background: var(--surface-overlay);
  color: var(--text-primary);
  cursor: pointer;
}

.edit-btns {
  display: flex;
  justify-content: flex-end;
  gap: var(--space-2);
}

.edit-save,
.edit-cancel {
  padding: var(--space-2) var(--space-4);
  font-size: var(--font-size-sm);
  border-radius: var(--radius-md);
  border: 1px solid var(--border-default);
  cursor: pointer;
}

.edit-save {
  background: var(--accent);
  border-color: var(--accent);
  color: var(--text-on-accent);
}

.edit-save:hover {
  background: var(--accent-hover);
  border-color: var(--accent-hover);
}

.edit-save:disabled,
.edit-cancel:disabled,
.edit-area:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.edit-cancel {
  background: transparent;
  color: var(--text-secondary);
}

.edit-cancel:hover {
  background: var(--surface-hover);
}
</style>
