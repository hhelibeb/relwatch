import type { AgentChatMessage } from '../../api/agent'
import type { SessionChatState } from './agentSessionCache'

export interface RpcStreamPayload {
  session_key: string
  run_id: number
  event: string
}

export const RPC_FLUSH_MS = 50

interface ParsedRpcEvent {
  type: string
  assistantMessageEvent?: { type?: string; delta?: string }
  toolCallId?: unknown
  toolName?: unknown
  args?: unknown
  delta?: unknown
}

function parseRpcEvent(raw: string): ParsedRpcEvent | null {
  try {
    const parsed = JSON.parse(raw) as ParsedRpcEvent
    if (!parsed || typeof parsed.type !== 'string') return null
    return parsed
  } catch {
    return null
  }
}

function ensureLiveAssistant(st: SessionChatState): AgentChatMessage {
  let msg = st.liveMessages[st.liveMessages.length - 1]
  if (!msg || msg.role !== 'assistant') {
    msg = { role: 'assistant', blocks: [], timestamp: new Date().toISOString(), model: null }
    st.liveMessages.push(msg)
  }
  return msg
}

/**
 * run 归属仲裁：run id 由 SQLite 自增分配，「更大」即「更新」。
 *
 * 同一 session_key 上取消旧任务后立即提交新任务时，旧 run 的残余 delta /
 * agent_settled 仍会推送过来；不按 run 归属过滤就会把旧回答的尾巴拼进新回答，
 * 或被旧 run 的 settled 清空当前流式内容。
 *
 * 返回 false = 迟到事件，调用方必须丢弃。
 */
export function claimStreamRun(st: SessionChatState, runId: number): boolean {
  if (st.activeStreamRunId === null || runId > st.activeStreamRunId) {
    st.activeStreamRunId = runId
    return true
  }
  return runId === st.activeStreamRunId
}

export function applyRpcEventToState(
  st: SessionChatState,
  payload: RpcStreamPayload,
  onSettled: (sessionKey: string) => void,
): void {
  if (typeof payload.run_id === 'number' && !claimStreamRun(st, payload.run_id)) return

  const ev = parseRpcEvent(payload.event)
  if (!ev) return

  if (ev.type === 'message_update') {
    const ae = ev.assistantMessageEvent
    if (!ae?.delta) return
    const msg = ensureLiveAssistant(st)
    if (ae.type === 'text_delta') {
      const last = msg.blocks[msg.blocks.length - 1]
      if (last && last.kind === 'text') {
        last.text += ae.delta
      } else {
        msg.blocks.push({ kind: 'text', text: ae.delta })
      }
    } else if (ae.type === 'thinking_delta') {
      const last = msg.blocks[msg.blocks.length - 1]
      if (last && last.kind === 'thinking') {
        last.text += ae.delta
      } else {
        msg.blocks.push({ kind: 'thinking', text: ae.delta })
      }
    }
  } else if (ev.type === 'tool_execution_start') {
    const msg = ensureLiveAssistant(st)
    msg.blocks.push({
      kind: 'toolCall',
      id: String(ev.toolCallId ?? ''),
      name: String(ev.toolName ?? ''),
      args: JSON.stringify(ev.args ?? {}),
    })
  } else if (ev.type === 'bash_execution_update') {
    const delta = typeof ev.delta === 'string' ? ev.delta : ''
    if (!delta) return
    const msg = ensureLiveAssistant(st)
    const last = msg.blocks[msg.blocks.length - 1]
    if (last && last.kind === 'bash') {
      last.output += delta
    } else {
      msg.blocks.push({ kind: 'bash', command: '', output: delta, exit_code: null, truncated: false })
    }
  } else if (ev.type === 'agent_settled') {
    st.liveMessages = []
    st.historySnapshot = []
    onSettled(payload.session_key)
  }
}

export function createRpcStreamBatcher(opts: {
  stateOf: (sessionKey: string) => SessionChatState
  onSettled: (sessionKey: string) => void
  onBatchFlushed: () => void
  flushMs?: number
}) {
  const flushMs = opts.flushMs ?? RPC_FLUSH_MS
  let pending: RpcStreamPayload[] = []
  let timer: ReturnType<typeof setTimeout> | undefined

  function discard(sessionKey?: string): void {
    if (sessionKey === undefined) {
      if (timer !== undefined) {
        clearTimeout(timer)
        timer = undefined
      }
      pending = []
      return
    }
    pending = pending.filter((e) => e.session_key !== sessionKey)
    if (pending.length === 0 && timer !== undefined) {
      clearTimeout(timer)
      timer = undefined
    }
  }

  function flush(): void {
    if (timer !== undefined) {
      clearTimeout(timer)
      timer = undefined
    }
    if (pending.length === 0) return
    const batch = pending
    pending = []
    for (const payload of batch) {
      try {
        applyRpcEventToState(opts.stateOf(payload.session_key), payload, opts.onSettled)
      } catch {
        // 单事件异常不影响同批其余事件
      }
    }
    opts.onBatchFlushed()
  }

  function enqueue(payload: RpcStreamPayload): void {
    pending.push(payload)
    if (timer === undefined) {
      timer = setTimeout(flush, flushMs)
    }
  }

  return {
    enqueue,
    flush,
    discard,
  }
}
