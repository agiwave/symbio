/**
 * VDFS **根地址锚点** —— 全前端唯一一处「根叫什么」的持有地
 *
 * ## 为什么是运行期数据
 *
 * 根名是后端 vdfs 插件的挂载规则（`plugins/vdfs/fs.rs::VDFS_ADDR_ROOT`，
 * 全后端唯一的字面量），前端**不该写死它**：后端改名（如 `<根>` → `<根>v3`）
 * 前端必须零改动照常工作。于是根地址与挂载目录、转写段名一样，是**启动期从
 * 后端取回的数据**（`vdfs/root` 操作，见 `services/vdfs.ensureVdfsRoot`）。
 *
 * ## 为什么是一个模块级变量而不是响应式 ref
 *
 * 引导发生在**挂载之前**（`main.ts` 顶层 await），此后只读——不存在
 * 「中途变化需要触发重渲染」的场景，响应式包装只会引入假信号风险。
 * 路径代数（`schemas/vdfs` / `schemas/vdfsAddress`）读它，签名不变。
 *
 * ## 未引导时
 *
 * `vdfsRoot()` 返回空串：地址代数全部退化为「无虚拟半」——判虚拟恒假、
 * 拼接退化为裸相对地址。这是**可诊断的降级**（首页会显示物理半），
 * 不是错误；真正的引导失败会在 `ensureVdfsRoot` 里记日志。
 */

let anchor = ''

/** 已引导的根地址（未引导时为空串） */
export function vdfsRoot(): string {
  return anchor
}

/** 登记根地址（引导入口调用；幂等，重复登记以最后一次为准） */
export function setVdfsRoot(root: string): void {
  anchor = (root ?? '').trim().replace(/\/+$/, '')
}

/** 是否已引导（测试与诊断用） */
export function vdfsRootResolved(): boolean {
  return anchor !== ''
}

/** 清空锚点（测试用） */
export function resetVdfsRoot(): void {
  anchor = ''
}
