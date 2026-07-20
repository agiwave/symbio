// Corresponding Backend: symbio/src/plugins/agent/manager/model.rs
//
// Agent 管理服务相关 schema
// - agent/list   : 列出所有 agent
// - agent/get    : 按 id 获取单个 agent
// - agent/delete : 删除 agent
//
// **注意**：agent/create 需要 cognition_units（认知单元）数组，是复杂结构，
// 适合 seed 脚本批量创建，不适合 UI 表单。因此 AgentView 只支持查看/删除。

export interface AgentProfile {
  id: string
  name: string
  description: string
}

/** agent/list - 列出所有 agent（裸数组） */
export namespace AgentList {
  export interface Request {}
  export type Response = AgentProfile[]
}

/** agent/get - 获取单个 agent */
export namespace AgentGet {
  export interface Request {
    id: string
  }
  export type Response = AgentProfile
}

/** agent/delete - 删除 agent */
export namespace AgentDelete {
  export interface Request {
    id: string
  }
  export interface Response {
    deleted: boolean
    id: string
  }
}
