// Frontend-only AI types: Agent models

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
