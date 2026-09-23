/**
 * 机制原生取值原语 —— 后端唤不起原生对话框，故这一小组通用取值能力由前端实现。
 *
 * ## 它是什么，不是什么
 *
 * 它是**机制级原语**（目录 / 文件选择），不含任何业务语义；闭集与线上面
 * （`DetailField.pick`）的唯一定义处是 `schemas/vdfs-form.DETAIL_PICKS`
 * （后端对应 `symbio_core/schemas/detail.rs` 的 `DETAIL_PICK_*`）。
 *
 * 取值结果由调用方按自己的绑定写入字段 / 载荷——本模块**不知道**谁会用它。
 *
 * ## 为什么收在一处
 *
 * 这个原语此前有三份独立实现（旧选项机制一份、两个组件各一份直接
 * import `plugin-dialog`），彼此无引用关系。闭集一旦是跨栈契约，
 * 「取值怎么做」就该只有一处——否则新增一个原语要改三处，而漏改不会有守卫报红。
 */

import { open } from '@tauri-apps/plugin-dialog'
import { DETAIL_PICK_DIRECTORY, DETAIL_PICKS, type DetailPick } from '@/schemas/vdfs-form'

/**
 * 唤起原生选择对话框，返回绝对路径；用户取消或取值不在闭集内返回 `null`。
 *
 * 未登记的取值返回 `null` 而非抛错：后端下发的值未必守约，页面不该因此白屏。
 */
export async function pickNative(kind: DetailPick | undefined): Promise<string | null> {
  if (!kind || !DETAIL_PICKS.includes(kind)) return null
  const directory = kind === DETAIL_PICK_DIRECTORY
  const picked = await open({
    directory,
    multiple: false,
    title: directory ? '选择目录' : '选择文件',
  })
  if (typeof picked === 'string') return picked
  if (Array.isArray(picked) && typeof picked[0] === 'string') return picked[0]
  return null
}
