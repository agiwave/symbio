<!--
  Session — 会话实体专属详情（WorkbenchView 机制下的差异化 editor，kind 级注册）

  会话 = 统一实体机制中的 read-only+mutable 实体（entities/list 摘要含
  is_working 实时状态；删除经重写的 delete_item 钩子走统一 entities/delete）。
  本组件只承载会话的"详情差异化"：

  - item 非 null（选中态）：聊天工作区两栏 = ChatMainPanel + SessionExplorerPanel
    （机制列表承担会话清单，本组件不再渲染列表）；
  - item 为 null（机制"新建"态）：新建会话引导（capabilities.independent_form +
    kind 级 editor 注册 → 机制新建按钮自动可用；空列表时页面自动进入此态）。

  选中同步：机制选中（:key 重挂载）是唯一真相，watch item.id → store.selectSession。
  创建/删除经 emit('created' / 'delete') 回到机制页面层，不在本组件内自持清单状态。
-->
<template>
  <div v-if="item" class="session-editor">
    <ChatMainPanel class="col-chat" @delete-session="emit('delete')" />
    <SessionExplorerPanel class="col-explorer" />
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
import type { EntityCapabilities, EntitySummary } from '@/schemas/entities'
import { useSessionsStore } from '@/stores/sessions'
import { useExplorerStore } from '@/stores/explorer'
import ChatMainPanel from '@/components/session/ChatMainPanel.vue'
import SessionExplorerPanel from '@/components/session/SessionExplorerPanel.vue'

const props = defineProps<{
  item: EntitySummary | null
  capabilities: EntityCapabilities
  saving?: boolean
  testing?: boolean
  deleting?: boolean
}>()

const emit = defineEmits<{
  /** 机制约定：editor 完成实体创建后上报 id，页面刷新清单并选中之 */
  (e: 'created', id: string): void
  /** 机制约定：editor 请求删除当前选中实体（走统一 entities/delete） */
  (e: 'delete'): void
}>()

const store = useSessionsStore()
const explorer = useExplorerStore()
const creating = ref(false)

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
    explorer.reset()
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

.col-explorer {
  flex: 0 0 17.5rem;
  min-width: 12.5rem;
  max-width: 26.25rem;
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
