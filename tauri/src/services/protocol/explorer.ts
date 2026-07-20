export interface FileItem {
  name: string;
  path: string;
  is_dir: boolean;
  size?: number;
  children?: FileItem[];
}

export namespace Explorer {
  // Input Types
  export type Input =
    | { action: 'list'; path?: string; recursive?: boolean }
    | { action: 'get'; path: string }
    | { action: 'read'; path: string }
    | { action: 'write'; path: string; content: string }
    | { action: 'exists'; path: string }
    | { action: 'config_get' }
    | { action: 'config_set'; show_hidden?: boolean; file_filter?: string[] }
    | { action: 'config_schema' }
    | { action: 'start_watch' }
    | { action: 'stop_watch' };

  // Response Types
  export interface ListResponse {
    data: {
      path: string;
      items: FileItem[];
    };
  }

  export interface ReadResponse {
    data: {
      path: string;
      content: string;
      file_type: string;
      size?: number;
    };
  }

  export interface WriteResponse {
    data: {
      path: string;
      size: number;
    };
  }

  export interface ExistsResponse {
    data: {
      path: string;
      exists: boolean;
    };
  }
}
