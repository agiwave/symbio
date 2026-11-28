/**
 * 全局「新建当前目录允许的类型」请求。
 *
 * 快捷键（Ctrl/Cmd+N）与命令面板都住在应用外壳，而「新建」真正的执行者是
 * 当前页面的工作台（它才知道当前目录声明了哪种 `new_type`）。此前那两个入口
 * 只做路由跳转，靠「进入目录且未选中任何项」的自动草稿兜底——于是用户正停在
 * 某条会话上时按 Ctrl+N，路由原地不动、草稿不铺，按钮像失灵一样（实测可复现）。
 *
 * 这里只定义**一条**请求通道：发起方给目标数据地址，工作台比对后决定要不要
 * 执行。地址不一致时不抢别页的草稿；不带地址则交给当前页（保留键盘直建的语义）。
 */
export const NEW_RESOURCE_EVENT = 'symbio:new-resource'

export function requestNewResource(addr?: string): void {
  window.dispatchEvent(new CustomEvent<string | undefined>(NEW_RESOURCE_EVENT, { detail: addr }))
}
