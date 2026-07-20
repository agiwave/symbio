/**
 * Corresponding Backend: symbio/src/symbio_core/types.rs
 */

export interface ToolFunction {
  name: string
  arguments: string
}

export interface ToolCall {
  id: string
  kind?: string
  function: ToolFunction
  result?: string
  success?: boolean
}

export interface ToolDefinition {
  name: string
  description: string
  parameters: any
}
