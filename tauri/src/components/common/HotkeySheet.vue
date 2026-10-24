<!--
  HotkeySheet — 快捷键速查（Cmd+/）

  ## 它存在的理由只有一条

  快捷键的失败方式是**静默**的：用户不知道有这条键，它就等于不存在；而把键位写在
  README 里的对照表，会在下一次改绑定时变成假的。**本面板的内容直接读
  `useHotkeys` 的登记表**——绑定声明一次，速查表与真实派发不可能不一致。

  因此这里不写任何**全局键**的字面量，也不写「本文另有几条快捷键」的补充说明。
  唯一的例外是页眉那句「按 Esc 关闭」：那是 `BaseModal` 自己的契约（焦点陷阱里的
  Esc 归它，全局派发因此对模态让路），不是登记表里的绑定。

  ## 打开时机决定清单内容

  登记表是动态的：列表移动、类别切换那几条由工作台控件在挂载时登记。所以清单在
  **每次打开时**重读一次，而不是模块加载时算好一份——否则用户在一个没有列表的
  页面上看到「↑↓ 移动」，按下去什么也不动，那条说明就是错的。
-->
<template>
  <BaseModal
    :visible="visible"
    panel-class="hotkey-sheet"
    labelledby="hotkey-sheet-title"
    @close="emit('close')"
  >
    <header class="hk-head">
      <h3 id="hotkey-sheet-title" class="hk-title">键盘快捷键</h3>
      <p class="hk-hint">按 <kbd>Esc</kbd> 关闭</p>
    </header>

    <p v-if="groups.length === 0" class="hk-hint">当前页面没有可用的快捷键。</p>
    <section v-for="g in groups" :key="g.name" class="hk-group">
      <h4 class="hk-group-name">{{ g.name }}</h4>
      <dl class="hk-rows">
        <div v-for="b in g.bindings" :key="b.id" class="hk-row">
          <dt class="hk-keys"><kbd>{{ formatHotkey(b.keys) }}</kbd></dt>
          <dd class="hk-label">{{ b.label }}</dd>
        </div>
      </dl>
    </section>
  </BaseModal>
</template>

<script setup lang="ts">
import { ref, watch } from 'vue'
import BaseModal from './BaseModal.vue'
import { formatHotkey, hotkeyTable, type HotkeyBinding } from '@/composables/useHotkeys'

const props = defineProps<{ visible: boolean }>()
const emit = defineEmits<{ (e: 'close'): void }>()

const groups = ref<{ name: string; bindings: HotkeyBinding[] }[]>([])

watch(
  () => props.visible,
  (v) => {
    if (!v) return
    const byName = new Map<string, HotkeyBinding[]>()
    for (const b of hotkeyTable()) {
      const list = byName.get(b.group) ?? []
      list.push(b)
      byName.set(b.group, list)
    }
    groups.value = [...byName.entries()].map(([name, bindings]) => ({ name, bindings }))
  },
  { immediate: true }
)
</script>

<style scoped>
.hotkey-sheet {
  width: min(30rem, 92vw);
  padding: var(--space-4);
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
}

.hk-head {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--space-3);
}

.hk-title {
  margin: 0;
  font-size: var(--font-size-md);
  color: var(--text-primary);
}

.hk-hint {
  margin: 0;
  font-size: var(--font-size-xs);
  color: var(--text-muted);
}

.hk-group + .hk-group {
  border-top: 1px solid var(--border-default);
  padding-top: var(--space-3);
}

.hk-group-name {
  margin: 0 0 var(--space-2);
  font-size: var(--font-size-xs);
  font-weight: 500;
  color: var(--text-muted);
  letter-spacing: 0.02em;
}

.hk-rows {
  margin: 0;
  display: flex;
  flex-direction: column;
  gap: var(--space-1);
}

.hk-row {
  display: flex;
  align-items: baseline;
  justify-content: space-between;
  gap: var(--space-3);
}

.hk-keys {
  flex-shrink: 0;
}

.hk-keys kbd {
  font-family: inherit;
  font-size: var(--font-size-xs);
  padding: 1px var(--space-2);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-sm);
  background: var(--surface-sunken);
  color: var(--text-primary);
}

.hk-label {
  margin: 0;
  font-size: var(--font-size-sm);
  color: var(--text-secondary);
  text-align: right;
}
</style>
