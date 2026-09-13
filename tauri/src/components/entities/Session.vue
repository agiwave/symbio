<!--
  Session — 会话专属详情（VDFS 下由 `ext = session` 选中，VdfsSessionDetail 薄适配）

  会话在后端仍是 `EntityProvider`（可读写、有 delete 钩子），但前端自 S8 起
  只经 VDFS 访问它：清单来自 `.vdfs/session` 的 `vdfs/list`（节点自带
  message_count / metadata / meta_tags），删除经 `@delete` 由 VDFS 页面层
  统一走 `vdfs/delete`。本组件只承载会话的"详情差异化"：

  - item 非 null（选中态）：聊天工作区 = ChatMainPanel（工作目录的层级
    浏览不在详情页——经机制动作「浏览内部」进入会话同名目录
    `<id>/工作目录[/<rel>]`，与子会话并列，见 vdfs-frontend.md §7 S6）；
  - item 为 null（机制"新建"态）：新建会话引导——输入区与现有会话完全一致
    （ChatInputArea + ChatOptionBar 草稿态：目录/Agent/模型/模式/风险等级/心跳
    均可选，由级联选项机制下发，暂存于机制内部的 metadata 缓冲，发送首条消息时
    经 createSession(patch) 一并写入 metadata 落库）（kind 级 editor 注册在
    `registry/entityTypes` → 该形态由 VDFS 的 ext 分发命中；空列表时页面
    自动进入此态）。

  选中同步：机制选中（:key 重挂载）是唯一真相，watch item.id → store.selectSession。
  创建经 emit('created') 回到机制页面层。机制动作（删除/容器入口等）经
  mechanism-actions prop 注入、由 ChatMainPanel 头部与自身按钮并排渲染——
  详情页只有自定义/机制化两种形态，机制动作一律在详情页内部渲染，
  页面不得另加外框。
-->
<template>
  <div v-if="item" class="session-editor">
    <ChatMainPanel
      class="col-chat"
      :mechanism-actions="mechanismActions"
      :deleting="deleting"
      @mech-action="onMechAction"
    />
  </div>

  <div v-else class="session-create">
    <div class="create-card">
      <h3 class="create-title">开始新会话</h3>
      <p class="create-desc">
        输入区与现有会话完全一致——工作目录、智能体、模型、运行模式与风险等级均可在发送前选择；发送第一条消息时才会真正创建会话（懒创建，不产生空会话）。
      </p>
      <ChatInputArea
        ref="draftInputRef"
        v-model="draftText"
        v-model:attached-images="draftImages"
        :is-loading="creating"
        class="create-chat-input"
        @submit="onSendFirst"
      />
      <!-- 草稿态选项行：与已有会话同源（级联选项机制），无会话时选择缓冲于
           机制内部，创建会话时作为 metadata 补丁一次写入 -->
      <ChatOptionBar ref="draftOptionsRef" />
    </div>
  </div>
</template>

<script setup lang="ts">
import { nextTick, onMounted, ref, watch } from 'vue'
import type { DetailAction, EntityCapabilities, EntitySummary } from '@/schemas/entities'
import type { ImageAttachment } from '@/types'
import { useSessionsStore } from '@/stores/sessions'
import ChatMainPanel from '@/components/session/ChatMainPanel.vue'
import ChatInputArea from '@/components/chat/ChatInputArea.vue'
import ChatOptionBar from '@/components/chat/ChatOptionBar.vue'

const props = defineProps<{
  item: EntitySummary | null
  capabilities: EntityCapabilities
  /** 机制动作注入（页面单一定义点计算：容器入口/删除等） */
  mechanismActions?: DetailAction[]
  saving?: boolean
  testing?: boolean
  deleting?: boolean
}>()

const emit = defineEmits<{
  /** 机制约定：editor 完成实体创建后上报 id，页面刷新清单并选中之 */
  (e: 'created', id: string): void
  /** 机制约定：editor 请求删除当前选中实体（VDFS 下由 `vdfs/delete` 承载） */
  (e: 'delete'): void
  /** 机制约定：editor 请求进入条目内部（VDFS 下 `enter(node.path)`；payload.kind 指定子类别） */
  (e: 'open-container', kind: string): void
}>()

const store = useSessionsStore()
const creating = ref(false)

/**
 * 新建态（懒创建）草稿：输入文本 + 选项行的 metadata 缓冲。
 * 发送首条消息时经 createSession(metadataPatch) 一并写入 metadata 落库。
 */
const draftInputRef = ref<{ resetHeight: () => void; textarea: HTMLTextAreaElement | null } | null>(null)
const draftOptionsRef = ref<{ getDraftMetadata: () => Record<string, unknown> } | null>(null)
const draftText = ref('')
const draftImages = ref<ImageAttachment[]>([])

// 挂载后自动聚焦输入框（新建引导，减少一次点击）
onMounted(() => {
  void nextTick(() => draftInputRef.value?.textarea?.focus())
})

/** 机制动作分发（ChatMainPanel 头部按钮 → 机制通道） */
function onMechAction(a: DetailAction) {
  if (a.id === 'delete') emit('delete')
  else if (a.id === 'open-container')
    emit('open-container', String((a.payload as Record<string, unknown> | undefined)?.kind ?? ''))
}

// 机制选中是唯一真相：editor 挂载/切换（:key 重挂载触发 watch）即选中该会话
watch(
  () => props.item?.id,
  (id) => {
    if (id) void store.selectSession(id)
  },
  { immediate: true }
)

/**
 * 新建模式（无 id）：发送首条消息时才真正创建会话（懒创建）。
 *
 * 机制约定（实体生命周期联动）：
 * - 新建态不创建任何实体、列表不更新；
 * - 发送首条消息才调 createSession 真正建会话（草稿选项行的 metadata 补丁由
 *   级联选项机制通用缓冲产出，创建时一次性写入 metadata；workdir 缺省回退最近
 *   使用目录由 createSession 兜底），同时向 store 排队该首条消息（文本 + 附件）；
 * - emit('created', id) 让 Workbench 刷新清单并立即选中新会话；
 * - Session editor 卸载 → 新选中项的 ChatMainPanel/ModelChatPanel 挂载 →
 *   消费排队消息（按 id 匹配）→ 发出首条消息，完成闭环。
 */
async function onSendFirst() {
  const text = draftText.value.trim()
  if ((!text && draftImages.value.length === 0) || creating.value) return
  creating.value = true
  try {
    // 懒创建：此刻才真正建会话；草稿选择（机制通用 metadata 补丁）写入本地条目
    // 与后端 metadata，保证选中切换后选项栏回显与草稿一致。
    const id = await store.createSession(draftOptionsRef.value?.getDraftMetadata())
    // 首条消息（含附件）排队给新会话的 ChatMainPanel；附件 thumbnailUrl 是
    // object URL，所有权随载荷转移（消费端发送后 revoke），此处**不可 revoke**。
    store.pendingFirstMessage = {
      id,
      text,
      images: draftImages.value.length ? draftImages.value : undefined
    }
    // 清空本地草稿输入（附件不 revoke，见上）
    draftText.value = ''
    draftImages.value = []
    emit('created', id)
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
  background: var(--color-bg);
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
