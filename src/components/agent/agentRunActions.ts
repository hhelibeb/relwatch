import { nextTick, type Ref } from 'vue'
import {
  cancelAgentRun,
  runAgentJob,
  type AgentModelRef,
  type AgentRunSummary,
} from '../../api/agent'
import type { Source } from '../../api/sources'
import type { ReleaseInfo } from '../../api/releases'
import type { AgentEntityRefSeed } from '../../types/agent'
import { t } from '../../i18n'
import { skillShortName } from '../../utils'
import { track } from '../../composables/useUsageTracking'
import { extractErrorMessage } from '../../services/feedback'
import { escapeRegExp, runEntities, runFiles, runModel } from './agentChatUtils'
import type { SessionChatState } from './agentSessionCache'

export function parseInlineRefs(
  text: string,
  skillPath: string | null,
): { entities: AgentEntityRefSeed[]; cleaned: string } {
  const found: AgentEntityRefSeed[] = []
  let cleaned = text.replace(/\[\[(source|release):(\d+)\]\]/g, (_m, kind: string, id: string) => {
    found.push({ kind: kind as 'source' | 'release', id: Number(id) })
    return ''
  })
  if (skillPath) {
    const name = skillShortName(skillPath)
    cleaned = cleaned.replace(new RegExp(`@${escapeRegExp(name)}\\s*`), '')
  }
  return { entities: found, cleaned }
}

export function createAgentRunActions(deps: {
  activeKey: Ref<string>
  stateOf: (key: string) => SessionChatState
  showToast: (msg: string) => void
  instruction: Ref<string>
  entities: Ref<AgentEntityRefSeed[]>
  skillPath: Ref<string | null>
  files: Ref<string[]>
  focusAtEnd: () => void
  effectiveModel: Ref<AgentModelRef | null>
  selectedModel: Ref<AgentModelRef | null>
  oneShotModel: Ref<AgentModelRef | null>
  modelOnce: Ref<boolean>
  sessionTitle: Ref<string>
  persistSessionMeta: (key: string, title: string, model: AgentModelRef | null) => void
  showSkillMenu: Ref<boolean>
  showEntityMenu: Ref<boolean>
  sources: Ref<Source[]>
  releases: Ref<ReleaseInfo[]>
  catalogReady: Ref<{ source: boolean; release: boolean }>
  activeRunId: Ref<number | null>
  canStop: Ref<boolean>
  submitting: Ref<boolean>
  loadChat: () => Promise<void>
  startPolling: () => void
  discardPendingRpcEvents: (sessionKey?: string) => void
}) {
  function existingEntities(ents: AgentEntityRefSeed[]): AgentEntityRefSeed[] {
    return ents.filter((e) =>
      e.kind === 'source'
        ? !deps.catalogReady.value.source || deps.sources.value.some((s) => s.id === e.id)
        : !deps.catalogReady.value.release || deps.releases.value.some((r) => r.id === e.id),
    )
  }

  function applyRunToComposer(run: AgentRunSummary): void {
    deps.instruction.value = run.instruction
    const ents = runEntities(run)
    const alive = existingEntities(ents)
    if (alive.length !== ents.length) {
      deps.showToast(t('agent.retry_entities_dropped', String(ents.length - alive.length)))
    }
    deps.entities.value = alive
    deps.skillPath.value = run.skill_path || null
    if (deps.effectiveModel.value === null) deps.selectedModel.value = runModel(run)
    deps.files.value = runFiles(run)
  }

  async function handleSubmit(): Promise<void> {
    if (deps.showSkillMenu.value || deps.showEntityMenu.value) return
    const key = deps.activeKey.value
    const st = deps.stateOf(key)
    const { entities: inline, cleaned } = parseInlineRefs(deps.instruction.value, deps.skillPath.value)
    const all = [...deps.entities.value, ...inline]
    const merged: AgentEntityRefSeed[] = []
    for (const e of all) {
      if (!merged.some((x) => x.kind === e.kind && x.id === e.id)) merged.push(e)
    }
    if (cleaned.trim() === '' && merged.length === 0 && deps.files.value.length === 0) {
      deps.showToast(t('agent.empty_job'))
      return
    }
    st.submitting = true
    try {
      st.historySnapshot = [...st.messages]
      st.liveMessages = [
        {
          role: 'user',
          blocks: [{ kind: 'text', text: cleaned.trim() }],
          timestamp: new Date().toISOString(),
          model: null,
        },
      ]
      const runId = await runAgentJob({
        sessionKey: key,
        entities: merged,
        skillPath: deps.skillPath.value,
        instruction: cleaned.trim(),
        model: deps.effectiveModel.value,
        files: deps.files.value.length > 0 ? [...deps.files.value] : null,
      })
      st.submittedRunId = runId
      track('agent.submit')
      const title = cleaned.trim() ? [...cleaned.trim()].slice(0, 40).join('') : deps.sessionTitle.value
      const modelToPersist = deps.selectedModel.value
      if (deps.activeKey.value === key) {
        deps.instruction.value = ''
        deps.oneShotModel.value = null
        deps.modelOnce.value = false
        deps.files.value = []
        deps.entities.value = []
        deps.skillPath.value = null
        deps.persistSessionMeta(key, title, modelToPersist)
        await deps.loadChat()
        deps.startPolling()
      } else {
        deps.persistSessionMeta(key, title, null)
      }
    } catch (e) {
      deps.showToast(extractErrorMessage(e))
      deps.discardPendingRpcEvents(key)
      st.liveMessages = []
      st.historySnapshot = []
    } finally {
      st.submitting = false
    }
  }

  async function handleCancel(): Promise<void> {
    const st = deps.stateOf(deps.activeKey.value)
    if (deps.activeRunId.value === null || st.cancelling) return
    st.cancelling = true
    try {
      await cancelAgentRun(deps.activeRunId.value)
      deps.showToast(t('agent.cancelling'))
      deps.startPolling()
    } catch (e) {
      deps.showToast(extractErrorMessage(e))
      st.cancelling = false
    }
  }

  async function handleRetry(run: AgentRunSummary): Promise<void> {
    if (deps.canStop.value || deps.submitting.value) {
      deps.showToast(t('agent.retry_blocked'))
      return
    }
    applyRunToComposer(run)
    await handleSubmit()
  }

  function handleRetryEdit(run: AgentRunSummary): void {
    if (deps.canStop.value || deps.submitting.value) {
      deps.showToast(t('agent.retry_blocked'))
      return
    }
    applyRunToComposer(run)
    nextTick(() => deps.focusAtEnd())
  }

  return {
    handleSubmit,
    handleCancel,
    handleRetry,
    handleRetryEdit,
  }
}
