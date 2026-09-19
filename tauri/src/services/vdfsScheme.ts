/**
 * 会话地址方案的**运行期解析**
 *
 * ## 为什么有这个模块
 *
 * 会话在 VDFS 上的地址是 `<挂载目录>/<sid>/<转写段>/<mid>`。两个段名都由后端
 * provider 决定（挂载段 = `PLUGIN_SESSION`，转写段 = `SEG_MESSAGES`），
 * 前端**不该**把它们写进自己的地址模板——写进去就是一份第二真相。
 *
 * 两者都能从数据里认出来，不需要任何字面量：
 * - **挂载目录**：composite 把 `root_new_types()` 挂到了挂载点节点上
 *   （`plugins/composite/vdfs.rs` 的 `dir_node`），会话 provider 声明的是
 *   `VdfsNewType::new(VDFS_EXT_SESSION, …)` ⇒ 列 `.vdfs` 根，找 `new_types`
 *   里含 `ext === 'session'` 的子节点即可。
 * - **转写段**：会话内部的子目录里，转写列表的 `kind` 是 `VDFS_KIND_MESSAGES`
 *   （稳定 ASCII 协议词，与展示名解耦）⇒ 列任一会话，找 `kind` 命中的子节点。
 *
 * ## 两者**可解析性不同**，因此分开缓存
 *
 * 挂载目录只依赖根清单，**永远解析得到**；转写段在会话内部，
 * **至少要有一个会话**才能推导。所以 `ensureSessionMountDir()` 与
 * `ensureVdfsSessionScheme()` 是两个入口：只碰会话叶子的操作（列清单 / 读会话 /
 * 删会话 / 改 metadata）用前者，在零会话的全新环境里照样能跑；
 * 碰转写的操作（清空 / 删消息 / 改消息 / 事件路由）用后者。
 *
 * ## 已知代价（写清楚，别把它当成免费）
 *
 * 解析要列目录（IPC），因此存在一段**引导窗口**：应用刚起来、还没拿到会话清单时
 * 转写段尚未解析，此间到达的转写变更无法路由（会被跳过）。写死常量时不存在这个
 * 窗口。影响被压到最小——`MainLayout` 启动即触发解析，`refreshList` 拿到会话后
 * 再补一次，新建会话后也补一次；而引导窗口内没有任何会话被展示。
 */

import { listVdfs } from './vdfs'
import {
  VDFS_EXT_SESSION,
  VDFS_KIND_MESSAGES,
  VDFS_ROOT,
  vdfsJoin,
  type VdfsSessionScheme,
} from '@/schemas/vdfs'
import { logger } from '@/utils/logger'

const MODULE_TAG = 'vdfs-scheme'

/**
 * 节点的**全路径**。
 *
 * 后端两种口径都出现过：挂载点节点给的是段名（`session`，相对于根），
 * 会话节点给的是展示全路径（`.vdfs/session/abc`）。一律按「已是全路径则直接用，
 * 否则按父地址拼」处理——不加这道判断就会拼出 `.vdfs/.vdfs/session/…`
 * （真实事故：读取会话转写直接 404）。
 */
function fullAddr(parent: string, node: { path?: string; name: string }): string {
  const p = node.path
  if (!p) return vdfsJoin(parent, node.name)
  if (p === VDFS_ROOT || p.startsWith(`${VDFS_ROOT}/`)) return p
  return vdfsJoin(parent, p)
}

let cachedMountDir: string | null = null
let cachedMessagesSeg: string | null = null

/** 已解析的方案（两者都就绪才返回）；引导窗口内返回 `null` */
export function vdfsSessionScheme(): VdfsSessionScheme | null {
  if (!cachedMountDir || !cachedMessagesSeg) return null
  return { mountDir: cachedMountDir, messagesSeg: cachedMessagesSeg }
}

/** 清空缓存（测试用） */
export function resetVdfsSessionScheme(): void {
  cachedMountDir = null
  cachedMessagesSeg = null
}

/**
 * 会话挂载目录（幂等、带缓存）。零会话时也能解析——它只依赖根清单。
 *
 * @throws 根清单里没有声明「可新建 `ext = session`」的子节点时抛错
 *   （那是会话 provider 没注册，属实打实的配置问题）。
 */
export async function ensureSessionMountDir(): Promise<string> {
  if (cachedMountDir) return cachedMountDir
  const resp = await listVdfs(VDFS_ROOT)
  const hit = (resp.items ?? []).find((n) =>
    (n.new_types ?? []).some((t) => t.ext === VDFS_EXT_SESSION),
  )
  if (!hit) {
    // 注意：`listVdfs` 失败时是**吞掉异常返回空列表**的，所以这里可能是「真没有
    // 挂载点」，也可能是「列目录失败了」。两种都说出来——只报前一种会让人去查
    // 后端注册，而真正的故障在网络 / IPC。
    throw new Error(
      `会话挂载点未找到：${VDFS_ROOT} 下没有声明可新建 ${VDFS_EXT_SESSION} 的子节点` +
        `（根清单为空或列目录失败——后者会被 listVdfs 吞成空列表，见 services/vdfs.ts）`,
    )
  }
  cachedMountDir = fullAddr(VDFS_ROOT, hit)
  logger.info(MODULE_TAG, 'mount resolved', cachedMountDir)
  return cachedMountDir
}

/**
 * 完整方案（挂载目录 + 转写段），幂等、带缓存。
 *
 * @throws 没有任何会话可供推导转写段时抛错。此时也不可能有转写事件，
 *   调用方（新建会话后 / 清单刷新后）补一次即可。
 */
export async function ensureVdfsSessionScheme(): Promise<VdfsSessionScheme> {
  const mountDir = await ensureSessionMountDir()
  if (cachedMessagesSeg) return { mountDir, messagesSeg: cachedMessagesSeg }
  cachedMessagesSeg = await resolveMessagesSeg(mountDir)
  return { mountDir, messagesSeg: cachedMessagesSeg }
}

/**
 * 转写段：列一个会话的内部子项，取 `kind` 命中 `VDFS_KIND_MESSAGES` 的那个。
 *
 * 按 `kind` 而不是名字——段名是**展示名**（可能随文案调整），`kind` 才是对外
 * 承诺的标识。这也是后端给这个目录显式声明 kind 的唯一理由。
 */
async function resolveMessagesSeg(mountDir: string): Promise<string> {
  const sessions = (await listVdfs(mountDir)).items ?? []
  const first = sessions.find((n) => n.ext === VDFS_EXT_SESSION)
  if (!first) {
    throw new Error(`转写段未解析：${mountDir} 下没有任何会话可供推导`)
  }
  const sessionPath = fullAddr(mountDir, first)
  const children = (await listVdfs(sessionPath)).items ?? []
  const hit = children.find((n) => n.kind === VDFS_KIND_MESSAGES)
  if (!hit) {
    throw new Error(`转写段未找到：${sessionPath} 下没有 kind=${VDFS_KIND_MESSAGES} 的子节点`)
  }
  return hit.name
}
