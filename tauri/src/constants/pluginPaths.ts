/**
 * 插件路径常量（统一管理）
 *
 * 注意：所有路径都使用 `worker/` 前缀，路由走 worker composite。
 * 历史上有 `session/...`（无前缀）的写法在某些上下文也能工作
 * （session 插件也挂在 home composite 下），但已统一收敛到 worker 路径。
 *
 * 一旦引入新插件或新能力，路径常量要在此处注册，便于全局检索。
 */

const W = 'worker' as const

/** 会话插件根路径 */
export const SESSION_PATH = `${W}/session` as const

/** 聊天能力根路径（send / abort） */
export const CHAT_PATH = `${SESSION_PATH}/chat` as const

/** 聊天子能力 */
export const CHAT_SEND = `${CHAT_PATH}/send` as const
export const CHAT_ABORT = `${CHAT_PATH}/abort` as const
