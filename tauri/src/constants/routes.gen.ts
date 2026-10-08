/**
 * 控制面路由常量 —— **由后端生成，请勿手改**。
 *
 * ⚠️ 本文件由 `scripts/gen-routes-ts.mjs` 从后端 `route()` 的 `match` 臂生成，
 * 门禁会自动重新生成（`scripts/gate.d/60-facts.mjs`）。在这里手改一行，下一次门禁
 * 就会把它覆盖掉——真正要改的地方是后端那条臂。
 *
 * 地址的**注册处**就是插件里的 `match` 臂（提取实现：`scripts/route-facts.mjs`），
 * 所以这份清单没有第二份真相，也不可能与后端不一致。
 *
 * ## 这里不包含什么
 *
 * - `vdfs/*` 的操作词：它们按 `VDFS_OPS` 动态校验，契约与响应类型同处
 *   `schemas/vdfs.ts`（定义权由 `mechanism-audit` 的 M-007 钉住）。
 * - 动态分发的地址（`local/<工具短名>`、容器挂载名）：静态提取不到，见
 *   `docs/CURRENT.md` §1 标「（动态）」的那些行。
 *
 * 命名与后端 `symbio_core/plugin/route.rs` 同构：`ROUTE_` + 地址大写（`/` → `_`），
 * 同名同值，跨栈检索只需一个标识符。
 */

// ==================== classify ====================

export const ROUTE_CLASSIFY_DECIDE = 'classify/decide'

// ==================== compose ====================

export const ROUTE_COMPOSE_WORDING = 'compose/wording'

// ==================== event_bus ====================

export const ROUTE_EVENT_BUS_PING = 'event_bus/ping'
export const ROUTE_EVENT_BUS_SUBSCRIBE = 'event_bus/subscribe'

// ==================== gateway ====================

export const ROUTE_GATEWAY_STATUS = 'gateway/status'

// ==================== home ====================

export const ROUTE_HOME_GET_HOMEDIR = 'home/get_homedir'
export const ROUTE_HOME_RELOAD = 'home/reload'

// ==================== hook ====================

export const ROUTE_HOOK_FIRE = 'hook/fire'
export const ROUTE_HOOK_LIST = 'hook/list'
export const ROUTE_HOOK_REGISTER = 'hook/register'

// ==================== session ====================

export const ROUTE_SESSION_CHAT_ABORT = 'session/chat/abort'
export const ROUTE_SESSION_CHAT_SEND = 'session/chat/send'
export const ROUTE_SESSION_STATS = 'session/stats'

// ==================== skill ====================

export const ROUTE_SKILL_EXECUTE = 'skill/execute'

// ==================== telegram ====================

export const ROUTE_TELEGRAM_GET_UPDATES = 'telegram/get_updates'
export const ROUTE_TELEGRAM_SEND = 'telegram/send'
export const ROUTE_TELEGRAM_SET_CHAT_ID = 'telegram/set_chat_id'
export const ROUTE_TELEGRAM_START_LISTENER = 'telegram/start_listener'
export const ROUTE_TELEGRAM_STATUS = 'telegram/status'
export const ROUTE_TELEGRAM_STOP_LISTENER = 'telegram/stop_listener'

// ==================== work ====================

export const ROUTE_WORK_GET_WORKSPACE = 'work/get_workspace'
export const ROUTE_WORK_SET_WORKSPACE = 'work/set_workspace'
