//! TypeScript 类型定义

export interface PluginMeta {
  name: string
  description: string
  version?: string
  input?: any
  output?: any
  author?: string
}

export interface JsonSchema {
  type: string
  properties?: Record<string, SchemaProperty>
  required?: string[]
  items?: JsonSchema
  description?: string
}

export interface SchemaProperty {
  type: string
  description?: string
  default?: any
  enum_values?: any[]
}

export interface PluginFactoryInfo {
  name: string
  description: string
  input_schema?: any
  output_schema?: any
}
