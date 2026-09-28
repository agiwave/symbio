/**
 * 会话级 UI 状态（纯前端，按 sessionId 记忆）——**唯一实现**
 *
 * ## 为什么要有它
 *
 * 会话面板是**按地址重挂载**的（`ChatMainPanel` 用 `:key="store.activeAddr"`，
 * 见那里的说明）。切走再切回时组件是全新的：输入框草稿、滚动位置一并不见。
 * 这不影响正确性，却是「切一下再回来，刚才写的东西没了」这类最高频的挫败感。
 *
 * 状态因此搬到一个**按 id 索引**的 store：面板重挂载后自取，不需要记住任何
 * 「上一个会话是谁」。放在 store 而不是某个上层组件里，也是因为两个消费点
 * （输入框在 `ModelChatPanel`，滚动容器在同文件）各自重挂载，没有共同祖先可传。
 *
 * ## 边界（写清楚，避免它长成第二个消息缓存）
 *
 * - 只放**纯 UI 状态**：不进后端、不参与任何判据、不参与渲染顺序；
 * - **不缓存消息**：消息的权威来源仍是 VDFS 变更 → `stores/sessions`，这里多一份
 *   就是第二个真相；
 * - **附件（图片）刻意不在其列**：`ImageAttachment.thumbnailUrl` 是 object URL，
 *   所有权随载荷移交并在**发送后**才 revoke（见 `stores/sessions` 的
 *   `PendingFirstMessage` 契约）。把它留在按 id 的 map 里等于把「谁负责 revoke」
 *   变成两处，提前 revoke 的表现就是裂图——宁可少记这一项。
 *
 * ## 为什么不需要守卫
 *
 * 读写的都是「本会话自己的」值，键由调用方给（面板只知道自己的 `sessionId`）。
 * 面板 A 永远读不到面板 B 的状态，故不存在「慢响应覆盖新会话」那类竞态
 * （那类问题的唯一来源是**共享单例**，见 `useGenerationGuard`）。
 */

import { defineStore } from 'pinia'
import { ref } from 'vue'

export const useSessionUiStateStore = defineStore('sessionUiState', () => {
  /** sessionId → 输入框草稿（无记录 = 空串，不存空串） */
  const drafts = ref<Record<string, string>>({})
  /** sessionId → 消息区滚动位置（无记录 = 从未滚动过，由调用方决定贴底还是还原） */
  const scrollTops = ref<Record<string, number>>({})

  /** 该会话的输入草稿（无记录 ⇒ 空串） */
  function draftOf(sessionId: string): string {
    return (sessionId && drafts.value[sessionId]) || ''
  }

  /**
   * 记一份草稿。
   *
   * 空串 = **清除**该条，而不是留一条空记录：用户把输入框清空之后，这份状态就该
   * 与「从没写过」完全一样（否则 `forget` / 容量统计都要额外区分「空记录」）。
   */
  function setDraft(sessionId: string, text: string): void {
    if (!sessionId) return
    if (draftOf(sessionId) === text) return
    const next = { ...drafts.value }
    if (text) next[sessionId] = text
    else delete next[sessionId]
    drafts.value = next
  }

  /** 该会话的滚动位置；`null` = 没有记录（调用方按「贴底」处理） */
  function scrollTopOf(sessionId: string): number | null {
    const v = scrollTops.value[sessionId]
    return typeof v === 'number' && Number.isFinite(v) ? v : null
  }

  /** 记一次滚动位置（非有限值一律忽略——那是布局中间态，不是用户位置） */
  function setScrollTop(sessionId: string, top: number): void {
    if (!sessionId || !Number.isFinite(top)) return
    if (scrollTops.value[sessionId] === top) return
    scrollTops.value = { ...scrollTops.value, [sessionId]: top }
  }

  /** 丢弃某会话的 UI 状态（会话被删除时调用，避免按会话数无限增长） */
  function forget(sessionId: string): void {
    if (!sessionId) return
    if (sessionId in drafts.value) {
      const d = { ...drafts.value }
      delete d[sessionId]
      drafts.value = d
    }
    if (sessionId in scrollTops.value) {
      const s = { ...scrollTops.value }
      delete s[sessionId]
      scrollTops.value = s
    }
  }

  return { draftOf, setDraft, scrollTopOf, setScrollTop, forget }
})
