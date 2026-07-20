// Corresponding Backend: symbio/src/symbio_core/schemas/work_set_workspace.rs

export interface Request {
  path: string;
}

export interface Response {
  workdir: string;
  expanded_path: string;
  recent_workspaces: string[];
  status: string;
}
