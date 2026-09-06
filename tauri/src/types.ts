//! TypeScript type definitions (UI-only types)

export interface SchemaProperty {
  type: string
  description?: string
  default?: unknown
  enum_values?: unknown[]
}

// Re-export from schemas for backward compatibility
export type { MessageContent, ContentPart } from './schemas/chat_message'

/** 工具风险等级（原 schemas/tools_policy，仅本文件使用，内联） */
export type RiskLevel = 'low' | 'medium' | 'high'

/** Frontend-only AI types: Agent models（原 schemas/model_types，仅本文件使用，内联） */
export interface AgentProfile {
  id: string
  name: string
  description: string
  knowledge: string[]
  experience: string[]
  skill: string[]
  judgment: string[]
  strategy: string[]
  intuition: string[]
  emotion: string[]
  context_messages: number
}


/** Image attachment (frontend-only UI type) */
export interface ImageAttachment {
  /** Base64 image data (without data:image/xxx;base64, prefix) */
  base64: string
  /** Image MIME type */
  mimeType: string
  /** Image file name */
  fileName?: string
  /** Thumbnail URL (for preview) */
  thumbnailUrl?: string
}
