<!--
  Session — 会话详情（VDFS 下由 `ext = session` 选中，VdfsSessionDetail 薄适配）

  会话在后端就是一个 `VdfsProvider` 提供的节点：清单来自 `<根>/session` 的
  `vdfs/list`（节点自带 message_count / metadata / meta_tags），正文是会话叶子的
  内容（`<根>/session/<id>` 的 `vdfs/read`），删除经 `@delete` 由页面层统一走
  `vdfs/delete`。本组件只承载会话的"详情差异化"：

  - node 有 id（选中态）：聊天工作区 = ChatMainPanel（工作目录的层级浏览不在
    详情页——经机制动作「浏览内部」进入会话同名目录
    `<id>/workdir[/<rel>]`，与子会话并列，见 docs/design/vdfs.md）；
  - node 无 id（机制「新建」态 = **草稿节点**，见 `useVdfs.startNew`）：新建会话
    引导——输入区与现有会话完全一致（`ChatComposer` 草稿态：目录/Agent/模型/模式/
    风险等级/心跳均可选，由会话选项栏（后端随节点下发定义）渲染，暂存于机制内部的 metadata 缓冲，
    发送首条消息时经 `createSessionWithFirstMessage` **一次 `vdfs/write`** 创建并
    落库——id 由后端生成）。

  选中同步：机制选中（:key 重挂载）是唯一真相，watch node.path（**地址**，
  含「住在哪个空间」）→ store.selectSession。
  创建经 emit('created') 回到机制页面层。

  动作分两来源（合并与渲染都交给 ChatMainPanel 头部的一处）：
  - **自有**（`actions`）= 「浏览内部」——进入会话同名目录，纯导航；
  - **机制**（`mechanismActions`）= 删除，由页面按访问位单点算好注入。

  详情页只有专属渲染器 / 机制化两种形态，机制动作一律在详情页内部渲染，
  页面不得另加外框。
-->
<template>
  <div v-if="hasId" class="session-editor">
    <ChatMainPanel
      class="col-chat"
      :actions="actions"
      :mechanism-actions="mechanismActions"
      :mechanism-busy="mechanismBusy"
      @action="onAction"
    />
  </div>

  <div v-else class="session-create">
    <div class="create-card">
      <h3 class="create-title">开始新会话</h3>
      <p class="create-desc">
        输入区与现有会话完全一致——工作目录、智能体、模型、运行模式与风险等级均可在发送前选择；发送第一条消息时才会真正创建会话（懒创建，不产生空会话）。
      </p>
      <ChatComposer
        ref="draftComposerRef"
        v-model="draftText"
        v-model:attached-images="draftImages"
        :is-loading="creating"
        :definition="draftDefinition"
        autofocus
        class="create-chat-input"
        @submit="onSendFirst"
      />
    </div>
  </div>
</template>

<script setup lang="ts">
import { computed, ref, watch } from 'vue'
import { isVdfsDraft, type DetailAction, type DetailDefinition, type VdfsItem } from '@/schemas/vdfs'
import type { ImageAttachment } from '@/types'
import { useSessionsStore } from '@/stores/sessions'
import { useToast } from '@/composables/useToast'
import { logger } from '@/utils/logger'
import ChatMainPanel from '@/components/session/ChatMainPanel.vue'
import ChatComposer from '@/components/chat/ChatComposer.vue'

const props = defineProps<{
  /** 会话条目（`<根>/session/<id>`）；无地址 = 新建草稿态 */
  node: VdfsItem | null
  /** 会话自有动作（浏览内部；由 VdfsSessionDetail 声明） */
  actions?: DetailAction[]
  /** 机制动作注入（页面单一定义点计算：删除等） */
  mechanismActions?: DetailAction[]
  /** 正在执行的机制动作 id（驱动其进行中文案） */
  mechanismBusy?: string | null
  saving?: boolean
}>()

const emit = defineEmits<{
  /** 机制约定：详情渲染完成资源创建后上报 id，页面刷新清单并选中之 */
  (e: 'created', id: string): void
  /** 机制约定：请求删除当前选中节点（VDFS 下由 `vdfs/delete` 承载） */
  (e: 'delete'): void
  /** 机制约定：请求进入节点内部（VDFS 下 `enter(node.path)`；payload.kind 指定子类别） */
  (e: 'open-container', kind: string): void
}>()

const store = useSessionsStore()
const { showToast } = useToast()
const creating = ref(false)

/**
 * 是否已落盘的会话。
 *
 * **判据与页面同源**（`schemas/vdfs.isVdfsDraft`：没有路径 = 还没落盘）：新建态
 * 由机制以「草稿节点」进入（无 path / 无 name），因此「点新建」与「选中一项」
 * 走的是同一个渲染器、同一张详情页——差别只是这一页有没有东西可看。
 * 不再需要第二种页面形态，也不再各写一份「有没有名字」的判据。
 */
const hasId = computed(() => !isVdfsDraft(props.node))

/**
 * 草稿态的选项定义 = 新建类型自带的 `schema`。
 *
 * 它是「点新建与选中一项进入同一详情页」这条约定在新形态下的落点：草稿节点由
 * `useVdfs.draftNodeOf` 按 `new_type.schema` 构造，与会话节点上的 `schema`
 * **逐字节相同**（同一个构造函数产出），因此草稿选项栏与真实会话选项栏渲染的是
 * 同一份定义，只差「值还没有」。
 */
const draftDefinition = computed<DetailDefinition | null>(
  () => (props.node?.schema as DetailDefinition | undefined) ?? null
)

/**
 * 新建态（懒创建）草稿：输入文本 + 选项行的 metadata 缓冲。
 * 发送首条消息时经 createSessionWithFirstMessage(metadataPatch, 首条消息)
 * 原子地「建会话 + 排队首条消息」（队列项的 id 必然等于新建出来的 id）。
 */
const draftComposerRef = ref<{
  getDraftMetadata: () => Record<string, unknown>
} | null>(null)
const draftText = ref('')
const draftImages = ref<ImageAttachment[]>([])

/** 动作分发（ChatMainPanel 头部按钮 → 页面层机制通道） */
function onAction(a: DetailAction) {
  if (a.id === 'delete') emit('delete')
  else if (a.id === 'open-container')
    emit('open-container', String((a.payload as Record<string, unknown> | undefined)?.kind ?? ''))
}

// 机制选中是唯一真相：渲染器挂载/切换（:key 重挂载触发 watch）即选中该会话
//
// 传的是**地址**（`node.path`，即 `<挂载目录>/<id>`）而不是名字：会话在前端的
// 身份是「空间 + id」，只传 id 会把「住在哪个空间」在这一跳丢掉——子智能体空间
// 里选中的会话于是被当成根空间的会话，消息发到了父智能体。地址解析见
// `schemas/vdfs.parseVdfsSessionAddr`。
watch(
  () => props.node?.path,
  (addr) => {
    if (addr) void store.selectSession(addr)
  },
  { immediate: true }
)

/**
 * 新建模式（无节点）：发送首条消息时才真正创建会话（懒创建）。
 *
 * 机制约定（资源生命周期联动）：
 * - 新建态不创建任何节点、清单不更新；
 * - 发送首条消息才真正建会话（草稿选项行的 metadata 补丁由会话选项栏通用
 *   缓冲产出；workdir 缺省回退最近使用目录由 store 兜底），同时把首条消息排进
 *   邮箱——**建会话与排队是一个原子操作**（`createSessionWithFirstMessage`），
 *   否则「队列项的 id === 新建出来的 id」这条不变式就得由调用方用局部变量维持；
 * - emit('created', id) 让工作台刷新清单并立即选中新会话；
 * - 本组件卸载 → 新选中项的 ChatMainPanel/ModelChatPanel 挂载 →
 *   消费排队消息（按 id 匹配）→ 发出首条消息，完成闭环。
 */
async function onSendFirst() {
  const text = draftText.value.trim()
  if ((!text && draftImages.value.length === 0) || creating.value) return
  creating.value = true
  try {
    // 懒创建：此刻才真正建会话——一次 `vdfs/write`（写会话挂载根，无名字，
    // id 由后端生成）；草稿选择（机制通用 metadata 补丁）随创建一并落库，
    // 保证选中切换后选项栏回显与草稿一致。
    const id = await store.createSessionWithFirstMessage(
      draftComposerRef.value?.getDraftMetadata(),
      { text, images: draftImages.value.length ? draftImages.value : undefined }
    )
    // 清空本地草稿输入（附件的 object URL 所有权已随载荷转移给邮箱，
    // 故此处**不可 revoke**——契约见 store 的 `PendingFirstMessage`）
    draftText.value = ''
    draftImages.value = []
    emit('created', id)
  } catch (e) {
    // 创建失败（后端没落库）→ 必须说出来，否则「点了没反应」；草稿内容原样保留，可重试
    logger.error('Session', '新建会话失败：', e)
    showToast('error', `新建会话失败：${e instanceof Error ? e.message : String(e)}`)
  } finally {
    creating.value = false
  }
}
</script>

<style scoped>
.session-editor {
  display: flex;
  width: 100%;
  height: 100%;
  min-height: 0;
  background: var(--surface-page);
}

.col-chat {
  flex: 1 1 auto;
  min-width: 0;
}

.session-create {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 2rem;
  overflow-y: auto;
}
.create-card {
  display: flex;
  flex-direction: column;
  gap: 0.75rem;
  width: min(40rem, 100%);
}
.create-title {
  margin: 0;
  font-size: var(--font-size-lg);
  font-weight: var(--font-weight-semibold);
  color: var(--text-primary);
  text-align: center;
}
.create-desc {
  margin: 0;
  font-size: var(--font-size-sm);
  color: var(--text-muted);
  line-height: var(--line-height-normal);
  text-align: center;
}

/* 新建态输入区（与现有会话 chat-controls 同构：输入框 + 选项行） */
.create-chat-input {
  width: 100%;
}
</style>
