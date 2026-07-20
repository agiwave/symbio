//! TypeScript type definitions (UI-only types)

export interface SchemaProperty {
  type: string
  description?: string
  default?: unknown
  enum_values?: unknown[]
}

// Re-export from schemas for backward compatibility
export type { RiskLevel } from './schemas/tools_policy'
export type { MessageContent, ContentPart } from './schemas/chat_message'
export type { AgentProfile } from './schemas/model_types'


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
