// Corresponding Backend: symbio/src/symbio_core/schemas/explorer_read.rs

export interface Request {
  path: string;
}

export interface ReadData {
  path: string;
  content: string;
  file_type: string;
  size?: number;
}

export type Response = ReadData;

