//! TypeScript type definitions (UI-only types)

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
