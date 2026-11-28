export interface AgentEntityRefSeed {
  kind: 'source' | 'release'
  id: number
}

export interface AgentWorkspaceSeed {
  entities?: AgentEntityRefSeed[]
}
