<!--
  CommandPalette — 全局命令面板（Cmd+K）

  ## 它解决什么

  这款应用里一切都是资源树上的一个节点，而「找到那个节点」目前只有一条路：
  从左栏一类一类点进去。会话多起来之后，这条路是鼠标遍历（P1 的效率天花板）。
  面板给的是**直达**：打两个字 → Enter → 就在那一项上。

  ## 三条口径

  1. **全库搜索归面板，不归中栏筛选**。中栏那个框只在**当前目录已加载的那一页**里
     筛（判据见 `useVdfs` 的筛选一节）；把全库结果混进那里会破坏「点目录即钻入」的
     空间心智。两条入口各有各的话，这里不越界。
  2. **结果按类别分组，组序 = 左栏序**。类别名取自节点 `title`（「会话」「模型」），
     本文件不出现任何类别字面量。
  3. **一次 `vdfs/tree`，筛词在本地**。打字不发请求 ⇒ 面板不会把后端的读放大
     乘上按键次数；被截断时 `truncated` 会说出来，不把「没返回」演成「不存在」。

  「新建 X」与「去 X 那一类」是**同一个目的地**：工作台进入带 `new_type` 的目录、
  且没选中任何项时会自动铺好一张草稿（`VdfsWorkbench` 的 `autoDraftDone`），
  因此面板不做出两条长得不同的入口。
-->
<template>
  <BaseModal
    :visible="visible"
    panel-class="palette"
    labelledby="palette-title"
    @close="emit('close')"
  >
    <h2 id="palette-title" class="pa-sr">命令面板</h2>
    <input
      v-model="query"
      class="pa-input"
      type="text"
      placeholder="搜索资源，或回车进入某一类…"
      aria-label="搜索资源"
      spellcheck="false"
      @keydown.down.prevent="move(1)"
      @keydown.up.prevent="move(-1)"
      @keydown.enter.prevent="activate()"
    />

    <div ref="listRef" class="pa-list" role="listbox" aria-label="结果">
      <p v-if="loading" class="pa-note">正在取资源清单…</p>
      <template v-else>
        <section v-for="g in groups" :key="g.name" class="pa-group">
          <p class="pa-group-name">{{ g.name }}</p>
          <button
            v-for="e in g.entries"
            :key="e.key"
            class="pa-row"
            :class="{ 'is-active': e.key === activeKey }"
            type="button"
            role="option"
            :aria-selected="e.key === activeKey"
            @mouseenter="activeKey = e.key"
            @click="activate(e.key)"
          >
            <span class="pa-title">{{ e.title }}</span>
            <span class="pa-hint">{{ e.hint }}</span>
          </button>
        </section>
        <p v-if="flat.length === 0" class="pa-note">
          {{ query.trim() ? '没有匹配的资源' : '没有可新建的类别' }}
        </p>
      </template>
    </div>

    <div class="pa-foot">
      <span v-if="truncated" class="pa-warn">清单未取完（还有目录没展开），结果可能不全</span>
      <span class="pa-keys"><kbd>↑↓</kbd> 选择 · <kbd>Enter</kbd> 前往 · <kbd>Esc</kbd> 关闭</span>
    </div>
  </BaseModal>
</template>

<script setup lang="ts">
import { computed, nextTick, ref, watch } from 'vue'
import { useRouter } from 'vue-router'
import BaseModal from './BaseModal.vue'
import { treeVdfs } from '@/services/vdfs'
import { READBACK_REASON } from '@/services/readback'
import { vdfsRoot } from '@/schemas/vdfsRoot'
import { vdfsBrowserPathOf } from '@/schemas/vdfsAddress'
import type { VdfsItem } from '@/schemas/vdfs'
import { buildPalette, flattenGroups } from '@/utils/palette'

const props = defineProps<{ visible: boolean }>()
const emit = defineEmits<{ (e: 'close'): void }>()

const router = useRouter()

const query = ref('')
const nodes = ref<VdfsItem[]>([])
const truncated = ref(false)
const loading = ref(false)
const activeKey = ref('')
const listRef = ref<HTMLElement | null>(null)

/** 类别 + 条目两层（再深是会话内部的事，面板不替用户钻进去） */
const DEPTH = 2

const groups = computed(() => buildPalette(nodes.value, { root: vdfsRoot(), query: query.value }))
const flat = computed(() => flattenGroups(groups.value))

async function load() {
  loading.value = true
  // 不写 try/catch：`treeVdfs` 与其余三个回读动词同一条降级口径（失败即空清单 +
  // 服务层留痕），拿不到东西时面板照常可关，这里再造一层只会掩盖那条留痕。
  const resp = await treeVdfs(READBACK_REASON.PALETTE_SEARCH, vdfsRoot(), { depth: DEPTH })
  nodes.value = resp.nodes ?? []
  truncated.value = Boolean(resp.truncated)
  loading.value = false
}

// 每次打开：清空上一次的词、重取一次清单（焦点由 BaseModal 自动落到输入框）。
//
// 重取而不是缓存：面板回答的是「现在有什么」，一份跨页面的缓存会把用户刚建好的
// 那一项挡在外面；而代价只是一次 `vdfs/tree`，不是每敲一个字一次。
//
// `immediate` 不能省：宿主用条件渲染决定显隐时（与其它弹窗同形），挂载这一拍
// `visible` **已经是 true**、没有"变化"可触发 watch ⇒ 面板开着一份空清单。
watch(
  () => props.visible,
  (v) => {
    if (!v) return
    query.value = ''
    void load()
  },
  { immediate: true }
)

// 结果换了 ⇒ 高亮回到第一格：否则高亮停在一条已经不在屏上的结果上，
// 用户按回车去的是他看不见的地方。
watch(
  flat,
  (list) => {
    activeKey.value = list[0]?.key ?? ''
  },
  { flush: 'post' }
)

/** 高亮格当前在摊平清单里的位置 */
const activeIndex = computed(() => flat.value.findIndex((e) => e.key === activeKey.value))

function move(step: number) {
  const list = flat.value
  if (list.length === 0) return
  const next = (activeIndex.value + step + list.length) % list.length
  activeKey.value = list[next].key
  void nextTick(() => {
    const el = listRef.value?.querySelector<HTMLElement>('.pa-row.is-active')
    // happy-dom 没有这个函数（真实浏览器才有）：滚动是呈现细节，缺它不影响选择
    if (typeof el?.scrollIntoView === 'function') el.scrollIntoView({ block: 'nearest' })
  })
}

/** 回车 / 点击：把数据地址换成浏览器地址推过去，然后收面板 */
function activate(key?: string) {
  const entry = key ? flat.value.find((e) => e.key === key) : flat.value[activeIndex.value]
  if (!entry) return
  emit('close')
  void router.push(vdfsBrowserPathOf(entry.addr))
}
</script>

<style scoped>
.palette {
  width: min(38rem, 94vw);
  padding: var(--space-4);
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

/* 面板有标题（aria-labelledby 要用），但不占一行视觉高度 */
.pa-sr {
  position: absolute;
  width: 1px;
  height: 1px;
  margin: 0;
  overflow: hidden;
  clip-path: inset(50%);
  white-space: nowrap;
}

.pa-input {
  width: 100%;
  padding: var(--space-2) var(--space-3);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: var(--surface-sunken);
  color: var(--text-primary);
  font-size: var(--font-size-sm);
  outline: none;
}

.pa-input:focus {
  border-color: var(--accent);
}

.pa-list {
  max-height: 24rem;
  overflow-y: auto;
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}

.pa-group {
  display: flex;
  flex-direction: column;
}

.pa-group-name {
  margin: var(--space-2) 0 var(--space-1);
  font-size: var(--font-size-xs);
  color: var(--text-muted);
  letter-spacing: 0.02em;
}

.pa-row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--space-3);
  padding: var(--space-2) var(--space-3);
  border: none;
  border-radius: var(--radius-md);
  background: transparent;
  color: var(--text-primary);
  font-size: var(--font-size-sm);
  text-align: left;
  cursor: pointer;
}

.pa-row:hover,
.pa-row.is-active {
  background: var(--surface-hover);
}

.pa-row.is-active {
  box-shadow: inset 2px 0 0 var(--accent);
}

.pa-title {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}

.pa-hint {
  flex-shrink: 0;
  font-size: var(--font-size-xs);
  color: var(--text-muted);
}

.pa-note {
  margin: var(--space-3) 0;
  font-size: var(--font-size-sm);
  color: var(--text-muted);
}

.pa-foot {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: var(--space-3);
  border-top: 1px solid var(--border-default);
  padding-top: var(--space-2);
  font-size: var(--font-size-xs);
  color: var(--text-muted);
}

.pa-warn {
  color: var(--danger-fg);
}

.pa-keys kbd {
  font-family: inherit;
  font-size: inherit;
}
</style>
