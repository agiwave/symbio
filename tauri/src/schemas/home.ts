// Corresponding Backend: symbio/src/plugins/home/schemas.rs

/**
 * home 控制面四条臂的载荷契约。
 *
 * **命名空间 = 后端模块名**（`home_reload` / `home_get_workspace` /
 * `home_set_workspace` / `home_get_homedir`），**类型名 = 后端结构体名**——那一个
 * 文件里四个模块各有一个 `Response`，靠同名限定区分，两侧用同一个词，读代码的人
 * 不做换算（[D-112](../../../docs/003-design/005-协议规范.md "DE-005-协议规范") 决策 2）。
 */
export namespace home_reload {
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
}

export namespace home_get_workspace {
  /** home/get_workspace 响应（工作区未配置时 `workdir` 与 `expanded_path` 都是空串） */
  export interface Response {
    workdir: string
    expanded_path: string
    recent_workspaces: string[]
  }
}

export namespace home_set_workspace {
  /** home/set_workspace 请求 */
  export interface Request {
    path: string
  }

  /** home/set_workspace 响应 */
  export interface Response {
    workdir: string
    expanded_path: string
    recent_workspaces: string[]
  }
}

export namespace home_get_homedir {
  /** home/get_homedir 响应 */
  export interface Response {
    homedir: string
    bootstrap_path: string
  }
}
