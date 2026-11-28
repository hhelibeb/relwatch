import { computed, reactive, type Ref } from 'vue'
import type { AgentChatMessage, AgentQueueStatus, AgentRunSummary } from '../../api/agent'

export interface SessionChatState {
  messages: AgentChatMessage[]
  messagesLoading: boolean
  submitting: boolean
  runs: AgentRunSummary[]
  queueInfo: AgentQueueStatus | null
  liveMessages: AgentChatMessage[]
  historySnapshot: AgentChatMessage[]
  submittedRunId: number | null
  /**
   * 当前接收流式事件的 run。取消/重新提交后，旧 run 的残余 delta 与
   * agent_settled 仍会送到同一个 session_key，必须按 run 归属过滤，
   * 否则会混进新 run 的流式缓冲或被旧终态清空。
   */
  activeStreamRunId: number | null
  cancelling: boolean
}

export const MAX_SESSION_STATES = 12

export function createEmptySessionState(): SessionChatState {
  return {
    messages: [],
    messagesLoading: false,
    submitting: false,
    runs: [],
    queueInfo: null,
    liveMessages: [],
    historySnapshot: [],
    submittedRunId: null,
    activeStreamRunId: null,
    cancelling: false,
  }
}

export function createSessionCache(activeKey: Ref<string>, maxStates = MAX_SESSION_STATES) {
  const states = reactive(new Map<string, SessionChatState>())

  function stateOf(key: string): SessionChatState {
    if (!states.has(key)) {
      states.set(key, createEmptySessionState())
    }
    return states.get(key) as SessionChatState
  }

  function hasState(key: string): boolean {
    return states.has(key)
  }

  function getState(key: string): SessionChatState | undefined {
    return states.get(key)
  }

  const current = computed(() => stateOf(activeKey.value))

  function prune(): void {
    if (states.size <= maxStates) return
    for (const [key, st] of [...states]) {
      if (states.size <= maxStates) break
      if (key === activeKey.value) continue
      if (st.submittedRunId !== null || st.liveMessages.length > 0) continue
      if (st.runs.some((r) => r.status === 'running' || r.status === 'pending')) continue
      states.delete(key)
    }
  }

  function resetState(key: string): void {
    const st = stateOf(key)
    st.submittedRunId = null
    st.activeStreamRunId = null
    st.cancelling = false
    st.liveMessages = []
    st.historySnapshot = []
    st.messages = []
    st.runs = []
  }

  function pruneInactive(): void {
    for (const [key, st] of [...states]) {
      if (st.submittedRunId !== null || st.liveMessages.length > 0) continue
      if (st.runs.some((r) => r.status === 'running' || r.status === 'pending')) continue
      states.delete(key)
    }
  }

  return {
    states,
    current,
    stateOf,
    hasState,
    getState,
    prune,
    resetState,
    pruneInactive,
  }
}
