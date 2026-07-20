// Corresponding Backend: symbio/src/symbio_core/schemas/explorer_event.rs

export enum ExplorerEventType {
  DirChanged = 'browser/dir_changed',
  FileChanged = 'browser/file_changed',
  WatchStarted = 'watch_started',
  WatchStopped = 'watch_stopped',
  ListResult = 'list_result',
}

export interface FileChangeEvent {
  event: 'file_change';
  path: string;
  kind: string;
}

export interface WatcherErrorEvent {
  event: 'watcher_error';
  message: string;
}

export type ExplorerEvent = FileChangeEvent | WatcherErrorEvent;

export interface EventInput {
  type: ExplorerEventType;
  data: ExplorerEvent;
}

export interface StatusInput {
  type: ExplorerEventType;
  data: any;
}

