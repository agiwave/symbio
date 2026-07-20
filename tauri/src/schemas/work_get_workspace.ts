// Corresponding Backend: symbio/src/symbio_core/schemas/work_get_workspace.rs

export interface Response {
  workdir: string;
  expanded_path: string;
  recent_workspaces: string[];
}

