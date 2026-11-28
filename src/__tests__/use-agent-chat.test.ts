import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { nextTick, ref } from 'vue'
import { t } from '../i18n'
import { createSessionCache } from '../components/agent/agentSessionCache'
import { createRpcStreamBatcher, applyRpcEventToState } from '../components/agent/agentRpcStream'
import {
  buildUserRunMap,
  computeMessageDecorations,
  computeRunFailedNote,
} from '../components/agent/agentMessageDecorate'
import { parseInlineRefs } from '../components/agent/agentRunActions'
import { useAgentStore } from '../stores/agent'
import { useCatalogStore } from '../stores/catalog'
import { useUiStore } from '../stores/ui'
import {
  cancelAgentRun,
  getAgentQueue,
  getAgentQueueStatus,
  getAgentRpcStatus,
  getAgentSessionUsage,
  listAgentMessages,
  listAgentRuns,
  runAgentJob,
  type AgentChatMessage,
  type AgentRunSummary,
} from '../api/agent'
import type { Source } from '../api/sources'
import type { ReleaseInfo } from '../api/releases'

vi.mock('../api/agent', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/agent')>()
  return {
    ...actual,
    listAgentRuns: vi.fn().mockResolvedValue([]),
    listAgentMessages: vi.fn().mockResolvedValue([]),
    getAgentQueueStatus: vi.fn().mockResolvedValue(null),
    getAgentQueue: vi.fn().mockResolvedValue([]),
    getAgentSessionUsage: vi.fn().mockResolvedValue(null),
    getAgentRpcStatus: vi.fn().mockResolvedValue({ running: false, pid: null, process_count: 0, restart_pending: false }),
    runAgentJob: vi.fn().mockResolvedValue(101),
    cancelAgentRun: vi.fn().mockResolvedValue(undefined),
  }
})

function userMsg(text: string, over: Partial<AgentChatMessage> = {}): AgentChatMessage {
  return {
    role: 'user',
    blocks: [{ kind: 'text', text }],
    timestamp: '2026-09-03T00:00:00Z',
    model: null,
    ...over,
  } as AgentChatMessage
}

function makeRun(over: Partial<AgentRunSummary> = {}): AgentRunSummary {
  return {
    id: 1,
    session_key: 's1',
    skill_path: null,
    entities: '[]',
    instruction: '做点事',
    model: null,
    session_path: null,
    status: 'running',
    exit_code: null,
    error: null,
    started_at: '2026-09-03T00:00:00Z',
    finished_at: null,
    created_at: '2026-09-03T00:00:00Z',
    files: null,
    ...over,
  }
}

const ev = (obj: Record<string, unknown>) => JSON.stringify(obj)
const rpc = (session_key: string, event: string) => ({ session_key, run_id: 1, event })

beforeEach(() => {
  vi.clearAllMocks()
  localStorage.clear()
})

afterEach(() => {
  vi.useRealTimers()
})

describe('agentSessionCache — 分片缓存与 LRU 回收', () => {
  it('按 key 惰性建分片并在超上限时仅回收非当前且无活跃 run 的最早分片', () => {
    const activeKey = ref('s1')
    const cache = createSessionCache(activeKey, 3)

    cache.stateOf('s1').messages = [userMsg('m1')]
    cache.stateOf('s2').runs = [makeRun({ session_key: 's2', status: 'running' })]
    cache.stateOf('s3').messages = [userMsg('m3')]
    cache.stateOf('s4').messages = [userMsg('m4')]

    cache.prune()
    expect(cache.states.size).toBe(3)
    expect(cache.hasState('s1')).toBe(true)
    expect(cache.hasState('s2')).toBe(true)
    expect(cache.hasState('s3')).toBe(false)
    expect(cache.hasState('s4')).toBe(true)
  })

  it('resetState 清空目标分片并保留其他分片', () => {
    const activeKey = ref('s1')
    const cache = createSessionCache(activeKey)
    cache.stateOf('s1').messages = [userMsg('m1')]
    cache.stateOf('s1').submittedRunId = 9
    cache.stateOf('s2').messages = [userMsg('m2')]

    cache.resetState('s1')
    expect(cache.stateOf('s1').messages).toEqual([])
    expect(cache.stateOf('s1').submittedRunId).toBeNull()
    expect(cache.stateOf('s2').messages).toHaveLength(1)
  })
})

describe('agentRpcStream — 50ms 合帧与协议解析', () => {
  it('合并 text_delta / thinking_delta / toolCall / bash，并在 agent_settled 时回调', () => {
    vi.useFakeTimers()
    const activeKey = ref('s1')
    const cache = createSessionCache(activeKey)
    const onSettled = vi.fn()
    const onFlushed = vi.fn()
    const batcher = createRpcStreamBatcher({
      stateOf: cache.stateOf,
      onSettled,
      onBatchFlushed: onFlushed,
    })

    batcher.enqueue(rpc('s1', ev({ type: 'message_update', assistantMessageEvent: { type: 'thinking_delta', delta: '想' } })))
    batcher.enqueue(rpc('s1', ev({ type: 'message_update', assistantMessageEvent: { type: 'thinking_delta', delta: '一想' } })))
    batcher.enqueue(rpc('s1', ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: '你好' } })))
    batcher.enqueue(rpc('s1', ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: '世界' } })))
    batcher.enqueue(rpc('s1', ev({ type: 'tool_execution_start', toolCallId: 'tc1', toolName: 'read', args: { p: 'a' } })))
    batcher.enqueue(rpc('s1', ev({ type: 'bash_execution_update', delta: 'line1\n' })))
    batcher.enqueue(rpc('s1', ev({ type: 'bash_execution_update', delta: 'line2' })))
    batcher.enqueue(rpc('s1', 'not-valid-json'))

    vi.advanceTimersByTime(50)
    expect(onFlushed).toHaveBeenCalledTimes(1)
    const live = cache.stateOf('s1').liveMessages
    expect(live).toHaveLength(1)
    expect(live[0].blocks).toEqual([
      { kind: 'thinking', text: '想一想' },
      { kind: 'text', text: '你好世界' },
      { kind: 'toolCall', id: 'tc1', name: 'read', args: '{"p":"a"}' },
      { kind: 'bash', command: '', output: 'line1\nline2', exit_code: null, truncated: false },
    ])

    applyRpcEventToState(cache.stateOf('s1'), rpc('s1', ev({ type: 'agent_settled' })), onSettled)
    expect(cache.stateOf('s1').liveMessages).toEqual([])
    expect(onSettled).toHaveBeenCalledWith('s1')
  })

  it('discard(sessionKey) 仅丢弃目标会话的未 flush 事件，保留后台会话事件', () => {
    vi.useFakeTimers()
    const activeKey = ref('s1')
    const cache = createSessionCache(activeKey)
    const batcher = createRpcStreamBatcher({
      stateOf: cache.stateOf,
      onSettled: vi.fn(),
      onBatchFlushed: vi.fn(),
    })

    batcher.enqueue(rpc('s1', ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: 'drop' } })))
    batcher.enqueue(rpc('s2', ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: 'keep' } })))
    batcher.discard('s1')

    vi.advanceTimersByTime(50)
    expect(cache.stateOf('s1').liveMessages).toEqual([])
    expect(cache.stateOf('s2').liveMessages[0].blocks).toEqual([{ kind: 'text', text: 'keep' }])
  })
})

describe('agentRpcStream — run 归属过滤', () => {
  it('旧 run 的迟滞 delta / agent_settled 被丢弃，不污染新 run', () => {
    vi.useFakeTimers()
    const cache = createSessionCache(ref('s1'))
    const onSettled = vi.fn()
    const batcher = createRpcStreamBatcher({
      stateOf: cache.stateOf,
      onSettled,
      onBatchFlushed: vi.fn(),
    })
    const delta = (text: string) =>
      ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: text } })

    // 新 run（id 更大）先声明归属
    batcher.enqueue({ session_key: 's1', run_id: 12, event: delta('新回答') })
    // 已取消的旧 run 残余：文本尾巴 + settled
    batcher.enqueue({ session_key: 's1', run_id: 11, event: delta('旧尾巴') })
    batcher.enqueue({ session_key: 's1', run_id: 11, event: ev({ type: 'agent_settled' }) })
    vi.advanceTimersByTime(50)

    const st = cache.stateOf('s1')
    expect(st.liveMessages).toHaveLength(1)
    expect(st.liveMessages[0].blocks).toEqual([{ kind: 'text', text: '新回答' }])
    expect(onSettled).not.toHaveBeenCalled()
    expect(st.activeStreamRunId).toBe(12)
  })

  it('首个事件声明归属，之后同一 run 的事件照常接收', () => {
    vi.useFakeTimers()
    const cache = createSessionCache(ref('s1'))
    const batcher = createRpcStreamBatcher({
      stateOf: cache.stateOf,
      onSettled: vi.fn(),
      onBatchFlushed: vi.fn(),
    })
    const delta = (text: string) =>
      ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: text } })

    batcher.enqueue({ session_key: 's1', run_id: 7, event: delta('第一') })
    batcher.enqueue({ session_key: 's1', run_id: 7, event: delta('第二') })
    vi.advanceTimersByTime(50)

    expect(cache.stateOf('s1').liveMessages[0].blocks).toEqual([{ kind: 'text', text: '第一第二' }])
  })
})

describe('agentMessageDecorate & parseInlineRefs', () => {
  it('按 run_id 直连优先、时间窗兜底对位 user 消息，并生成失败备注', () => {
    const r1 = makeRun({ id: 10, status: 'timeout', error: 'err.agent.timeout|300', entities: '[{"kind":"source","id":1}]' })
    const msgs = [
      userMsg('问个问题', { run_id: 10 } as Partial<AgentChatMessage>),
      { role: 'assistant', blocks: [{ kind: 'text', text: '答' }], timestamp: '2026-09-03T00:00:01Z', model: null } as AgentChatMessage,
    ]
    const fallback = buildUserRunMap(msgs, [r1])
    const decs = computeMessageDecorations(msgs, [r1], fallback)

    expect(decs[0]?.run?.id).toBe(10)
    expect(decs[0]?.entities).toEqual([{ kind: 'source', id: 1 }])
    expect(decs[1]).toBeNull()
    expect(computeRunFailedNote(r1)).toBe(t('err.agent.timeout', '300'))
    expect(computeRunFailedNote(makeRun({ status: 'success' }))).toBeNull()
  })

  it('parseInlineRefs 剥离内联实体引用与已选 @skill 短名', () => {
    const res = parseInlineRefs('请看 [[source:3]] 和 [[release:9]] @my-skill 总结一下', '/skills/my-skill/SKILL.md')
    expect(res.entities).toEqual([
      { kind: 'source', id: 3 },
      { kind: 'release', id: 9 },
    ])
    expect(res.cleaned.trim()).toBe('请看  和  总结一下')
  })
})

describe('useAgentStore — 提交、停止、重试与排队状态', () => {
  it('空输入提交被拒绝并弹出 Toast', async () => {
    const ui = useUiStore()
    const spy = vi.spyOn(ui, 'showToast')
    const store = useAgentStore()

    await store.handleSubmit()
    expect(spy).toHaveBeenCalledWith(t('agent.empty_job'))
    expect(runAgentJob).not.toHaveBeenCalled()
  })

  it('提交成功后清空输入与单次覆盖，进入可停止态并支持取消', async () => {
    const store = useAgentStore()
    store.instruction = '分析一下'
    store.entities = [{ kind: 'source', id: 1 }]
    store.files = ['C:/a.log']
    store.modelOnce = true
    store.oneShotModel = { provider: 'openai', model_id: 'gpt-4' }

    vi.mocked(listAgentRuns).mockResolvedValueOnce([makeRun({ id: 101, session_key: store.activeKey, status: 'running' })])
    await store.handleSubmit()

    expect(runAgentJob).toHaveBeenCalledWith(
      expect.objectContaining({
        instruction: '分析一下',
        entities: [{ kind: 'source', id: 1 }],
        files: ['C:/a.log'],
        model: { provider: 'openai', model_id: 'gpt-4' },
      }),
    )
    expect(store.instruction).toBe('')
    expect(store.entities).toEqual([])
    expect(store.files).toEqual([])
    expect(store.oneShotModel).toBeNull()
    expect(store.modelOnce).toBe(false)
    expect(store.canStop).toBe(true)

    await store.handleCancel()
    expect(cancelAgentRun).toHaveBeenCalledWith(101)
    store.cleanupWorkspaceRuntime()
  })

  it('重试时保留当前已加载目录中存在的实体、剔除已删除实体，且不覆盖用户新选模型', async () => {
    const catalog = useCatalogStore()
    catalog.sources = [{ id: 1 } as Source]
    catalog.releases = [{ id: 10 } as ReleaseInfo]
    catalog.sourcesLoaded = true
    catalog.releasesLoaded = true

    const ui = useUiStore()
    const spy = vi.spyOn(ui, 'showToast')
    const store = useAgentStore()
    store.selectedModel = { provider: 'anthropic', model_id: 'claude' }

    const failedRun = makeRun({
      id: 5,
      status: 'failed',
      instruction: '重试这条',
      entities: '[{"kind":"source","id":1},{"kind":"release","id":999}]',
      model: JSON.stringify({ provider: 'openai', model_id: 'gpt-old' }),
    })

    store.handleRetryEdit(failedRun)
    await nextTick()
    expect(store.instruction).toBe('重试这条')
    expect(store.entities).toEqual([{ kind: 'source', id: 1 }])
    expect(store.selectedModel).toEqual({ provider: 'anthropic', model_id: 'claude' })
    expect(spy).toHaveBeenCalledWith(t('agent.retry_entities_dropped', '1'))
  })

  it('onRunFinished 仅清目标会话流式残留，后台会话完成不干扰当前会话流式输出', async () => {
    vi.useFakeTimers()
    const store = useAgentStore()
    store.mountWorkspaceRuntime()
    const activeKey = store.activeKey
    store.liveMessages = [userMsg('当前流式')]

    // 后台会话先建分片再收到完成事件
    store.handleRpcStream(rpc('bg-session', ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: '后台文本' } })))
    vi.advanceTimersByTime(50)

    vi.mocked(listAgentMessages).mockResolvedValueOnce([userMsg('后台落库')])
    vi.mocked(getAgentQueueStatus).mockResolvedValueOnce(null as never)
    vi.mocked(getAgentQueue).mockResolvedValueOnce([])
    vi.mocked(getAgentRpcStatus).mockResolvedValueOnce({ running: true, pid: 1, process_count: 1, restart_pending: false })

    await store.onRunFinished('bg-session')
    expect(store.liveMessages).toEqual([userMsg('当前流式')])

    store.historySnapshot = [userMsg('snap')]
    vi.mocked(listAgentMessages).mockResolvedValueOnce([userMsg('persisted')])
    vi.mocked(getAgentSessionUsage).mockResolvedValueOnce(null as never)
    vi.mocked(getAgentQueueStatus).mockResolvedValueOnce(null as never)
    vi.mocked(getAgentQueue).mockResolvedValueOnce([])
    vi.mocked(getAgentRpcStatus).mockResolvedValueOnce({ running: true, pid: 1, process_count: 1, restart_pending: false })

    await store.onRunFinished(activeKey)
    expect(store.liveMessages).toEqual([])
    expect(store.historySnapshot).toEqual([])
    expect(store.messages).toHaveLength(1)
    store.cleanupWorkspaceRuntime()
  })

  it('旧 run 的终态晚到：不清空新 run 的流式输出，仅刷新运行记录', async () => {
    vi.useFakeTimers()
    const store = useAgentStore()
    store.mountWorkspaceRuntime()
    const key = store.activeKey

    // 取消旧任务后立即提交新任务：新 run 已在流式输出
    store.handleRpcStream({
      session_key: key,
      run_id: 20,
      event: ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: '新回答' } }),
    })
    vi.advanceTimersByTime(50)
    expect(store.liveMessages).toHaveLength(1)

    vi.mocked(listAgentRuns).mockResolvedValue([makeRun({ id: 20, session_key: key, status: 'running' })])
    vi.mocked(getAgentQueue).mockResolvedValueOnce([])
    vi.mocked(getAgentQueueStatus).mockResolvedValueOnce(null as never)
    vi.mocked(getAgentRpcStatus).mockResolvedValueOnce({ running: true, pid: 1, process_count: 1, restart_pending: false })

    // 旧 run（id 更小）的终态事件晚到
    await store.onRunFinished(key, 19)
    expect(store.liveMessages).toHaveLength(1)
    expect(vi.mocked(listAgentMessages)).not.toHaveBeenCalled()

    // 当前 run 自己的终态照旧收尾
    vi.mocked(listAgentMessages).mockResolvedValueOnce([])
    vi.mocked(getAgentQueueStatus).mockResolvedValueOnce(null as never)
    vi.mocked(getAgentQueue).mockResolvedValueOnce([])
    vi.mocked(getAgentRpcStatus).mockResolvedValueOnce({ running: false, pid: null, process_count: 0, restart_pending: false })
    vi.mocked(listAgentRuns).mockResolvedValueOnce([makeRun({ id: 20, session_key: key, status: 'success' })])
    await store.onRunFinished(key, 20)
    expect(store.liveMessages).toEqual([])
    store.cleanupWorkspaceRuntime()
  })

  it('提交前丢弃旧 run 的在途合帧事件，旧尾巴不会拼进新气泡', async () => {
    vi.useFakeTimers()
    const store = useAgentStore()
    store.mountWorkspaceRuntime()
    const key = store.activeKey

    // 旧 run 的 delta 已入合帧队列（50ms 窗口）尚未 flush
    store.handleRpcStream({
      session_key: key,
      run_id: 5,
      event: ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: '旧尾巴' } }),
    })

    let resolveJob: (runId: number) => void = () => {}
    vi.mocked(runAgentJob).mockImplementationOnce(
      () => new Promise<number>((resolve) => { resolveJob = resolve }),
    )
    store.instruction = '新问题'
    const pending = store.handleSubmit()
    await nextTick()

    // 提交在途（新 run 尚未声明归属）时旧事件到点 flush
    vi.advanceTimersByTime(50)
    resolveJob(30)
    await pending

    expect(store.liveMessages).toHaveLength(1)
    expect(store.liveMessages[0].role).toBe('user')
    store.cleanupWorkspaceRuntime()
  })

  it('迟到的旧 run 终态不会清掉新 run 的取消中状态', async () => {
    const store = useAgentStore()
    store.mountWorkspaceRuntime()
    const key = store.activeKey

    vi.mocked(listAgentRuns).mockResolvedValueOnce([makeRun({ id: 40, session_key: key, status: 'running' })])
    store.handleRpcStream({
      session_key: key,
      run_id: 40,
      event: ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: '新回答' } }),
    })
    await store.loadChat()
    await new Promise((r) => setTimeout(r, 60))
    await store.handleCancel()
    expect(store.cancelling).toBe(true)

    vi.mocked(listAgentRuns).mockResolvedValue([makeRun({ id: 40, session_key: key, status: 'running' })])
    vi.mocked(getAgentQueue).mockResolvedValueOnce([])
    vi.mocked(getAgentQueueStatus).mockResolvedValueOnce(null as never)
    vi.mocked(getAgentRpcStatus).mockResolvedValueOnce({ running: true, pid: 1, process_count: 1, restart_pending: false })

    // 旧 run（id 更小）的终态晚到：不能抹掉「取消中」
    await store.onRunFinished(key, 39)
    expect(store.cancelling).toBe(true)
    store.cleanupWorkspaceRuntime()
  })

  it('提交在途切走会话：原会话仍记住提交时选定的模型', async () => {
    const store = useAgentStore()
    const key = store.activeKey
    store.selectedModel = { provider: 'openai', model_id: 'gpt-4' }
    store.instruction = '分析这个版本'

    let resolveJob: (runId: number) => void = () => {}
    vi.mocked(runAgentJob).mockImplementationOnce(
      () => new Promise<number>((resolve) => { resolveJob = resolve }),
    )

    const pending = store.handleSubmit()
    await nextTick()
    // 提交还没返回时切到另一个会话（selectedModel 随之重置）
    store.switchSession('another-session')
    expect(store.selectedModel).toBeNull()

    resolveJob(30)
    await pending

    const stored = JSON.parse(localStorage.getItem('relwatch.agent.sessions.v1')!) as Array<{
      key: string
      model?: { provider: string; model_id: string } | null
    }>
    expect(stored.find((s) => s.key === key)?.model).toEqual({ provider: 'openai', model_id: 'gpt-4' })
    store.cleanupWorkspaceRuntime()
  })

  it('loadChat：并发加载、水位预清、submittedRunId 复位与活跃 run 时冻结 historySnapshot', async () => {
    const store = useAgentStore()
    const key = store.activeKey
    store.submittedRunId = 88
    store.usage = {
      tokens: 1000,
      context_window: 10000,
      percent: 10,
      estimated: false,
    } as never

    vi.mocked(listAgentRuns).mockResolvedValueOnce([makeRun({ id: 88, session_key: key, status: 'running' })])
    vi.mocked(listAgentMessages).mockResolvedValueOnce([userMsg('历史消息')])
    vi.mocked(getAgentQueueStatus).mockResolvedValueOnce(null as never)
    vi.mocked(getAgentQueue).mockResolvedValueOnce([])
    vi.mocked(getAgentSessionUsage).mockResolvedValueOnce(null as never)

    await store.loadChat()
    expect(store.submittedRunId).toBeNull()
    expect(store.usage).toBeNull()
    expect(store.historySnapshot).toHaveLength(1)
    expect(store.historySnapshot[0].blocks[0]).toEqual({ kind: 'text', text: '历史消息' })
    store.cleanupWorkspaceRuntime()
  })

  it('loadChat 并发竞态：后发先至后，慢请求返回不得覆盖 runs / messages / queueInfo / usage', async () => {
    const store = useAgentStore()
    const key = store.activeKey

    let resolveOldRuns!: (v: AgentRunSummary[]) => void
    let resolveOldMsgs!: (v: AgentChatMessage[]) => void
    let resolveOldQueue!: (v: never) => void
    let resolveOldUsage!: (v: never) => void

    vi.mocked(listAgentRuns)
      .mockImplementationOnce(() => new Promise((r) => { resolveOldRuns = r }))
      .mockResolvedValueOnce([makeRun({ id: 2, session_key: key, status: 'success' })])
    vi.mocked(listAgentMessages)
      .mockImplementationOnce(() => new Promise((r) => { resolveOldMsgs = r }))
      .mockResolvedValueOnce([userMsg('新消息')])
    vi.mocked(getAgentQueueStatus)
      .mockImplementationOnce(() => new Promise((r) => { resolveOldQueue = r }))
      .mockResolvedValueOnce({ position: 0, other_running: false, running_sessions: [], max_concurrency: 1, running_count: 0 } as never)
    vi.mocked(getAgentSessionUsage)
      .mockImplementationOnce(() => new Promise((r) => { resolveOldUsage = r }))
      .mockResolvedValueOnce({ message_count: 2 } as never)

    const p1 = store.loadChat()
    const p2 = store.loadChat()
    await p2

    resolveOldRuns([makeRun({ id: 1, session_key: key, status: 'running' })])
    resolveOldMsgs([userMsg('旧消息')])
    resolveOldQueue({ position: 5, other_running: true, running_sessions: ['x'], max_concurrency: 1, running_count: 1 } as never)
    resolveOldUsage({ message_count: 99 } as never)
    await p1

    expect(store.runs.map((r) => r.id)).toEqual([2])
    expect(store.messages[0].blocks[0]).toEqual({ kind: 'text', text: '新消息' })
    expect(store.queueInfo?.position).toBe(0)
    expect(store.usage?.message_count).toBe(2)
    store.cleanupWorkspaceRuntime()
  })

  it('新会话首条提交后 loadChat 若已读到落盘的 user 消息，不将其写进 historySnapshot 导致重复展示', async () => {
    const store = useAgentStore()
    const key = store.activeKey
    store.instruction = '首条提问'

    vi.mocked(listAgentRuns).mockResolvedValueOnce([makeRun({ id: 101, session_key: key, status: 'running' })])
    vi.mocked(listAgentMessages).mockResolvedValueOnce([userMsg('首条提问')])

    await store.handleSubmit()

    expect(store.historySnapshot).toEqual([])
    expect(store.displayedMessages).toHaveLength(1)
    store.cleanupWorkspaceRuntime()
  })

  it('handleSubmit：内联实体与已选实体合并去重；菜单打开不提交；被拒时回滚并保留输入', async () => {
    const ui = useUiStore()
    const toastSpy = vi.spyOn(ui, 'showToast')
    const store = useAgentStore()
    store.instruction = '帮我检查 [[source:1]] [[release:2]]'
    store.entities = [{ kind: 'source', id: 1 }]
    store.showSkillMenu = true

    await store.handleSubmit()
    expect(runAgentJob).not.toHaveBeenCalled()

    store.showSkillMenu = false
    vi.mocked(runAgentJob).mockResolvedValueOnce(202)
    await store.handleSubmit()
    expect(runAgentJob).toHaveBeenCalledWith(
      expect.objectContaining({
        instruction: '帮我检查',
        entities: [
          { kind: 'source', id: 1 },
          { kind: 'release', id: 2 },
        ],
      }),
    )

    store.instruction = '失败后保留'
    vi.mocked(runAgentJob).mockRejectedValueOnce(new Error('Error: rpc busy'))
    await store.handleSubmit()

    expect(store.submitting).toBe(false)
    expect(store.liveMessages).toEqual([])
    expect(store.historySnapshot).toEqual([])
    expect(store.instruction).toBe('失败后保留')
    expect(toastSpy).toHaveBeenCalledWith(expect.stringContaining('rpc busy'))
    store.cleanupWorkspaceRuntime()
  })

  it('handleCancel：无可停 run 忽略；成功置 cancelling；取消失败复位并提示', async () => {
    const ui = useUiStore()
    const toastSpy = vi.spyOn(ui, 'showToast')
    const store = useAgentStore()

    await store.handleCancel()
    expect(cancelAgentRun).not.toHaveBeenCalled()

    store.submittedRunId = 77
    vi.mocked(cancelAgentRun).mockResolvedValueOnce(undefined)
    await store.handleCancel()
    expect(store.cancelling).toBe(true)

    store.cancelling = false
    vi.mocked(cancelAgentRun).mockRejectedValueOnce(new Error('Error: cancel fail'))
    await store.handleCancel()
    expect(store.cancelling).toBe(false)
    expect(toastSpy).toHaveBeenCalledWith(expect.stringContaining('cancel fail'))
    store.cleanupWorkspaceRuntime()
  })

  it('handleRetry：活跃 run 时阻止；目录未加载时不剔除实体；单次模型覆盖不污染长期模型', async () => {
    const store = useAgentStore()
    store.submittedRunId = 55
    await store.handleRetry(makeRun({ id: 5, status: 'failed', instruction: '重试' }))
    expect(runAgentJob).not.toHaveBeenCalled()

    store.submittedRunId = null
    useCatalogStore().sourcesLoaded = false
    useCatalogStore().releasesLoaded = false
    store.selectedModel = { provider: 'anthropic', model_id: 'claude' }
    store.modelOnce = true
    store.oneShotModel = { provider: 'openai', model_id: 'gpt-4o' }

    await store.handleRetry(
      makeRun({
        id: 6,
        status: 'failed',
        instruction: '未加载目录重发',
        entities: '[{"kind":"source","id":999}]',
        model: JSON.stringify({ provider: 'deepseek', model_id: 'deepseek-chat' }),
      }),
    )

    expect(runAgentJob).toHaveBeenCalledWith(
      expect.objectContaining({
        instruction: '未加载目录重发',
        entities: [{ kind: 'source', id: 999 }],
        model: { provider: 'openai', model_id: 'gpt-4o' },
      }),
    )
    expect(store.selectedModel).toEqual({ provider: 'anthropic', model_id: 'claude' })
    store.cleanupWorkspaceRuntime()
  })

  it('会话切换三模式：switch 停轮询并切分片 / new 清目标分片不停轮询 / delete 不动当前态', async () => {
    vi.useFakeTimers()
    const store = useAgentStore()
    store.mountWorkspaceRuntime()
    const keyA = store.activeKey
    store.handleRpcStream(rpc(keyA, ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: 'A在途' } })))

    // switch：真正切到新会话，停止旧轮询，旧会话在途 delta 保留在旧分片里
    store.startPolling()
    store.switchSession('session-b')
    expect(store.activeKey).toBe('session-b')
    const queueCallsBefore = vi.mocked(getAgentQueue).mock.calls.length
    vi.advanceTimersByTime(3100)
    expect(vi.mocked(getAgentQueue).mock.calls.length).toBe(queueCallsBefore)
    expect(store.liveMessages).toEqual([])

    store.switchSession(keyA)
    expect(store.liveMessages[0].blocks).toEqual([{ kind: 'text', text: 'A在途' }])

    // new：目标分片清空，不停轮询，不牵连其他分片
    store.handleRpcStream(rpc('s-other', ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: '他会话' } })))
    store.startPolling()
    store.resetForSessionSwitch('new')
    expect(store.liveMessages).toEqual([])
    const queueCallsNew = vi.mocked(getAgentQueue).mock.calls.length
    vi.advanceTimersByTime(1600)
    expect(vi.mocked(getAgentQueue).mock.calls.length).toBeGreaterThan(queueCallsNew)
    store.switchSession('s-other')
    expect(store.liveMessages[0].blocks).toEqual([{ kind: 'text', text: '他会话' }])

    // delete：保留当前流式态不变
    store.liveMessages = [userMsg('保留')]
    store.resetForSessionSwitch('delete')
    expect(store.liveMessages).toHaveLength(1)
    store.cleanupWorkspaceRuntime()
  })

  it('startPolling 在 120s 截止后自动停止轮询', async () => {
    vi.useFakeTimers()
    const store = useAgentStore()

    store.startPolling()
    await vi.advanceTimersByTimeAsync(125_000)
    const callsAtDeadline = vi.mocked(getAgentQueue).mock.calls.length
    expect(callsAtDeadline).toBeGreaterThan(0)

    await vi.advanceTimersByTimeAsync(10_000)
    expect(vi.mocked(getAgentQueue).mock.calls.length).toBe(callsAtDeadline)
  })

  it('queueHint / queueOccupiedBy 覆盖单并发占用、多并发未满/跑满与本会话占用全部分支', () => {
    const store = useAgentStore()
    const selfKey = store.activeKey
    store.runs = [makeRun({ id: 1, session_key: selfKey, status: 'pending' })]

    store.queueInfo = {
      other_running: true,
      running_sessions: ['other-session'],
      max_concurrency: 1,
      running_count: 1,
      position: 2,
    } as never
    expect(store.queueOccupiedBy).toBe('other-session')
    expect(store.queueHint).toBe(t('agent.queue_other_running_pos', '2'))

    store.queueInfo = {
      other_running: true,
      running_sessions: [selfKey],
      max_concurrency: 1,
      running_count: 1,
      position: 1,
    } as never
    expect(store.queueOccupiedBy).toBeNull()
    expect(store.queueHint).toBe(t('agent.queue_other_running'))

    store.queueInfo = {
      other_running: true,
      running_sessions: ['other-session'],
      max_concurrency: 3,
      running_count: 1,
      position: 1,
    } as never
    expect(store.queueOccupiedBy).toBeNull()
    expect(store.queueHint).toBeNull()

    store.queueInfo = {
      other_running: true,
      running_sessions: ['other-session'],
      max_concurrency: 2,
      running_count: 2,
      position: 3,
    } as never
    expect(store.queueOccupiedBy).toBe('other-session')
    expect(store.queueHint).toBe(t('agent.queue_limit_reached_pos', '3'))
  })

  it('cleanupWorkspaceRuntime 刷入在途缓冲、回收空闲分片、保留运行中分片并复位瞬态菜单', () => {
    vi.useFakeTimers()
    const store = useAgentStore()
    store.mountWorkspaceRuntime()
    const idleKey = store.activeKey
    store.messages = [userMsg('idle-msg')]

    // 构造一个运行中后台分片 + 尚未 flush 的在途 delta + 展开菜单
    store.switchSession('active-bg')
    store.runs = [makeRun({ id: 9, session_key: 'active-bg', status: 'running' })]
    store.handleRpcStream(rpc('active-bg', ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: '不丢这段' } })))
    store.rpcMenuOpen = true
    store.showModelMenu = true
    store.historyOpen = true

    store.cleanupWorkspaceRuntime()

    // 瞬态菜单全部复位，防止 Esc 覆盖层锁死
    expect(store.rpcMenuOpen).toBe(false)
    expect(store.showModelMenu).toBe(false)
    expect(store.historyOpen).toBe(false)

    // 运行中分片的在途 delta 已 flush 保留，不会断层缺字
    expect(store.liveMessages[0].blocks).toEqual([{ kind: 'text', text: '不丢这段' }])

    // 空闲分片已回收
    store.switchSession(idleKey)
    expect(store.messages).toEqual([])

    // 面板关闭期间对陌生 session_key 不分配新分片
    store.handleRpcStream(rpc('stranger-session', ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: 'ignored' } })))
    vi.advanceTimersByTime(50)
    store.switchSession('stranger-session')
    expect(store.liveMessages).toEqual([])
  })
})
