/**
 * 会话地址方案的**运行期解析**
 *
 * ## 为什么有这个模块
 *
 * 会话在 VDFS 上的地址是 `<挂载目录>/<sid>/<集合段>/<项 id>`。挂载目录的段名与
 * 集合段的段名都由后端 provider 决定（挂载段 = `PLUGIN_ID_SESSION`，转写段 =
 * `SEG_MESSAGES`，收件箱段 = `SEG_INBOX`），前端**不该**把它们写进自己的地址
 * 模板——写进去就是一份第二真相。
 *
 * 三者都能从数据里认出来，不需要任何字面量：
 * - **挂载目录**：composite 把各 provider **根节点自述里的 `new_type`** 挂到了
 *   挂载点节点上（`plugins/composite/vdfs.rs` 的 `dir_node_full`——它向 provider
 *   发一次 `Stat("")` 取 `new_type`），会话 provider 声明的是
 *   `VdfsNewType::new(VDFS_EXT_SESSION, …)` ⇒ 列虚拟根，找 `new_type.ext`
 *   等于 `'session'` 的子节点即可。
 * - **集合段**：会话内部的子目录里，转写列表的 `kind` 是 `VDFS_KIND_MESSAGES`、
 *   收件箱的 `kind` 是 `VDFS_KIND_INBOX`（稳定 ASCII 协议词，与展示名解耦）
 *   ⇒ 列任一会话，找 `kind` 命中的子节点。
 *
 * ## 挂载目录**不是唯一的一份**，因此按它缓存
 *
 * 子智能体空间（`agent/<id>/…`）是一棵**完整的子树**，内部有自己的 `session`
 * 挂载（`agent/<id>/session`）。于是「会话住在哪个空间」是地址的一部分——
 * 只缓存一份全局挂载目录会让子空间的会话**既送不进也收不到**（它的变更地址
 * 匹配不上那个前缀）。本模块因此按挂载目录缓存方案，而不是全局一份。
 *
 * `ensureSessionMountDir()` 仍是**默认**入口（根清单里那份），现有只碰根空间的
 * 调用方行为不变。
 *
 * ## 两者**可解析性不同**，因此分开缓存
 *
 * 挂载目录只依赖根清单，**永远解析得到**；集合段在会话内部，**至少要有一个会话**
 * 才能推导（且解析的是「挂载目录级」的事实，同一挂载下所有会话同形）。所以
 * `ensureSessionMountDir()` 与 `ensureSessionScheme()` 是两个入口：只碰会话叶子
 * 的操作（列清单 / 读会话 / 删会话 / 改 metadata）用前者，在零会话的全新环境里
 * 照样能跑；碰集合的操作（清空 / 删消息 / 改消息 / 发言）用后者。
 *
 * ## 已知代价（写清楚，别把它当成免费）
 *
 * 解析要列目录（IPC），因此存在一段**引导窗口**：应用刚起来、还没拿到会话清单时
 * 集合段尚未解析，此间到达的转写变更无法路由（会被跳过）。写死常量时不存在这个
 * 窗口。影响被压到最小——`MainLayout` 启动即触发解析，`refreshList` 拿到会话后
 * 再补一次，新建会话后也补一次；而引导窗口内没有任何会话被展示。
 */

import { listVdfs } from './vdfs'
import { READBACK_REASON } from './readback'
import {
  VDFS_EXT_SESSION,
  VDFS_KIND_INBOX,
  VDFS_KIND_MESSAGES,
  isVdfsSystemAddr,
  vdfsJoin,
  type VdfsSessionScheme,
} from '@/schemas/vdfs'
import { vdfsRoot } from '@/schemas/vdfsRoot'
import { logger } from '@/utils/logger'

const MODULE_TAG = 'vdfs-scheme'

/**
 * 节点的**全路径**。
 *
 * 后端两种口径都出现过：挂载点节点给的是段名（`session`，相对于根），
 * 会话节点给的是展示全路径（`<根>/session/abc`）。一律按「已是全路径则直接用，
 * 否则按父地址拼」处理——不加这道判断就会拼出 `<根>/<根>/session/…`
 * （真实事故：读取会话转写直接 404）。
 */
function fullAddr(parent: string, node: { path?: string; name: string }): string {
  const p = node.path
  if (!p) return vdfsJoin(parent, node.name)
  if (isVdfsSystemAddr(p)) return p
  return vdfsJoin(parent, p)
}

let cachedMountDir: string | null = null
/** 挂载目录 → 集合段方案（**按挂载目录**缓存，见模块头） */
const cachedSegs = new Map<string, { messagesSeg: string; inboxSeg: string }>()

/** 已解析的方案（两者都就绪才返回）；引导窗口内返回 `null` */
export function vdfsSessionScheme(mountDir?: string): VdfsSessionScheme | null {
  const dir = mountDir ?? cachedMountDir
  if (!dir) return null
  const segs = cachedSegs.get(dir)
  if (!segs) return null
  return { mountDir: dir, ...segs }
}

/** 清空缓存（测试用） */
export function resetVdfsSessionScheme(): void {
  cachedMountDir = null
  cachedSegs.clear()
}

/**
 * 会话挂载目录（幂等、带缓存）。零会话时也能解析——它只依赖根清单。
 *
 * @throws 根清单里没有声明「可新建 `ext = session`」的子节点时抛错
 *   （那是会话 provider 没注册，属实打实的配置问题）。
 */
export async function ensureSessionMountDir(): Promise<string> {
  if (cachedMountDir) return cachedMountDir
  const resp = await listVdfs(READBACK_REASON.BOOTSTRAP)
  const hit = (resp.items ?? []).find((n) => n.new_type?.ext === VDFS_EXT_SESSION)
  if (!hit) {
    // 注意：`listVdfs` 失败时是**吞掉异常返回空列表**的，所以这里可能是「真没有
    // 挂载点」，也可能是「列目录失败了」。两种都说出来——只报前一种会让人去查
    // 后端注册，而真正的故障在网络 / IPC。
    throw new Error(
      `会话挂载点未找到：根目录下没有声明可新建 ${VDFS_EXT_SESSION} 的子节点` +
        `（根清单为空或列目录失败——后者会被 listVdfs 吞成空列表，见 services/vdfs.ts）`,
    )
  }
  cachedMountDir = fullAddr(vdfsRoot(), hit)
  logger.info(MODULE_TAG, 'mount resolved', cachedMountDir)
  return cachedMountDir
}

/**
 * 完整方案（挂载目录 + 集合段），幂等、带缓存。
 *
 * `mountDir` 省略 = 默认挂载目录（[`ensureSessionMountDir`]）——即根空间那份，
 * 与从前逐字节一致。子智能体空间传 `agent/<id>/session`。
 *
 * @throws 该挂载目录下没有任何会话可供推导集合段时抛错。此时也不可能有转写事件，
 *   调用方（新建会话后 / 清单刷新后 / 进入子空间后）补一次即可。
 */
export async function ensureSessionScheme(mountDir?: string): Promise<VdfsSessionScheme> {
  const dir = mountDir ?? (await ensureSessionMountDir())
  const cached = cachedSegs.get(dir)
  if (cached) return { mountDir: dir, ...cached }
  const segs = await resolveSegs(dir)
  cachedSegs.set(dir, segs)
  return { mountDir: dir, ...segs }
}

/** 旧名（默认挂载目录那份）。保留给只碰根空间的调用方，语义与从前一致。 */
export async function ensureVdfsSessionScheme(): Promise<VdfsSessionScheme> {
  return ensureSessionScheme()
}

/**
 * 集合段：列一个会话的内部子项，取 `kind` 命中 `VDFS_KIND_MESSAGES` /
 * `VDFS_KIND_INBOX` 的那两个。
 *
 * 按 `kind` 而不是名字——段名是**展示名**（可能随文案调整），`kind` 才是对外
 * 承诺的标识。这也是后端给这些目录显式声明 kind 的唯一理由。
 *
 * 两个段**一次列目录同时取**：它们住在同一个父下，分两次解析就是两次 IPC
 * 换同一份列表。
 */
async function resolveSegs(mountDir: string): Promise<{ messagesSeg: string; inboxSeg: string }> {
  const sessions = (await listVdfs(READBACK_REASON.BOOTSTRAP, mountDir)).items ?? []
  const first = sessions.find((n) => n.ext === VDFS_EXT_SESSION)
  if (!first) {
    throw new Error(`集合段未解析：${mountDir} 下没有任何会话可供推导`)
  }
  const sessionPath = fullAddr(mountDir, first)
  const children = (await listVdfs(READBACK_REASON.BOOTSTRAP, sessionPath)).items ?? []
  const messages = children.find((n) => n.kind === VDFS_KIND_MESSAGES)
  const inbox = children.find((n) => n.kind === VDFS_KIND_INBOX)
  if (!messages) {
    throw new Error(`转写段未找到：${sessionPath} 下没有 kind=${VDFS_KIND_MESSAGES} 的子节点`)
  }
  if (!inbox) {
    throw new Error(`收件箱段未找到：${sessionPath} 下没有 kind=${VDFS_KIND_INBOX} 的子节点`)
  }
  return { messagesSeg: messages.name, inboxSeg: inbox.name }
}
