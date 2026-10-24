/**
 * 聊天消息滚动 composable —— **滚动位置表示的唯一实现**
 *
 * 提供智能的滚动策略，包括：
 * - 自动滚动到底部
 * - 跟踪用户是否在底部
 * - 仅在用户在底部时自动滚动
 *
 * ## 为什么对外只给「距底部」，不给 `scrollTop`
 *
 * 会话流是一个**尾部在长的**列表，而且渲染层还有一道窗口（`useTranscriptWindow`）：
 * DOM 里铺的最近若干条根节点。于是**从顶部量的偏移没有稳定含义**——窗口起点变了
 * （切回来时铺的条数不同）、上方内容变了（一条工具输出展开、一段思考被折叠），
 * 同一个数字指的就不再是原来那条消息。而「离最新一条多远」在这两件事上都稳定：
 * 最新的一条恒在 DOM 里，上方怎么变都不动它的下边界。
 *
 * 所以这里 `scrollTop` 只作为**写入浏览器的手段**存在，读出来的一律换算成距底。
 * 阈值判据（`isUserAtBottom`）与记忆、还原共用同一个 `gapFromBottom`——分两处写
 * 就会出现「界面上算在底部，存下来却按不在底部还原」的错位。
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
   * 当前位置离底部多少像素；容器还没挂上时给 `null`。
   *
   * `null` 与 `0` 必须是两个值：`0` 是「正贴底」这个**事实**，`null` 是「还没有可量的
   * 容器」——把后者当 0 存下来，切回会话时就会凭空得到一个「用户当时在底部」的记录。
   */
  function gapFromBottom(): number | null {
    const el = containerRef.value
    if (!el) return null
    return el.scrollHeight - el.scrollTop - el.clientHeight
  }

  /**
   * 回到「离底 `gap` 像素」处。
   *
   * 内容比记下的位置短（会话被截断、消息被删）时落到**当前能到的最上处**而不是失败：
   * 用户要的是「往上一些」，不是精确到像素的那一条。
   */
  function scrollToGap(gap: number) {
    const el = containerRef.value
    if (!el) return
    el.scrollTop = Math.max(0, el.scrollHeight - el.clientHeight - gap)
  }

  /**
   * 处理滚动事件，更新 isUserAtBottom 状态
   */
  function handleScroll() {
    const gap = gapFromBottom()
    if (gap === null) return

    isUserAtBottom.value = gap < SCROLL_THRESHOLD
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
    scrollToGap,
    gapFromBottom,
    handleScroll,
    smartScroll
  }
}
