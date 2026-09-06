<!--
  Session — 会话实体专属详情（WorkbenchView 机制下的差异化 editor，kind 级注册）

  会话 = 统一实体机制中的 read-only+mutable 实体（entities/list 摘要含
  is_working 实时状态；删除经重写的 delete_item 钩子走统一 entities/delete）。
  本组件只承载会话的"详情差异化"：

  - item 非 null（选中态）：聊天工作区 = ChatMainPanel（工作目录的层级
    浏览不在详情页——经机制动作「管理内部实体」进入会话容器实体页，
    目录树是 tree 机制的一个场景子类别，与子会话并列）；
  - item 为 null（机制"新建"态）：新建会话引导（capabilities.independent_form +
    kind 级 editor 注册 → 机制新建按钮自动可用；空列表时页面自动进入此态）。

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
        将使用最近的工作目录创建新会话；尚未设置过目录时，可在创建后的聊天区引导中选择。
      </p>
      <button class="action-btn" :disabled="creating" @click="onCreate">
        {{ creating ? '创建中…' : '新建会话' }}
      </button>
    </div>
  </div>
</template>

<script setup lang="ts">
import { ref, watch } from 'vue'
import type { DetailAction, EntityCapabilities, EntitySummary } from '@/schemas/entities'
import { useSessionsStore } from '@/stores/sessions'
import ChatMainPanel from '@/components/session/ChatMainPanel.vue'

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
  /** 机制约定：editor 请求删除当前选中实体（走统一 entities/delete） */
  (e: 'delete'): void
  /** 机制约定：editor 请求进入容器实体管理页（payload.kind 指定容器类别） */
  (e: 'open-container', kind: string): void
}>()

const store = useSessionsStore()
const creating = ref(false)

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

async function onCreate() {
  if (creating.value) return
  creating.value = true
  try {
    // 直接创建会话（有最近 workdir 则自动沿用）；未设置 workdir 时由聊天区的
    // EmptyWorkdirState 空态引导用户选择，不在此强制弹目录对话框打断流程。
    const id = await store.createSession()
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
  align-items: center;
  gap: 0.75rem;
  text-align: center;
  max-width: 24rem;
}
.create-title {
  margin: 0;
  font-size: var(--font-size-lg);
  font-weight: var(--font-weight-semibold);
  color: var(--text-primary);
}
.create-desc {
  margin: 0;
  font-size: var(--font-size-sm);
  color: var(--text-muted);
  line-height: var(--line-height-normal);
}
</style>
