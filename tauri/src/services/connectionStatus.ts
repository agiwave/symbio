/**
 * 全局连接状态 —— 「断连可见性」的唯一写口（`DE-020` 的「还差什么」表第 3 行）。
 *
 * ## 为什么单独一个模块
 *
 * 状态必须**从真实连接事件来**（`DE-020` 的判据要点），而不是界面定时器猜：事件总线
 * 是唯一知道「连接断了 / 又接上了」的地方，界面只该读结果。把它塞进 `eventBus.ts`
 * 会让那个模块同时是「通道」与「状态广播」两件事，而它今天不依赖任何框架——加一个
 * `ref` 就把 Vue 的响应式引入了通道层。
 *
 * 于是写成一份**极小的状态单元**：三个写函数（首连 / 接上 / 断开）由总线调用，界面
 * 只读 `connectionStatus()`。写方唯一，所以「界面显示断开但总线还在收帧」这种分裂
 * 不可能发生。
 *
 * ## 三态而不是两态
 *
 * `connecting`（首连还没成功）与 `reconnecting`（曾经连上过、现在断了）对用户是
 * 两件事：前者是启动的一瞬，后者是「你的改动可能在丢」。只留 `connected` /
 * `disconnected` 会把前者也渲染成故障。
 */

import { readonly, ref, type Ref } from 'vue'

/** 连接状态闭集。界面据此渲染，不自行推导。 */
export type ConnectionStatus = 'connecting' | 'connected' | 'reconnecting'

const state = ref<ConnectionStatus>('connecting')

/** 只读视图：界面拿到的引用不能写（写口只有下面三个函数）。 */
export function connectionStatus(): Readonly<Ref<ConnectionStatus>> {
  return readonly(state)
}

/** 连接成功（首连与重连都走这里——对用户而言都是「通了」）。 */
export function markConnected(): void {
  state.value = 'connected'
}

/** 连接断开 / 连接失败：总线的重连排期已经开始。 */
export function markReconnecting(): void {
  state.value = 'reconnecting'
}

/**
 * 测试专用：回到初始态。
 *
 * `eventBus` 的 `_resetEventBusForTest` 会调它——状态是跨 spec 文件存活的，
 * 不复位会把上一个文件留下的 `connected` 读成本文件的首连结果。
 */
export function resetConnectionStatus(): void {
  state.value = 'connecting'
}
