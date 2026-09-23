// Corresponding Backend: symbio/src/symbio_core/schemas/home_reload.rs

/** home/reload 请求 */
// dead-code-allow R-001: 与后端 symbio_core/schemas/home_reload.rs 的请求半边对应，保留以标明契约
export interface Request {
  /** 目标 homedir（可选；不传则仅重新加载当前 homedir） */
  homedir?: string
}

/** home/reload 响应 */
export interface Response {
  old_homedir: string
  new_homedir: string
  reloaded_plugins: number
  homedir_changed: boolean
  bootstrap_path: string
}
