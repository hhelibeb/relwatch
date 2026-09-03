import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { computed, defineComponent, ref } from 'vue'
import type { Ref } from 'vue'
import { t } from '../i18n'
import { useAgentChat } from '../components/agent/useAgentChat'
import {
  cancelAgentRun,
  getAgentQueue,
  getAgentQueueStatus,
  listAgentMessages,
  listAgentRuns,
  runAgentJob,
  type AgentChatMessage,
  type AgentModelRef,
  type AgentQueueItem,
  type AgentRunSummary,
  type AgentSessionUsage,
} from '../api/agent'
import type { Source } from '../api/sources'
import type { ReleaseInfo } from '../api/releases'
import type { AgentEntityRefSeed } from '../injection-keys'

vi.mock('../api/agent', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/agent')>()
  return {
    ...actual,
    listAgentRuns: vi.fn().mockResolvedValue([]),
    listAgentMessages: vi.fn().mockResolvedValue([]),
    getAgentQueueStatus: vi.fn().mockResolvedValue(null),
    getAgentQueue: vi.fn().mockResolvedValue([]),
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

const deps = () => {
  const selectedModel = ref<AgentModelRef | null>(null)
  const oneShotModel = ref<AgentModelRef | null>(null)
  return {
    activeKey: ref('s1'),
    showToast: vi.fn(),
    instruction: ref(''),
    entities: ref<AgentEntityRefSeed[]>([]),
    skillPath: ref<string | null>(null),
    files: ref<string[]>([]),
    focusAtEnd: vi.fn(),
    // 复刻 models 域语义：单次覆盖优先，否则会话级（只读，cast 以匹配入参 Ref 类型）
    effectiveModel: computed(
      () => oneShotModel.value ?? selectedModel.value,
    ) as Ref<AgentModelRef | null>,
    selectedModel,
    oneShotModel,
    modelOnce: ref(false),
    usage: ref<AgentSessionUsage | null>(null),
    loadUsage: vi.fn().mockResolvedValue(undefined),
    sessionTitle: ref('标题'),
    persistSessionMeta: vi.fn(),
    showSkillMenu: ref(false),
    showEntityMenu: ref(false),
    loadRpcStatus: vi.fn().mockResolvedValue(undefined),
    sources: ref<Source[]>([]),
    releases: ref<ReleaseInfo[]>([]),
    queueActive: ref<AgentQueueItem[]>([]),
  }
}

type Deps = ReturnType<typeof deps>

// 在宿主组件 setup 内调用 composable（watch/onUnmounted 需要组件实例）；
// 挂到 document 上便于统一清理
const wrappers: { unmount: () => void }[] = []
function setup(over: Partial<Deps> = {}) {
  const d = { ...deps(), ...over } as Deps
  let api!: ReturnType<typeof useAgentChat>
  const wrapper = mount(
    defineComponent({
      setup() {
        api = useAgentChat(d)
        return {}
      },
      template: '<div/>',
    }),
    { attachTo: document.body },
  )
  wrappers.push(wrapper)
  return { api, d }
}

beforeEach(() => {
  vi.clearAllMocks()
  // clearAllMocks 不清实现：逐个重设默认值，防止上一用例的 mockResolvedValue 泄漏
  vi.mocked(listAgentRuns).mockResolvedValue([])
  vi.mocked(listAgentMessages).mockResolvedValue([])
  vi.mocked(getAgentQueueStatus).mockResolvedValue({ position: null, other_running: false, running_sessions: [] })
  vi.mocked(getAgentQueue).mockResolvedValue([])
  vi.mocked(runAgentJob).mockResolvedValue(101)
  vi.mocked(cancelAgentRun).mockResolvedValue(undefined)
  vi.useFakeTimers()
})
afterEach(() => {
  while (wrappers.length) wrappers.pop()?.unmount()
  document.body.innerHTML = ''
  vi.useRealTimers()
})

describe('useAgentChat 加载与合帧', () => {
  it('loadChat：并发加载 + 水位预清联动 + 提交兜底复位 + 活跃 run 时冻结快照', async () => {
    const { api, d } = setup()
    // 脏值铺垫：上一会话水位、提交兜底 id
    d.usage.value = { message_count: 9 } as AgentSessionUsage
    api.submittedRunId.value = 999
    vi.mocked(listAgentRuns).mockResolvedValue([makeRun({ status: 'running' })])
    vi.mocked(listAgentMessages).mockResolvedValue([userMsg('你好')])

    await api.loadChat()

    expect(d.usage.value).toBeNull() // loadUsage 返回前不闪现旧水位
    expect(d.loadUsage).toHaveBeenCalledTimes(1)
    expect(api.runs.value).toEqual([makeRun({ status: 'running' })])
    expect(api.messages.value).toEqual([userMsg('你好')])
    expect(api.submittedRunId.value).toBeNull() // runs 已刷新，兜底使命结束
    expect(api.messagesLoading.value).toBe(false)
    // 活跃 run 存在且流式未接管 → 历史冻结进快照（切回运行中会话不吞历史）
    expect(api.historySnapshot.value).toEqual([userMsg('你好')])
    expect(api.canStop.value).toBe(true)
  })

  it('合帧：50ms 一批按到达顺序处理 text/thinking/tool/bash，同 kind 追加、换 kind 新块', async () => {
    const { api } = setup()
    api.handleRpcStream(rpc('s1', ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: '你' } })))
    api.handleRpcStream(rpc('s1', ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: '好' } })))
    api.handleRpcStream(rpc('s1', ev({ type: 'message_update', assistantMessageEvent: { type: 'thinking_delta', delta: '想' } })))
    api.handleRpcStream(rpc('s1', ev({ type: 'tool_execution_start', toolCallId: 't1', toolName: 'bash', args: { cmd: 'ls' } })))
    api.handleRpcStream(rpc('s1', ev({ type: 'bash_execution_update', delta: 'out' })))

    await vi.advanceTimersByTimeAsync(50)

    expect(api.liveMessages.value.length).toBe(1)
    expect(api.liveMessages.value[0].role).toBe('assistant')
    expect(api.liveMessages.value[0].blocks).toEqual([
      { kind: 'text', text: '你好' },
      { kind: 'thinking', text: '想' },
      { kind: 'toolCall', id: 't1', name: 'bash', args: '{"cmd":"ls"}' },
      { kind: 'bash', command: '', output: 'out', exit_code: null, truncated: false },
    ])
    expect(api.displayedMessages.value).toEqual([...api.messages.value, ...api.liveMessages.value])
  })

  it('合帧：他session 事件丢弃；agent_settled 停轮询 + 清流式/快照 + 全量校准', async () => {
    const { api, d } = setup()
    api.handleRpcStream(rpc('other', ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: '串台' } })))
    api.startPolling() // 供 agent_settled 停掉；同时验证启动即刷新指示灯
    expect(d.loadRpcStatus).toHaveBeenCalledTimes(1)
    api.liveMessages.value = [{ role: 'assistant', blocks: [], timestamp: 't', model: null }]
    api.historySnapshot.value = [userMsg('snap')]

    api.handleRpcStream(rpc('s1', ev({ type: 'agent_settled' })))
    await vi.advanceTimersByTimeAsync(50)

    // 串台事件未写入；settled 清空流式与快照并触发 loadChat 全量校准
    expect(api.liveMessages.value).toEqual([])
    expect(api.historySnapshot.value).toEqual([])
    expect(vi.mocked(listAgentMessages).mock.calls.length).toBe(1)
    // 轮询已停：advance 一个周期后队列不再被拉取
    // （取样在 flush 后：settled 触发的 loadChat 自身已拉过一次队列）
    const queueCalls = vi.mocked(getAgentQueue).mock.calls.length
    await vi.advanceTimersByTimeAsync(1600)
    expect(vi.mocked(getAgentQueue).mock.calls.length).toBe(queueCalls)
  })

  it('两实例合帧状态隔离（pendingRpcEvents/rpcFlushTimer 为实例级，不串帧）', async () => {
    const { api: a } = setup()
    const { api: b } = setup({ activeKey: ref('s2') })
    a.handleRpcStream(rpc('s1', ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: 'A' } })))
    b.handleRpcStream(rpc('s2', ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: 'B' } })))

    await vi.advanceTimersByTimeAsync(50)

    expect(a.liveMessages.value[0].blocks).toEqual([{ kind: 'text', text: 'A' }])
    expect(b.liveMessages.value[0].blocks).toEqual([{ kind: 'text', text: 'B' }])
  })
})

describe('useAgentChat 提交 / 停止 / 重试', () => {
  it('handleSubmit：菜单打开不提交；空提交提示不发包', async () => {
    const { api, d } = setup()
    d.showSkillMenu.value = true
    await api.handleSubmit()
    expect(runAgentJob).not.toHaveBeenCalled()

    d.showSkillMenu.value = false
    await api.handleSubmit()
    expect(d.showToast).toHaveBeenCalledWith(t('agent.empty_job'))
    expect(runAgentJob).not.toHaveBeenCalled()
  })

  it('handleSubmit：实体合并去重、提交参数、消费一次性覆盖与附件、固化会话登记、启动轮询', async () => {
    const { api, d } = setup()
    // 提交后 loadChat 刷新出 pending run：活跃 run 由 runs 推导接管（兜底 id 复位）
    vi.mocked(listAgentRuns).mockResolvedValue([makeRun({ id: 101, status: 'pending' })])
    d.instruction.value = '帮我 [[source:1]] 分析'
    d.entities.value = [{ kind: 'release', id: 7 }]
    d.skillPath.value = 'E:\\x\\SKILL.md'
    d.oneShotModel.value = { provider: 'x', model_id: 'y' }
    d.modelOnce.value = true
    d.selectedModel.value = { provider: 'deepseek', model_id: 'm1' }
    d.files.value = ['C:/a.log']

    await api.handleSubmit()

    // effectiveModel = oneShot 优先（本次覆盖生效）；inline 实体去重后并入
    expect(runAgentJob).toHaveBeenCalledWith({
      sessionKey: 's1',
      entities: [{ kind: 'release', id: 7 }, { kind: 'source', id: 1 }],
      skillPath: 'E:\\x\\SKILL.md',
      instruction: '帮我  分析',
      model: { provider: 'x', model_id: 'y' },
      files: ['C:/a.log'],
    })    // 提交成功：清指令/一次性覆盖/附件；固化的是 selectedModel（会话长期选择）
    expect(d.instruction.value).toBe('')
    expect(d.oneShotModel.value).toBeNull()
    expect(d.modelOnce.value).toBe(false)
    expect(d.files.value).toEqual([])
    expect(d.persistSessionMeta).toHaveBeenCalledWith('s1', '帮我  分析', { provider: 'deepseek', model_id: 'm1' })
    // runId 兜底已随 loadChat 复位，活跃 run 由 runs 推导接管
    expect(api.canStop.value).toBe(true)
    expect(api.liveMessages.value[0].role).toBe('user')
    expect(api.historySnapshot.value).toEqual(api.messages.value)
    expect(api.submitting.value).toBe(false)
    // startPolling：启动即刷新指示灯
    expect(d.loadRpcStatus).toHaveBeenCalled()
  })

  it('handleSubmit：提交被拒时清本地回显与快照，submitting 复位', async () => {
    const { api, d } = setup()
    d.instruction.value = 'hi'
    vi.mocked(runAgentJob).mockRejectedValueOnce(new Error('boom'))

    await api.handleSubmit()

    expect(d.showToast).toHaveBeenCalledWith('Error: boom')
    expect(api.liveMessages.value).toEqual([])
    expect(api.historySnapshot.value).toEqual([])
    expect(api.submitting.value).toBe(false)
  })

  it('handleCancel：无可停 run 忽略；成功保持 cancelling 等终态；失败复位', async () => {
    const { api, d } = setup()
    await api.handleCancel() // activeRunId null
    expect(cancelAgentRun).not.toHaveBeenCalled()

    vi.mocked(listAgentRuns).mockResolvedValue([makeRun({ id: 5, status: 'running' })])
    await api.loadChat()
    await api.handleCancel()
    expect(cancelAgentRun).toHaveBeenCalledWith(5)
    expect(d.showToast).toHaveBeenCalledWith(t('agent.cancelling'))
    expect(api.cancelling.value).toBe(true) // 等终态事件刷新

    vi.mocked(cancelAgentRun).mockRejectedValueOnce(new Error('nope'))
    api.cancelling.value = false
    await api.handleCancel()
    expect(d.showToast).toHaveBeenLastCalledWith('Error: nope')
    expect(api.cancelling.value).toBe(false)
  })

  it('handleRetry：活跃 run 时阻止；否则回填输入区并原样重发', async () => {
    const { api, d } = setup()
    vi.mocked(listAgentRuns).mockResolvedValue([makeRun({ id: 5, status: 'running' })])
    await api.loadChat()
    await api.handleRetry(makeRun({ id: 9 }))
    expect(d.showToast).toHaveBeenCalledWith(t('agent.retry_blocked'))
    expect(runAgentJob).not.toHaveBeenCalled()

    vi.mocked(listAgentRuns).mockResolvedValue([])
    await api.loadChat()
    d.sources.value = [{ id: 1 } as Source]
    const run = makeRun({
      id: 9,
      status: 'failed',
      instruction: '重试指令',
      skill_path: 'E:\\s\\SKILL.md',
      entities: JSON.stringify([{ kind: 'source', id: 1 }, { kind: 'release', id: 99 }]),
      model: JSON.stringify({ provider: 'deepseek', model_id: 'm1' }),
      files: JSON.stringify(['C:/a.log']),
    })
    await api.handleRetry(run)
    // 已删除实体（release:99 不在目录）被剔除 + toast 告知；随后原样重发
    expect(d.showToast).toHaveBeenCalledWith(t('agent.retry_entities_dropped', '1'))
    expect(runAgentJob).toHaveBeenCalledWith(expect.objectContaining({
      instruction: '重试指令',
      entities: [{ kind: 'source', id: 1 }],
      skillPath: 'E:\\s\\SKILL.md',
      model: { provider: 'deepseek', model_id: 'm1' },
      files: ['C:/a.log'],
    }))
    expect(d.selectedModel.value).toEqual({ provider: 'deepseek', model_id: 'm1' })
  })

  it('handleRetryEdit：回填但不提交，光标送到输入框末尾', async () => {
    const { api, d } = setup()
    vi.mocked(listAgentRuns).mockResolvedValue([])
    await api.loadChat()
    const run = makeRun({ id: 9, instruction: '编辑它' })
    api.handleRetryEdit(run)
    await vi.advanceTimersByTimeAsync(0) // nextTick
    expect(d.instruction.value).toBe('编辑它')
    expect(runAgentJob).not.toHaveBeenCalled()
    expect(d.focusAtEnd).toHaveBeenCalled()
  })
})

describe('useAgentChat 会话切换清空（§4.2 三 mode 逐状态复刻）', () => {
  /** 铺垫：轮询中 + 待 flush 流式事件 + 提交/流式态脏值。 */
  function seed(api: ReturnType<typeof useAgentChat>) {
    api.startPolling()
    api.handleRpcStream(rpc('s1', ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: 'X' } })))
    api.submittedRunId.value = 9
    api.cancelling.value = true
    api.liveMessages.value = [{ role: 'assistant', blocks: [], timestamp: 't', model: null }]
    api.historySnapshot.value = [userMsg('snap')]
  }

  it('switch：停轮询 + 丢帧 + 提交/流式态复位；messages/runs 不清（loadChat 覆盖）', async () => {
    const { api } = setup()
    api.messages.value = [userMsg('m')]
    api.runs.value = [makeRun()]
    seed(api)

    api.resetForSessionSwitch('switch')

    expect(api.submittedRunId.value).toBeNull()
    expect(api.cancelling.value).toBe(false)
    expect(api.liveMessages.value).toEqual([])
    expect(api.historySnapshot.value).toEqual([])
    expect(api.messages.value.length).toBe(1)
    expect(api.runs.value.length).toBe(1)
    // 停轮询：advance 一个周期不再拉队列
    const queueCalls = vi.mocked(getAgentQueue).mock.calls.length
    await vi.advanceTimersByTimeAsync(1600)
    expect(vi.mocked(getAgentQueue).mock.calls.length).toBe(queueCalls)
    // 丢帧：合帧 timer 已清，残留 delta 永不写入（无幽灵流式消息）
    expect(api.liveMessages.value).toEqual([])
  })

  it('new：同 switch 但立即清 messages/runs，且不停轮询（原实现即如此）', async () => {
    const { api } = setup()
    api.messages.value = [userMsg('m')]
    api.runs.value = [makeRun()]
    seed(api)

    api.resetForSessionSwitch('new')

    expect(api.messages.value).toEqual([])
    expect(api.runs.value).toEqual([])
    expect(api.submittedRunId.value).toBeNull()
    expect(api.historySnapshot.value).toEqual([])
    // 轮询仍活：advance 一个周期队列照常拉取
    const queueCalls = vi.mocked(getAgentQueue).mock.calls.length
    await vi.advanceTimersByTimeAsync(1600)
    expect(vi.mocked(getAgentQueue).mock.calls.length).toBeGreaterThan(queueCalls)
    // 丢帧照旧（discardPendingRpcEvents 在 new mode 也执行）
    expect(api.liveMessages.value).toEqual([])
  })

  it('delete：一律不动（草稿/提交态/流式残留保留，残留 delta 照常 flush）', async () => {
    const { api } = setup()
    api.messages.value = [userMsg('m')]
    api.runs.value = [makeRun()]
    seed(api)

    api.resetForSessionSwitch('delete')

    expect(api.submittedRunId.value).toBe(9)
    expect(api.cancelling.value).toBe(true)
    expect(api.liveMessages.value.length).toBe(1)
    expect(api.historySnapshot.value).toEqual([userMsg('snap')])
    expect(api.messages.value.length).toBe(1)
    expect(api.runs.value.length).toBe(1)
    // 合帧 timer 未清：残留 delta 照常 flush 进流式消息（现状行为）
    await vi.advanceTimersByTimeAsync(50)
    expect(api.liveMessages.value[0].blocks).toEqual([{ kind: 'text', text: 'X' }])
  })
})

describe('useAgentChat 排队横幅提示', () => {
  it('queueHint/queueOccupiedBy：pending + 其他会话 running 时提示占用与位置', () => {
    const { api } = setup()
    api.runs.value = [makeRun({ status: 'pending' })]
    api.queueInfo.value = { position: 3, other_running: true, running_sessions: ['s2'] }
    expect(api.queueHint.value).toBe(t('agent.queue_other_running_pos', '3'))
    expect(api.queueOccupiedBy.value).toBe('s2')

    // 占用者即本会话 → 无「被谁占用」跳转
    api.queueInfo.value = { position: 1, other_running: true, running_sessions: ['s1'] }
    expect(api.queueOccupiedBy.value).toBeNull()

    // 非 pending → 无提示
    api.runs.value = [makeRun({ status: 'success' })]
    expect(api.queueHint.value).toBeNull()
    expect(api.queueOccupiedBy.value).toBeNull()
  })
})
