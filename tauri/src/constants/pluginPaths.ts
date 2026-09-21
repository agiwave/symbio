/**
 * 插件路径常量（**控制面插件路由名的唯一登记处**）
 *
 * ## 为什么要有这张表
 *
 * 路由名是**后端的地址**，前端本不该持有。这件事分两半，只有一半做到了：
 *
 * - **地址**做到了：前端只认 `<根>/<挂载点>/…` 这种由后端下发的展示地址，
 *   连根叫什么都在启动期取回（`schemas/vdfsRoot`），改挂载名前端零改动。
 * - **控制面操作名**做不到：homedir 切换 / 事件总线订阅 / 网关出站这些动作
 *   没有「可下发」的等价物，前端必须把它们的路由名写死。
 *
 * 既然必须写死，就让这份知识**可检索**：新增一个控制面路由，在此处登记一行，
 * 全仓据此可查；调用点只引常量，不写字面量。
 *
 * ## VDFS 的常量**不**在此表（有门禁钉住，不要往这搬）
 *
 * `vdfs/list` 这类 op 与 `VDFS_*` 常量一律定义在 `schemas/vdfs.ts` ——
 * `scripts/mechanism-audit.mjs` 的 **M-007** 禁止在别处再定义一份（导入使用允许）。
 * 理由：它们是 VDFS **契约**的一部分，必须与响应类型、变更词汇、地址代数同处
 * 一个文件，否则契约被拆成两半。所以本表与它不是「同一个登记处的两半」，
 * 而是两个层次：**这里是控制面路由，那里是 VDFS 契约**。
 *
 * ## 前缀口径（容易搞错，写清楚）
 *
 * - `worker/` 前缀 = 走 worker composite 的**会话域**路由（session 及它的子能力
 *   chat / options）。历史上也写过无前缀的 `session/...`（session 插件同时挂在
 *   home composite 下），已统一收敛到 `worker/`。
 * - **其余插件按插件名直接寻址**（`home/...` / `work/...` / `event_bus/...` /
 *   `gateway/...`）：它们挂在 home composite 下，不带 `worker/` 前缀。
 *   因此「所有路径都用 worker/ 前缀」是**错的**，本文件里两类共存。
 *
 * 路由的权威清单（含后端侧）见 `docs/reference/ROUTES.md`。
 */

const W = 'worker' as const

/** 会话插件根路径 */
export const SESSION_PATH = `${W}/session` as const

/** 聊天能力根路径（send / abort） */
export const CHAT_PATH = `${SESSION_PATH}/chat` as const

/** 聊天子能力 */
export const CHAT_SEND = `${CHAT_PATH}/send` as const
export const CHAT_ABORT = `${CHAT_PATH}/abort` as const

/**
 * 会话**转写实时流**（消息的唯一实时通道）。
 *
 * 长连接订阅：后端按 `NodeEvent`（`session_id` + 流内单调 `seq` + 显式操作）逐帧
 * 下发在途转写；历史面（落库转写 / `消息` 目录投影）仍走 VDFS 读。
 * 消费端 `services/transcriptStream.ts`；服务端 `plugins/session/plugin.rs::handle_stream_subscribe`。
 */
export const SESSION_STREAM = `${SESSION_PATH}/stream` as const

/**
 * 级联选项机制（选项宿主 = session 插件）。
 *
 * 根选项列表与子层共用一个端点（`parent` 参数区分），与 VDFS
 * `vdfs/list` 的树懒加载同构。
 */
export const OPTIONS_PATH = `${SESSION_PATH}/options` as const
export const OPTIONS_LIST = `${OPTIONS_PATH}/list` as const

// ==================== 事件总线 ====================

/** 事件总线订阅（长连接；`services/eventBus.ts` 用它建流） */
export const EVENT_BUS_SUBSCRIBE = 'event_bus/subscribe'

// ==================== 系统目录 / 工作区（home 插件） ====================

/** 当前系统目录（homedir）信息 */
export const HOME_GET_HOMEDIR = 'home/get_homedir'
/** 切换系统目录并热重载子插件 */
export const HOME_RELOAD = 'home/reload'
/** 当前工作区（workdir）详情 */
export const WORK_GET_WORKSPACE = 'work/get_workspace'
/** 设置当前工作区 */
export const WORK_SET_WORKSPACE = 'work/set_workspace'

/**
 * 后端返回的**默认工作区**。
 *
 * 它不代表真实目录，而是后端「还没配过 workdir」时的占位值——
 * 前端据此**不把它记成最近使用目录**（否则新建会话会默认落到一个不存在的路径）。
 */
export const DEFAULT_WORKSPACE = '~/projects'

// ==================== 网关 ====================

/**
 * 网关插件名。
 *
 * 它的特殊之处：`gateway/*` 是**控制面**接口（读写本机网关设置 / 出站协议），
 * 因此必须始终走本机 native 传输——若跟随当前出站配置走 http，就会去读远端实例的
 * 网关设置，既看不到本机配置，也切不回 native（死锁）。判定见 `services/plugin.ts`。
 */
export const GATEWAY_PATH = 'gateway'
