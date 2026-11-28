import type { AgentChatMessage, AgentRunSummary } from '../../api/agent'
import type { AgentEntityRefSeed } from '../../types/agent'
import { t } from '../../i18n'
import { runEntities, runErrorText, splitUserBlocks, timeAdjacent } from './agentChatUtils'

export interface AgentMessageDecoration {
  run: AgentRunSummary | undefined
  entities: AgentEntityRefSeed[]
  main: string
  folded: string | null
}

export function buildUserRunMap(
  messages: AgentChatMessage[],
  runs: AgentRunSummary[],
): Map<number, AgentRunSummary> {
  const map = new Map<number, AgentRunSummary>()
  const runsAsc = [...runs].reverse()
  let userIdx = 0
  for (let i = 0; i < messages.length; i++) {
    const m = messages[i]
    if (m.role !== 'user') continue
    const run = runsAsc[userIdx]
    if (run && timeAdjacent(run.created_at, m.timestamp)) {
      map.set(i, run)
    }
    userIdx++
  }
  return map
}

export function computeMessageDecorations(
  displayedMessages: AgentChatMessage[],
  runs: AgentRunSummary[],
  fallbackMap: Map<number, AgentRunSummary>,
): Array<AgentMessageDecoration | null> {
  const runById = new Map(runs.map((r) => [r.id, r]))
  return displayedMessages.map((msg, idx) => {
    if (msg.role !== 'user') return null
    let run: AgentRunSummary | undefined
    if (msg.run_id) {
      const byId = runById.get(msg.run_id)
      if (byId && byId.started_at && timeAdjacent(byId.started_at, msg.timestamp)) run = byId
    }
    if (!run) run = fallbackMap.get(idx)
    const split = splitUserBlocks(msg.blocks)
    return {
      run,
      entities: run ? runEntities(run) : [],
      main: split.main,
      folded: split.folded,
    }
  })
}

export function computeRunFailedNote(run: AgentRunSummary | undefined): string | null {
  if (!run) return null
  if (run.status !== 'failed' && run.status !== 'timeout' && run.status !== 'unknown') return null
  return runErrorText(run, t)
}
