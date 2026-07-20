// Corresponding Backend: symbio/src/symbio_core/schemas/shell_execute.rs

export interface Request {
  command: string;
  approved?: boolean;
}

export interface Response {
  exit_code?: number;
  output: string;
  risk_level: string;
}

