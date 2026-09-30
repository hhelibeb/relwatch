import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { computed, defineComponent, nextTick, ref } from 'vue'
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
    // 默认「目录已成功加载」：沿用既有剔除语义；「未加载不剔除」的用例单独覆盖
    catalogReady: ref({ source: true, release: true }),
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
  vi.mocked(getAgentQueueStatus).mockResolvedValue({ position: null, other_running: false, running_sessions: [], max_concurrency: 1, running_count: 0 })
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

  it('合帧：他 session 的事件写入它自己的分片（不串到当前会话）；settled 只清自己的', async () => {
    const { api, d } = setup()
    api.handleRpcStream(rpc('other', ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: '后台产出' } })))
    api.startPolling() // 供 agent_settled 停掉；同时验证启动即刷新指示灯
    expect(d.loadRpcStatus).toHaveBeenCalledTimes(1)
    api.liveMessages.value = [{ role: 'assistant', blocks: [], timestamp: 't', model: null }]
    api.historySnapshot.value = [userMsg('snap')]

    api.handleRpcStream(rpc('s1', ev({ type: 'agent_settled' })))
    await vi.advanceTimersByTimeAsync(50)

    // 当前会话（s1）：settled 清空流式与快照并触发 loadChat 全量校准；other 的 delta 未串进来
    expect(api.liveMessages.value).toEqual([])
    expect(api.historySnapshot.value).toEqual([])
    expect(vi.mocked(listAgentMessages).mock.calls.length).toBe(1)
    // 后台会话的流式内容留在它自己的分片里（切回去要看得见）
    d.activeKey.value = 'other'
    expect(api.liveMessages.value[0].blocks).toEqual([{ kind: 'text', text: '后台产出' }])
    d.activeKey.value = 's1'
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

  it('handleSubmit：实体合并去重、提交参数、消费一次性覆盖/附件/引用/技能、固化会话登记、启动轮询', async () => {
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
    })    // 提交成功：清指令/一次性覆盖/附件/引用/技能；固化的是 selectedModel（会话长期选择）
    expect(d.instruction.value).toBe('')
    expect(d.oneShotModel.value).toBeNull()
    expect(d.modelOnce.value).toBe(false)
    expect(d.files.value).toEqual([])
    // 引用与技能是一次性输入：本轮已由 run 承载（消息区有 chip / 徽章），输入区不再保留
    expect(d.entities.value).toEqual([])
    expect(d.skillPath.value).toBeNull()
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
    d.entities.value = [{ kind: 'source', id: 1 }]
    d.skillPath.value = 'E:\\x\\SKILL.md'
    d.files.value = ['C:/a.log']
    vi.mocked(runAgentJob).mockRejectedValueOnce(new Error('boom'))

    await api.handleSubmit()

    expect(d.showToast).toHaveBeenCalledWith('Error: boom')
    expect(api.liveMessages.value).toEqual([])
    expect(api.historySnapshot.value).toEqual([])
    expect(api.submitting.value).toBe(false)
    // 被拒 = 没有 run 承载这轮输入 → 指令/引用/技能/附件全保留，改一改即可重发
    // （与成功路径的清空刻意不对称，勿「顺手对齐」）
    expect(d.instruction.value).toBe('hi')
    expect(d.entities.value).toEqual([{ kind: 'source', id: 1 }])
    expect(d.skillPath.value).toBe('E:\\x\\SKILL.md')
    expect(d.files.value).toEqual(['C:/a.log'])
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

  it('handleRetry：目录从未成功加载时不剔除引用（未加载 ≠ 已删除，误剔是静默丢数据）', async () => {
    const { api, d } = setup({ catalogReady: ref({ source: false, release: false }) })
    vi.mocked(listAgentRuns).mockResolvedValue([])
    await api.loadChat()
    const run = makeRun({
      id: 9,
      status: 'failed',
      instruction: '重试指令',
      entities: JSON.stringify([{ kind: 'source', id: 1 }, { kind: 'release', id: 99 }]),
    })
    await api.handleRetry(run)
    expect(d.showToast).not.toHaveBeenCalledWith(t('agent.retry_entities_dropped', '2'))
    expect(runAgentJob).toHaveBeenCalledWith(expect.objectContaining({
      instruction: '重试指令',
      entities: [{ kind: 'source', id: 1 }, { kind: 'release', id: 99 }],
    }))
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

  it('重试前用户在当前会话显式换了模型：尊重当前选择，不被 run 的旧模型覆盖', async () => {
    const { api, d } = setup()
    vi.mocked(listAgentRuns).mockResolvedValue([])
    await api.loadChat()
    // 失败 run 用的是旧模型 m1；用户随后在下拉里换成了 m2（会写 selectedModel）
    d.selectedModel.value = { provider: 'deepseek', model_id: 'm2' }
    const run = makeRun({
      id: 9,
      status: 'failed',
      instruction: '再来一次',
      model: JSON.stringify({ provider: 'deepseek', model_id: 'm1' }),
    })
    await api.handleRetry(run)
    // 当前会话已显式选过模型 → 提交用它；run 的旧模型不覆盖
    expect(runAgentJob).toHaveBeenCalledWith(
      expect.objectContaining({ model: { provider: 'deepseek', model_id: 'm2' } }),
    )
  })

  it('重试时勾了「仅本次」：提交用一次性模型，会话长期选择不被 run 的旧模型污染', async () => {
    const { api, d } = setup()
    vi.mocked(listAgentRuns).mockResolvedValue([])
    await api.loadChat()
    // 用户勾「仅本次」选了 m3：只落一次性槽位，会话长期选择仍是默认（null）
    d.oneShotModel.value = { provider: 'deepseek', model_id: 'm3' }
    d.modelOnce.value = true
    const run = makeRun({
      id: 9,
      status: 'failed',
      instruction: '再来一次',
      model: JSON.stringify({ provider: 'deepseek', model_id: 'm1' }),
    })
    await api.handleRetry(run)
    // 提交用一次性模型（m3），失败那次的 m1 不参与
    expect(runAgentJob).toHaveBeenCalledWith(
      expect.objectContaining({ model: { provider: 'deepseek', model_id: 'm3' } }),
    )
    // 长期选择仍是默认：没被 run 的旧模型写脏，提交固化到会话的也是 null
    expect(d.selectedModel.value).toBeNull()
    expect(d.persistSessionMeta).toHaveBeenCalledWith(
      expect.any(String),
      expect.any(String),
      null,
    )
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

  it('switch：只停轮询；离开的会话状态（含在途 delta）留在它自己的分片里，切回即恢复', async () => {
    const { api, d } = setup() // activeKey = s1
    api.messages.value = [userMsg('m')]
    api.runs.value = [makeRun()]
    seed(api) // s1：轮询中 + 一条待 flush 的 delta + 提交/流式态脏值

    // 真实序列：AgentWorkspace.switchSession 先切 activeKey，再调复位
    d.activeKey.value = 's2'
    api.resetForSessionSwitch('switch')

    // 目标会话是空白分片：不受影响（无需清，也不该被清）
    expect(api.messages.value).toEqual([])
    expect(api.liveMessages.value).toEqual([])
    // 停轮询：advance 一个周期不再拉队列
    const queueCalls = vi.mocked(getAgentQueue).mock.calls.length
    await vi.advanceTimersByTimeAsync(1600)
    expect(vi.mocked(getAgentQueue).mock.calls.length).toBe(queueCalls)
    // 切回 s1：提交兜底 / 流式残留 / 快照都还在（它们本就是 s1 的状态）
    d.activeKey.value = 's1'
    expect(api.submittedRunId.value).toBe(9)
    expect(api.cancelling.value).toBe(true)
    expect(api.historySnapshot.value).toEqual([userMsg('snap')])
    expect(api.messages.value.length).toBe(1)
    expect(api.runs.value.length).toBe(1)
    // 在途 delta 未被丢弃：替自己的会话写进流式消息（后台会话的打字机不断档）
    await vi.advanceTimersByTimeAsync(50)
    expect(api.liveMessages.value.length).toBe(1)
    expect(api.liveMessages.value[0].blocks).toEqual([{ kind: 'text', text: 'X' }])
  })

  it('切回正在后台跑的会话：它的流式内容不被复位清掉（清它 = 把后台输出抹了）', async () => {
    const { api, d } = setup() // activeKey = s1
    api.handleRpcStream(rpc('s2', ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: '后台' } })))
    api.handleRpcStream(rpc('s2', ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: '产出' } })))
    await vi.advanceTimersByTimeAsync(50)
    expect(api.liveMessages.value).toEqual([]) // 当前是 s1，看不到 s2 的流式

    d.activeKey.value = 's2'
    api.resetForSessionSwitch('switch')

    // 切进来即可看到后台已产出的内容（新 delta 会接着往下追加，不是从头重建）
    expect(api.liveMessages.value.length).toBe(1)
    expect(api.liveMessages.value[0].blocks).toEqual([{ kind: 'text', text: '后台产出' }])
  })

  it('new：目标（空白会话）立即清空、不停轮询；离开的会话在途 delta 不被牵连', async () => {
    const { api, d } = setup()
    api.messages.value = [userMsg('m')]
    api.runs.value = [makeRun()]
    seed(api) // s1 脏值

    d.activeKey.value = 's-new' // registerNew 的实际效果：activeKey 换到新 key
    api.resetForSessionSwitch('new')

    expect(api.messages.value).toEqual([])
    expect(api.runs.value).toEqual([])
    expect(api.submittedRunId.value).toBeNull()
    expect(api.historySnapshot.value).toEqual([])
    expect(api.liveMessages.value).toEqual([])
    // 轮询仍活：advance 一个周期队列照常拉取
    const queueCalls = vi.mocked(getAgentQueue).mock.calls.length
    await vi.advanceTimersByTimeAsync(1600)
    expect(vi.mocked(getAgentQueue).mock.calls.length).toBeGreaterThan(queueCalls)
    // 离开的会话状态原样：在途 delta 仍在它自己的分片里 flush
    d.activeKey.value = 's1'
    expect(api.submittedRunId.value).toBe(9)
    expect(api.liveMessages.value[0].blocks).toEqual([{ kind: 'text', text: 'X' }])
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

describe('useAgentChat 并行会话分片（多会话同时运行）', () => {
  it('后台会话的流式事件写入自己的分片，切回后内容仍在（旧实现直接丢弃）', async () => {
    const { api, d } = setup() // activeKey = s1
    api.handleRpcStream(rpc('s2', ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: '后台' } })))
    api.handleRpcStream(rpc('s2', ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: '产出' } })))
    await vi.advanceTimersByTimeAsync(50)

    // 当前会话视图不受影响（不串台）
    expect(api.liveMessages.value).toEqual([])
    // 切到 s2：后台期间的流式内容完整保留
    d.activeKey.value = 's2'
    expect(api.liveMessages.value.length).toBe(1)
    expect(api.liveMessages.value[0].blocks).toEqual([{ kind: 'text', text: '后台产出' }])
  })

  it('agent_settled 只清自己的分片，其他会话进行中的流式不受牵连', async () => {
    const { api } = setup()
    api.handleRpcStream(rpc('s1', ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: 'A' } })))
    api.handleRpcStream(rpc('s2', ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: 'B' } })))
    await vi.advanceTimersByTimeAsync(50)
    expect(api.liveMessages.value[0].blocks).toEqual([{ kind: 'text', text: 'A' }])

    // s2 先结束：只清 s2 的分片，s1 的流式照旧
    api.handleRpcStream(rpc('s2', ev({ type: 'agent_settled' })))
    await vi.advanceTimersByTimeAsync(50)
    expect(api.liveMessages.value[0].blocks).toEqual([{ kind: 'text', text: 'A' }])
  })

  it('onRunFinished 只收尾该会话的分片，顺带刷新全局队列与进程状态', async () => {
    const { api, d } = setup()
    api.handleRpcStream(rpc('s1', ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: 'A' } })))
    api.handleRpcStream(rpc('s2', ev({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: 'B' } })))
    await vi.advanceTimersByTimeAsync(50)

    const queueCalls = vi.mocked(getAgentQueue).mock.calls.length
    await api.onRunFinished('s2')

    // 只对该会话做全量校准
    expect(vi.mocked(listAgentMessages)).toHaveBeenCalledWith('s2')
    d.activeKey.value = 's2'
    expect(api.liveMessages.value).toEqual([])
    d.activeKey.value = 's1'
    expect(api.liveMessages.value[0].blocks).toEqual([{ kind: 'text', text: 'A' }])
    // 队列与指示灯顺带刷新（其他会话结束会腾出并发位）
    expect(vi.mocked(getAgentQueue).mock.calls.length).toBeGreaterThan(queueCalls)
    expect(d.loadRpcStatus).toHaveBeenCalled()
  })

  it('分片回收：超出上限时丢非当前且无活跃 run 的最早分片，正在跑的后台会话保留', async () => {
    const { api, d } = setup()
    // s1 用「已结束的 run」当分片存活探针：它不阻止回收，但分片若还在就一定看得见
    // （全返回空列表的话，「已回收」与「分片还在但本来就是空的」无法区分）。
    vi.mocked(listAgentRuns).mockImplementation(async (key: string) => {
      if (key === 's2') return [makeRun({ session_key: 's2', status: 'running' })]
      if (key === 's1') return [makeRun({ session_key: 's1', status: 'success' })]
      return []
    })
    // 造 14 个会话分片（上限 12），其中 s2 持有活跃 run
    for (const k of ['s1', 's2', ...Array.from({ length: 12 }, (_, i) => `x${i}`)]) {
      d.activeKey.value = k
      await api.loadChat()
      await nextTick() // 回收挂在 watch(activeKey) 上，等它跑完再进下一个
    }

    // s2 的活跃 run 让分片免于回收：切回去仍看得到
    d.activeKey.value = 's2'
    await nextTick()
    expect(api.runs.value.map((r) => r.status)).toEqual(['running'])
    // s1 无活跃 run 且在最早位置 → 已被回收：切回去是重新加载前的空白分片
    d.activeKey.value = 's1'
    await nextTick()
    expect(api.runs.value).toEqual([])
  })
})

describe('useAgentChat 排队横幅提示', () => {
  it('queueHint/queueOccupiedBy：pending + 其他会话 running 时提示占用与位置', () => {
    const { api } = setup()
    api.runs.value = [makeRun({ status: 'pending' })]
    api.queueInfo.value = { position: 3, other_running: true, running_sessions: ['s2'], max_concurrency: 1, running_count: 1 }
    expect(api.queueHint.value).toBe(t('agent.queue_other_running_pos', '3'))
    expect(api.queueOccupiedBy.value).toBe('s2')

    // 占用者即本会话 → 无「被谁占用」跳转
    api.queueInfo.value = { position: 1, other_running: true, running_sessions: ['s1'], max_concurrency: 1, running_count: 1 }
    expect(api.queueOccupiedBy.value).toBeNull()

    // 非 pending → 无提示
    api.runs.value = [makeRun({ status: 'success' })]
    expect(api.queueHint.value).toBeNull()
    expect(api.queueOccupiedBy.value).toBeNull()
  })

  it('queueHint：并发上限 > 1 且未满时不提示排队，跑满才提示等待执行位', () => {
    const { api } = setup()
    api.runs.value = [makeRun({ status: 'pending' })]

    // 上限 3、只有 1 个在跑 → 还有空位，本会话马上就会被调度，不该说「被别会话挡住」
    api.queueInfo.value = { position: 1, other_running: true, running_sessions: ['s2'], max_concurrency: 3, running_count: 1 }
    expect(api.queueHint.value).toBeNull()
    expect(api.queueOccupiedBy.value).toBeNull()

    // 跑满 → 真排队
    api.queueInfo.value = { position: 4, other_running: true, running_sessions: ['s2'], max_concurrency: 3, running_count: 3 }
    expect(api.queueHint.value).toBe(t('agent.queue_limit_reached_pos', '4'))
    expect(api.queueOccupiedBy.value).toBe('s2')
  })
})
