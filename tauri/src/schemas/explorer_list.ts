// Corresponding Backend: symbio/src/symbio_core/schemas/explorer_list.rs

export interface FileItem {
  name: string;
  path: string;
  is_dir: boolean;
  size?: number;
  children?: FileItem[];
}

export interface Request {
  path?: string;
  recursive?: boolean;
}

export interface Response {
  path: string;
  items: FileItem[];
}

