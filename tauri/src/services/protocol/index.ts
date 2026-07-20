export * from './explorer';
export * from './work';

export namespace Home {
  export type Input =
    | { action: 'root' }
    | { action: 'save_config' };

  export interface RootResponse {
    name: string;
    children: string[];
  }
}

export namespace Agent {
  // Placeholder for agent types
  export type Input = any;
  export type Event = any;
}
