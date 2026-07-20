// Corresponding Backend: symbio/src/symbio_core/schemas/home_reload.rs

/** home/reload 请求 */
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
