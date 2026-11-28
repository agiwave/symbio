/**
 * 键盘输入的**唯一 IME 判据**。
 *
 * 中文等输入法在组合态里按 Enter 是「选词 / 上屏」，不是「提交」。把这个判据
 * 写在每个 `@keydown.enter` 旁边，必然有一处漏掉（消息输入、重命名、搜索框、
 * 路径输入各自演化），而漏掉的表现是**用户正在打的字被直接提交出去**。
 *
 * `isComposing` 是标准字段；`keyCode === 229` 覆盖旧 WebView 在组合期间只报
 * 229 的情形（Tauri WebView2 / WebKit 历史版本）。
 *
 * 注意：组合态的 Enter 还**不能** `preventDefault`——那会阻止输入法上屏。
 * 因此判据必须先于 `preventDefault`，模板里也应写成 `@keydown.enter` 再在
 * 处理函数内决定是否拦截，而不是直接挂 `.prevent`。
 */
export function isImeComposing(e: KeyboardEvent): boolean {
  return e.isComposing || e.keyCode === 229
}
