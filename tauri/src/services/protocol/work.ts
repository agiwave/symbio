export namespace Work {
  // Input Types
  export type Input =
    | { action: 'get_workspace' }
    | { action: 'set_workspace'; path: string }
    | { action: 'config_get' }
    | { action: 'config_set'; workdir?: string; recent_workspaces?: string[] }
    | { action: 'config_schema' }
    | { action: 'workdir' };

  // Response Types
  export interface WorkspaceResponse {
    success: boolean;
    workdir: string;
    expanded_path?: string;
  }
}
