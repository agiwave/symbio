/**
 * 聊天消息滚动 composable
 * 
 * 提供智能的滚动策略，包括：
 * - 自动滚动到底部
 * - 跟踪用户是否在底部
 * - 仅在用户在底部时自动滚动
 */

import { ref, type Ref } from 'vue'

export function useChatScroll(containerRef: Ref<HTMLElement | null>) {
  // 跟踪用户是否在底部
  const isUserAtBottom = ref(true)
  // 滚动阈值（像素）
  const SCROLL_THRESHOLD = 50

  /**
   * 滚动到底部
   */
  function scrollToBottom() {
    if (containerRef.value) {
      containerRef.value.scrollTop = containerRef.value.scrollHeight
    }
  }

  /**
   * 处理滚动事件，更新 isUserAtBottom 状态
   */
  function handleScroll() {
    if (!containerRef.value) return

    const { scrollTop, scrollHeight, clientHeight } = containerRef.value
    const distanceFromBottom = scrollHeight - scrollTop - clientHeight
    isUserAtBottom.value = distanceFromBottom < SCROLL_THRESHOLD
  }

  /**
   * 智能滚动：仅当用户在底部附近时才滚动
   */
  function smartScroll() {
    if (isUserAtBottom.value) {
      scrollToBottom()
    }
  }

  return {
    isUserAtBottom,
    scrollToBottom,
    handleScroll,
    smartScroll
  }
}
