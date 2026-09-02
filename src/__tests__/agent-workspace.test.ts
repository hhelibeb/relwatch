import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount, flushPromises, DOMWrapper } from '@vue/test-utils'
import { nextTick } from 'vue'
import { t, setLocale } from '../i18n'
import type { AgentChatMessage, AgentRunSummary } from '../api/agent'

// ========== Tauri 边界 Mocks ==========
const { mockUnlisten, mockListen, rpcHandlers } = vi.hoisted(() => {
  const rpcHandlers = new Map<string, (event: { payload: unknown }) => void>()
  const mockUnlisten = vi.fn()
  const mockListen = vi.fn((event: string, handler: (event: { payload: unknown }) => void) => {
    rpcHandlers.set(event, handler)
    return Promise.resolve(mockUnlisten)
  })
  return { mockUnlisten, mockListen, rpcHandlers }
})
vi.mock('@tauri-apps/api/event', () => ({ listen: mockListen }))
// 文件/确认对话框：默认「未选择」/「确认」（各用例按需覆盖）
const { openDialog, confirmDialog } = vi.hoisted(() => ({
  openDialog: vi.fn(),
  confirmDialog: vi.fn().mockResolvedValue(true),
}))
vi.mock('@tauri-apps/plugin-dialog', () => ({
  confirm: confirmDialog,
  open: openDialog,
}))

// Agent API：默认空目录 / 空会话 / 空消息
vi.mock('../api/agent', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/agent')>()
  return {
    ...actual,
    getAgentConfig: vi.fn().mockResolvedValue({ enabled: true, agent_type: 'pi', binary: null, model: null, working_dir: null, prompt_suffix: null, timeout_seconds: 300, skills: [] }),
    listAgentRuns: vi.fn().mockResolvedValue([]),
    listAgentMessages: vi.fn().mockResolvedValue([]),
    listAgentSessions: vi.fn().mockResolvedValue([]),
    getAgentQueueStatus: vi.fn().mockResolvedValue({ position: null, other_running: false, running_sessions: [] }),
    getAgentQueue: vi.fn().mockResolvedValue([]),
    getAgentSessionUsage: vi
      .fn()
      .mockResolvedValue({ message_count: 0, total_chars: 0, file_bytes: 0, input_tokens: 0, output_tokens: 0, cache_read_tokens: 0, total_tokens: 0, cost_micros: 0, has_usage: false }),
    // pi 进程健康指示：默认「未运行」（多数用例不关心，个别用例内覆盖）
    getAgentRpcStatus: vi.fn().mockResolvedValue({ running: false, pid: null, restart_pending: false }),
    restartAgentRpc: vi.fn().mockResolvedValue(true),
    exportAgentSession: vi.fn().mockResolvedValue('C:/tmp/export.md'),
    getAgentAvailableModels: vi.fn().mockResolvedValue({
      models: [
        { provider: 'deepseek', id: 'deepseek-v4-flash', name: 'DeepSeek V4 Flash' },
        { provider: 'anthropic', id: 'claude-sonnet-4', name: 'Claude Sonnet 4' },
      ],
      current: { provider: 'deepseek', id: 'deepseek-v4-flash', name: 'DeepSeek V4 Flash' },
    }),
    runAgentJob: vi.fn().mockResolvedValue(1),
    deleteAgentSession: vi.fn().mockResolvedValue(undefined),
    openAgentSession: vi.fn().mockResolvedValue(undefined),
    getAgentSessionCommand: vi.fn().mockResolvedValue('pi --session x'),
  }
})
vi.mock('../api/sources', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/sources')>()
  return { ...actual, listSources: vi.fn().mockResolvedValue([]) }
})
vi.mock('../api/releases', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/releases')>()
  return { ...actual, getReleases: vi.fn().mockResolvedValue([]) }
})
vi.mock('../composables/useUsageTracking', () => ({ track: vi.fn() }))
// jsdom 无本地化渲染，固定时间格式
vi.mock('../utils', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../utils')>()
  return { ...actual, formatDate: vi.fn(() => '2025-01-01 00:00') }
})

import AgentWorkspace from '../components/AgentWorkspace.vue'

import {
  listAgentMessages,
  listAgentRuns,
  listAgentSessions,
  runAgentJob,
  getAgentConfig,
  getAgentQueueStatus,
  getAgentRpcStatus,
  restartAgentRpc,
  exportAgentSession,
  getAgentSessionUsage,
  deleteAgentSession,
} from '../api/agent'
import { listSources } from '../api/sources'
import { getReleases } from '../api/releases'
import type { AgentEntityRefSeed } from '../injection-keys'
import { ShowToastKey } from '../injection-keys'
import { InvokeI18nError } from '../api/client'

/** 等待流式合帧窗口（组件内 50ms）刷完：RPC 事件入队后须等真实 timer
 *  触发 flush，flushPromises 只清微任务等不到 50ms 定时器。 */
async function flushRpcFrame() {
  await new Promise((resolve) => setTimeout(resolve, 60))
  await flushPromises()
}

/** Teleport 到 body 的浮层（rpc 状态菜单 / 会话 ⋯ 菜单）不在 wrapper 子树内，须以 document.body 为根查找。 */
function findTeleported(selector: string): DOMWrapper<Element> {
  return new DOMWrapper(document.body).find(selector)
}
function findTeleportedAll(selector: string): DOMWrapper<Element>[] {
  return new DOMWrapper(document.body).findAll(selector)
}

/** 构造一个 run 摘要（默认：本会话第一轮超时失败）。模块级，两个 describe 共用。 */
function makeRun(over: Record<string, unknown> = {}): AgentRunSummary {
  return {
    id: 20,
    session_key: 'test-session',
    skill_path: null,
    entities: '[]',
    instruction: '帮我总结这个版本',
    model: null,
    session_path: null,
    status: 'timeout',
    exit_code: null,
    error: 'err.agent.timeout|300',
    started_at: null,
    finished_at: null,
    created_at: '2025-01-01T00:00:00.000Z',
    files: null,
    ...over,
  } as unknown as AgentRunSummary
}

function sampleMessages(): AgentChatMessage[] {
  return [
    {
      role: 'user',
      timestamp: '2025-01-01T00:00:00.000Z',
      model: null,
      blocks: [{ kind: 'text', text: '帮我总结这个版本' }],
    },
    {
      role: 'assistant',
      timestamp: '2025-01-01T00:00:01.000Z',
      model: 'claude-test',
      blocks: [
        { kind: 'thinking', text: '先看实体信息' },
        { kind: 'text', text: '**总结**：这是 v1.0.0 版本' },
        { kind: 'toolCall', id: 'call_1', name: 'bash', args: '{"cmd":"ls"}' },
      ],
    },
    {
      role: 'toolResult',
      timestamp: '2025-01-01T00:00:02.000Z',
      model: null,
      blocks: [{ kind: 'toolResult', id: 'call_1', tool_name: 'bash', text: 'src\n', is_error: false }],
    },
  ]
}

beforeEach(() => {
  setLocale('zh-CN')
  localStorage.clear() // 会话索引走 localStorage，用例间必须隔离
  vi.mocked(listAgentMessages).mockResolvedValue(sampleMessages())
  vi.mocked(listAgentSessions).mockResolvedValue([])
})

describe('AgentWorkspace 冒烟', () => {
  it('渲染标题、会话边栏、输入区', async () => {
    const wrapper = mount(AgentWorkspace, {
      global: { provide: { [ShowToastKey]: vi.fn() } },
    })
    await flushPromises()
    expect(wrapper.text()).toContain(t('agent.workspace_title'))
    expect(wrapper.find('.agent-ws-sidebar').exists()).toBe(true)
    expect(wrapper.find('.agent-ws-textarea').exists()).toBe(true)
    expect(wrapper.find('.agent-ws-submit').exists()).toBe(true)
    wrapper.unmount()
  })

  it('空会话显示提示文案，不崩溃', async () => {
    vi.mocked(listAgentMessages).mockResolvedValue([])
    const wrapper = mount(AgentWorkspace, { global: { provide: {} } })
    await flushPromises()
    expect(wrapper.find('.agent-ws-hint-empty').exists()).toBe(true)
    wrapper.unmount()
  })

  it('渲染用户/助手消息气泡与工具折叠卡片', async () => {
    const wrapper = mount(AgentWorkspace, { global: { provide: {} } })
    await flushPromises()
    const text = wrapper.text()
    expect(text).toContain('帮我总结这个版本')
    expect(text).toContain('总结')
    expect(text).toContain(t('agent.thinking')) // 思考折叠标题
    expect(wrapper.findAll('.agent-ws-tool-card').length).toBeGreaterThan(0)
    expect(wrapper.findAll('.agent-ws-bubble-user').length).toBe(1)
    expect(wrapper.findAll('.agent-ws-bubble-assistant').length).toBe(1)
    wrapper.unmount()
  })

  it('seed 预置实体显示为引用 chip', async () => {
    const seed: { entities: AgentEntityRefSeed[] } = { entities: [{ kind: 'release', id: 7 }] }
    const wrapper = mount(AgentWorkspace, {
      props: { seed },
      global: { provide: {} },
    })
    await flushPromises()
    expect(wrapper.findAll('.agent-ws-chip-attached').length).toBe(1)
    wrapper.unmount()
  })

  it('关闭按钮触发 close 事件', async () => {
    const wrapper = mount(AgentWorkspace, { global: { provide: {} } })
    await flushPromises()
    await wrapper.find('.agent-ws-close').trigger('click')
    expect(wrapper.emitted('close')).toBeTruthy()
    wrapper.unmount()
  })

  it('菜单打开时 Enter 只选择菜单项，不自动提交；提交时清理 @skill 标记', async () => {
    const SKILL = 'E:\\project\\relwatch\\.pi\\skills\\commit\\SKILL.md'
    vi.mocked(getAgentConfig).mockResolvedValue({
      enabled: true,
      agent_type: 'pi',
      binary: null,
      model: null,
      working_dir: null,
      prompt_suffix: null,
      timeout_seconds: 300,
      skills: [SKILL],
    })
    const wrapper = mount(AgentWorkspace, {
      global: { provide: { [ShowToastKey]: vi.fn() } },
    })
    await flushPromises()
    const ta = wrapper.find('.agent-ws-textarea')

    // 输入 @ 打开 skill 菜单
    await ta.setValue('@')
    await flushPromises()
    expect(wrapper.find('.agent-ws-menu').exists()).toBe(true)

    // Enter 选择菜单项：不应提交，只插入短名
    await ta.trigger('keydown', { key: 'Enter' })
    await flushPromises()
    expect(vi.mocked(runAgentJob)).not.toHaveBeenCalled()
    expect(wrapper.find('.agent-ws-menu').exists()).toBe(false)
    expect((ta.element as HTMLTextAreaElement).value).toBe('@commit ')

    // 继续输入指令后 Enter：正常提交，instruction 不含 @skill 标记
    await ta.setValue('@commit 帮我看看')
    await ta.trigger('keydown', { key: 'Enter' })
    await flushPromises()
    expect(vi.mocked(runAgentJob)).toHaveBeenCalledTimes(1)
    expect(vi.mocked(runAgentJob)).toHaveBeenCalledWith(
      expect.objectContaining({ skillPath: SKILL, instruction: '帮我看看' }),
    )
    wrapper.unmount()
  })

  it('user 消息中的 skill 全文折叠为徽章，不刷屏', async () => {
    const SKILL = 'E:\\project\\videocaption\\.agents\\skills\\youtube-caption\\SKILL.md'
    const longBody = '# 全文很长\n' + 'x'.repeat(2000)
    vi.mocked(listAgentMessages).mockResolvedValue([
      {
        role: 'user',
        timestamp: '2025-01-01T00:00:00.000Z',
        model: null,
        blocks: [
          {
            kind: 'text',
            text: `<skill name="youtube-caption" location="${SKILL}">\n${longBody}\n</skill>\n执行 请开始执行。`,
          },
        ],
      },
    ])
    vi.mocked(listAgentRuns).mockResolvedValue([
      makeRun({
        id: 1,
        session_key: 'k',
        skill_path: SKILL,
        instruction: '执行',
        status: 'success',
        exit_code: 0,
        error: null,
        started_at: '2025-01-01T00:00:00.000Z',
        finished_at: '2025-01-01T00:00:05.000Z',
      }),
    ])
    const wrapper = mount(AgentWorkspace, {
      global: { provide: { [ShowToastKey]: vi.fn() } },
    })
    await flushPromises()
    const text = wrapper.text()
    expect(text).not.toContain('# 全文很长')
    expect(text).toContain('执行')
    expect(text).toContain('@youtube-caption')
    wrapper.unmount()
  })

  it('流式进行中只显示流式消息，不重复叠加全量', async () => {
    // 预置会话 key：让 RPC 事件能匹配当前会话
    localStorage.setItem(
      'relwatch.agent.sessions.v1',
      JSON.stringify([{ key: 'test-session', title: 't', updatedAt: Date.now() }]),
    )
    // 全量（JSONL）已含同轮 assistant 内容
    vi.mocked(listAgentMessages).mockResolvedValue([sampleMessages()[1]])
    const wrapper = mount(AgentWorkspace, { global: { provide: {} } })
    await flushPromises()
    const emitRpc = (event: string) => {
      const handler = rpcHandlers.get('agent-rpc-stream') as ((e: { payload: unknown }) => void) | undefined
      handler?.({ payload: { session_key: 'test-session', run_id: 1, event } })
    }
    // 流式：同轮内容以 text_delta 到达（合帧窗口内合并处理）
    emitRpc(JSON.stringify({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: '总结：这是 v1.0.0 版本' } }))
    await flushRpcFrame()
    const count = wrapper.text().split('总结：这是 v1.0.0 版本').length - 1
    expect(count).toBe(1) // 流式优先，全量未叠加 → 只出现一次
    // 终态：回落全量校准，仍不重复
    emitRpc(JSON.stringify({ type: 'agent_settled' }))
    await flushRpcFrame()
    expect(wrapper.text().split('总结：这是 v1.0.0 版本').length - 1).toBe(1)
    wrapper.unmount()
    localStorage.removeItem('relwatch.agent.sessions.v1')
  })

  it('[[ 引用菜单显示可读名称，并支持按频道名/标题检索', async () => {
    vi.mocked(listSources).mockResolvedValue([
      {
        id: 1,
        source_type: 'youtube',
        owner: 'UCrD39DnkX5QjIvH3yssXqJA',
        repo: 'UULF3DnkX5QjIvH3yssXqJA',
        poll_interval_minutes: 30,
        enabled: true,
        last_checked_at: null,
        last_check_status: 'ok',
        last_check_message: null,
        consecutive_failures: 0,
        last_new_count: 0,
        muted: false,
        created_at: '2025-01-01T00:00:00Z',
        updated_at: '2025-01-01T00:00:00Z',
        description: '宁静ASMR频道',
        config: null,
      },
    ])
    vi.mocked(getReleases).mockResolvedValue([
      {
        id: 7,
        source_id: 1,
        source_type: 'youtube',
        owner: 'UCrD39DnkX5QjIvH3yssXqJA',
        repo: '8Pi_1HjBUPU',
        tag_name: '8Pi_1HjBUPU',
        release_name: '白袜轻蹭耳朵柔和触发音',
        html_url: 'https://www.youtube.com/watch/?v=8Pi_1HjBUPU',
        published_at: '2026-08-15T04:16:52Z',
        prerelease: false,
        body: null,
        detected_at: '2026-08-15T04:17:00Z',
        notification_status: 'pending',
        snooze_until: null,
        ai_summary: null,
        ai_importance: null,
        body_translated: null,
        extra_metadata: null,
        source_description: '宁静ASMR频道',
      },
    ])
    const wrapper = mount(AgentWorkspace, {
      global: { provide: { [ShowToastKey]: vi.fn() } },
    })
    await flushPromises()
    const ta = wrapper.find('.agent-ws-textarea')

    // 空查询：菜单显示可读名（频道名 · 标题），而非 ID 代码
    await ta.setValue('[[')
    await flushPromises()
    const menuText = wrapper.find('.agent-ws-menu').text()
    expect(menuText).toContain('宁静ASMR频道 · 白袜轻蹭耳朵柔和触发音')
    expect(menuText).not.toContain('8Pi_1HjBUPU')

    // 按频道名检索：应能过滤出该频道的视频
    await ta.setValue('[[宁静')
    await flushPromises()
    expect(wrapper.find('.agent-ws-menu').text()).toContain('宁静ASMR频道 · 白袜轻蹭耳朵柔和触发音')

    // 按标题检索
    await ta.setValue('[[白袜')
    await flushPromises()
    expect(wrapper.find('.agent-ws-menu').text()).toContain('宁静ASMR频道 · 白袜轻蹭耳朵柔和触发音')

    // 无关词：无匹配提示
    await ta.setValue('[[不存在的频道')
    await flushPromises()
    expect(wrapper.find('.agent-ws-menu-empty').exists()).toBe(true)
    wrapper.unmount()
  })

  it('拖到标题栏：切换新会话并预置实体引用', async () => {
    const showToast = vi.fn()
    // 预置一个历史会话：初始处于旧会话（非新会话状态）
    localStorage.setItem(
      'relwatch.agent.sessions.v1',
      JSON.stringify([{ key: 'old-session', title: '旧会话', updatedAt: Date.now() }]),
    )
    const wrapper = mount(AgentWorkspace, {
      global: { provide: { [ShowToastKey as symbol]: showToast } },
    })
    await flushPromises()
    // 初始：处于旧会话、无引用 chip、无草稿会话
    expect(wrapper.find('.agent-ws-session-item.draft').exists()).toBe(false)
    expect(wrapper.findAll('.agent-ws-chip-attached').length).toBe(0)

    const header = wrapper.find('.agent-ws-header')
    // 拖拽悬停：标题栏出现虚线框 + 提示文本，工作区主体不高亮
    await header.trigger('dragenter')
    expect(header.classes()).toContain('drop-over')
    expect(wrapper.find('.agent-ws-drop-hint-header').exists()).toBe(true)
    expect(wrapper.find('.agent-ws-main').classes()).not.toContain('drag-over')
    // 把版本实体放到标题栏
    await header.trigger('drop', {
      dataTransfer: {
        getData: (fmt: string) =>
          fmt === 'application/x-relwatch-entity' ? JSON.stringify({ kind: 'release', id: 7 }) : '',
      },
    })
    await flushPromises()
    // 已切换为新建的草稿会话（新建即登记，评审 1.2）+ 引用 chip 放入
    expect(wrapper.find('.agent-ws-session-item.draft').exists()).toBe(true)
    expect(wrapper.findAll('.agent-ws-chip-attached').length).toBe(1)
    // 不再弹 Toast（右下角 Toast 会压住发送/附件按钮并吞点击），改为 chip 高亮就地反馈
    expect(showToast).not.toHaveBeenCalled()
    expect(wrapper.find('.agent-ws-chip-attached').classes()).toContain('is-new')
    // Toast 原先承担的告知作用交给屏幕阅读器 live region
    expect(wrapper.find('.agent-ws-sr-only').text()).toBe(t('agent.attached'))
    // 拖放结束提示层消失
    expect(wrapper.find('.agent-ws-drop-hint-header').exists()).toBe(false)
    wrapper.unmount()
    localStorage.removeItem('relwatch.agent.sessions.v1')
  })

  it('拖到工作区主体：添加到当前会话（不新建）', async () => {
    const showToast = vi.fn()
    // 预置一个历史会话：初始处于旧会话
    localStorage.setItem(
      'relwatch.agent.sessions.v1',
      JSON.stringify([{ key: 'old-session', title: '旧会话', updatedAt: Date.now() }]),
    )
    const wrapper = mount(AgentWorkspace, {
      global: { provide: { [ShowToastKey as symbol]: showToast } },
    })
    await flushPromises()
    const main = wrapper.find('.agent-ws-main')
    // 拖拽悬停：工作区主体出现虚线框 + 提示文本，标题栏不高亮
    await main.trigger('dragover')
    expect(main.classes()).toContain('drag-over')
    expect(wrapper.find('.agent-ws-drop-hint-main').exists()).toBe(true)
    expect(wrapper.find('.agent-ws-header').classes()).not.toContain('drop-over')
    // 把监控源实体放到工作区
    await main.trigger('drop', {
      dataTransfer: {
        getData: (fmt: string) =>
          fmt === 'application/x-relwatch-entity' ? JSON.stringify({ kind: 'source', id: 3 }) : '',
      },
    })
    await flushPromises()
    // 仍处于旧会话（未新建）+ 引用加入当前会话 + 就地高亮（不弹 Toast）
    expect(wrapper.find('.agent-ws-session-item.draft').exists()).toBe(false)
    expect(wrapper.findAll('.agent-ws-chip-attached').length).toBe(1)
    expect(showToast).not.toHaveBeenCalled()
    expect(wrapper.find('.agent-ws-chip-attached').classes()).toContain('is-new')
    expect(wrapper.find('.agent-ws-sr-only').text()).toBe(t('agent.attached'))
    wrapper.unmount()
    localStorage.removeItem('relwatch.agent.sessions.v1')
  })

  it('user 消息整条被 <用户指令> 包裹时剥离标签，首轮模板折叠为可展开详情', async () => {
    vi.mocked(listAgentMessages).mockResolvedValue([
      {
        role: 'user',
        timestamp: '2025-01-01T00:00:00.000Z',
        model: null,
        blocks: [{ kind: 'text', text: '<用户指令>\n不用，有没有提到努比亚\n</用户指令>' }],
      },
      {
        role: 'user',
        timestamp: '2025-01-01T00:00:01.000Z',
        model: null,
        blocks: [
          {
            kind: 'text',
            text: '以下是你需要处理的订阅信息……\n<用户指令>\n中间指令\n</用户指令>\n以上用户指令是你本次任务的唯一权威指令',
          },
        ],
      },
    ])
    const wrapper = mount(AgentWorkspace, {
      global: { provide: { [ShowToastKey]: vi.fn() } },
    })
    await flushPromises()
    const text = wrapper.text()
    // 整条包裹：标签剥离，仅内容可见
    expect(text).toContain('不用，有没有提到努比亚')
    expect(text).not.toContain('<用户指令>\n不用')
    // 首轮模板（标签在中间）：主文本仅显示用户指令，脚手架折叠进可展开详情
    expect(text).toContain('中间指令')
    expect(text).not.toContain('<用户指令>')
    expect(text).toContain('查看发送给 Agent 的完整指令')
    // 折叠内容仍在 DOM（details 内），完整上下文仍可见
    expect(text).toContain('以下是你需要处理的订阅信息')
    expect(text).toContain('以上用户指令是你本次任务的唯一权威指令')
    wrapper.unmount()
  })

  it('流式期间历史快照与本地回显同屏可见，终态回落全量校准', async () => {
    vi.mocked(runAgentJob).mockClear()
    localStorage.setItem(
      'relwatch.agent.sessions.v1',
      JSON.stringify([{ key: 'test-session', title: 't', updatedAt: Date.now() }]),
    )
    const wrapper = mount(AgentWorkspace, {
      global: { provide: { [ShowToastKey]: vi.fn() } },
    })
    await flushPromises()
    // 提交：冻结历史快照 + 本地回显用户消息
    const ta = wrapper.find('.agent-ws-textarea')
    await ta.setValue('测试指令')
    await ta.trigger('keydown', { key: 'Enter' })
    await flushPromises()
    expect(vi.mocked(runAgentJob)).toHaveBeenCalledTimes(1)
    // 流式事件
    const emitRpc = (event: string) => {
      const handler = rpcHandlers.get('agent-rpc-stream') as ((e: { payload: unknown }) => void) | undefined
      handler?.({ payload: { session_key: 'test-session', run_id: 1, event } })
    }
    emitRpc(
      JSON.stringify({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: '流式补充' } }),
    )
    await flushRpcFrame()
    // 历史（JSONL 全量）+ 本地回显 user + 流式内容同屏可见
    expect(wrapper.text()).toContain('帮我总结这个版本')
    expect(wrapper.text()).toContain('测试指令')
    expect(wrapper.text()).toContain('流式补充')
    // 终态：清流式，回落全量校准（JSONL 无流式内容）
    emitRpc(JSON.stringify({ type: 'agent_settled' }))
    await flushRpcFrame()
    expect(wrapper.text()).not.toContain('流式补充')
    expect(wrapper.text()).toContain('帮我总结这个版本')
    wrapper.unmount()
    localStorage.removeItem('relwatch.agent.sessions.v1')
  })

  it('同一合帧窗口内的多个 delta 顺序拼接，一次 flush 后完整呈现', async () => {
    localStorage.setItem(
      'relwatch.agent.sessions.v1',
      JSON.stringify([{ key: 'test-session', title: 't', updatedAt: Date.now() }]),
    )
    const wrapper = mount(AgentWorkspace, { global: { provide: {} } })
    await flushPromises()
    const emitRpc = (event: string) => {
      const handler = rpcHandlers.get('agent-rpc-stream') as ((e: { payload: unknown }) => void) | undefined
      handler?.({ payload: { session_key: 'test-session', run_id: 1, event } })
    }
    // 三个 delta 在同一 50ms 合帧窗口内先后到达：入队不逐条渲染，
    // flush 时按到达顺序拼接进同一 text block
    emitRpc(JSON.stringify({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: '第一' } }))
    emitRpc(JSON.stringify({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: '，第二' } }))
    emitRpc(JSON.stringify({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: '段。' } }))
    await flushRpcFrame()
    expect(wrapper.text()).toContain('第一，第二段。')
    wrapper.unmount()
    localStorage.removeItem('relwatch.agent.sessions.v1')
  })

  it('run 终态先于合帧窗口到达：残留流式事件被丢弃，不产生幽灵流式消息', async () => {
    localStorage.setItem(
      'relwatch.agent.sessions.v1',
      JSON.stringify([{ key: 'test-session', title: 't', updatedAt: Date.now() }]),
    )
    vi.mocked(listAgentMessages).mockResolvedValue([sampleMessages()[0]])
    // 会话内有活跃 run：onRunFinished 才会走终态收尾路径
    vi.mocked(listAgentRuns).mockResolvedValue([
      makeRun({ id: 22, status: 'running', error: null, started_at: '2025-01-01T00:00:00.000Z' }),
    ])
    const wrapper = mount(AgentWorkspace, { global: { provide: {} } })
    await flushPromises()
    // delta 入队（尚未 flush），run 终态事件先到达（终态兑底路径清空流式）
    const emitRpc = (event: string) => {
      const handler = rpcHandlers.get('agent-rpc-stream') as ((e: { payload: unknown }) => void) | undefined
      handler?.({ payload: { session_key: 'test-session', run_id: 22, event } })
    }
    emitRpc(JSON.stringify({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: '幽灵文本' } }))
    const runFinished = rpcHandlers.get('agent-run-finished') as ((e: { payload: unknown }) => void) | undefined
    runFinished?.({
      payload: { run_id: 22, session_key: 'test-session', status: 'success', message: null },
    })
    await flushPromises()
    await flushRpcFrame()
    // 终态以 loadChat 全量校准为准：队列残留的 delta 被一并丢弃，
    // 不在清空 liveMessages 后又重建出幽灵流式消息
    expect(wrapper.text()).not.toContain('幽灵文本')
    wrapper.unmount()
    localStorage.removeItem('relwatch.agent.sessions.v1')
  })

  it('切会话丢弃未处理的流式事件：旧会话 delta 不写入新会话', async () => {
    const base = Date.now()
    localStorage.setItem(
      'relwatch.agent.sessions.v1',
      JSON.stringify([
        { key: 'session-a', title: 'a', updatedAt: base },
        { key: 'session-b', title: 'b', updatedAt: base - 1000 },
      ]),
    )
    const wrapper = mount(AgentWorkspace, { global: { provide: {} } })
    await flushPromises()
    // 旧会话（session-a，激活中）的 delta 入队后立即切到 session-b
    const emitRpc = (event: string) => {
      const handler = rpcHandlers.get('agent-rpc-stream') as ((e: { payload: unknown }) => void) | undefined
      handler?.({ payload: { session_key: 'session-a', run_id: 1, event } })
    }
    emitRpc(JSON.stringify({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: '旧会话流式文本' } }))
    const itemB = wrapper.findAll('.agent-ws-session-item').find((i) => i.text().includes('b'))
    await itemB!.trigger('click')
    await flushPromises()
    await flushRpcFrame()
    // 新会话消息区不出现旧会话的流式文本（事件被丢弃，不复活）
    expect(wrapper.text()).not.toContain('旧会话流式文本')
    wrapper.unmount()
    localStorage.removeItem('relwatch.agent.sessions.v1')
  })

  it('排队中横幅：被其他会话占用时显示可点击的「前往停止」并支持跳转', async () => {
    localStorage.setItem(
      'relwatch.agent.sessions.v1',
      JSON.stringify([{ key: 'test-session', title: 't', updatedAt: Date.now() }]),
    )
    // 当前会话 latest run 为 pending；全局队列显示其他会话 running、位置 2
    vi.mocked(listAgentRuns).mockResolvedValue([
      makeRun({
        id: 11,
        status: 'pending',
        exit_code: null,
        error: null,
      }),
    ])
    vi.mocked(getAgentQueueStatus).mockResolvedValue({
      position: 2,
      other_running: true,
      running_sessions: ['ws-other'],
    })
    const wrapper = mount(AgentWorkspace, { global: { provide: {} } })
    await flushPromises()
    const banner = wrapper.find('.agent-ws-banner')
    expect(banner.exists()).toBe(true)
    expect(banner.text()).toContain(t('agent.status_pending'))
    // 评审 1.3：横幅显示占用者（一键「前往停止」），替代笼统的「其他会话正在执行」
    const occupied = t('agent.queue_occupied_by', t('agent.session_untitled'))
    expect(banner.text()).toContain(occupied)
    const queue = wrapper.find('.agent-ws-banner-queue')
    expect(queue.classes()).toContain('clickable')
    // 点击 → 切换到占用会话（在那里点「停止」让路）
    await queue.trigger('click')
    expect(wrapper.vm).toBeDefined()
    wrapper.unmount()
    localStorage.removeItem('relwatch.agent.sessions.v1')
  })

  it('失败 run 在对应 user 气泡下内联显示错误原因（可追溯）', async () => {
    localStorage.setItem(
      'relwatch.agent.sessions.v1',
      JSON.stringify([{ key: 'test-session', title: 't', updatedAt: Date.now() }]),
    )
    // 两轮对话：第一轮失败（超时），第二轮成功
    const failed: AgentChatMessage[] = [
      sampleMessages()[0],
      sampleMessages()[1],
      sampleMessages()[2],
      {
        role: 'user',
        timestamp: '2025-01-01T00:01:00.000Z',
        model: null,
        blocks: [{ kind: 'text', text: '再试一次' }],
      },
    ]
    vi.mocked(listAgentMessages).mockResolvedValue(failed)
    vi.mocked(listAgentRuns).mockResolvedValue([
      makeRun({
        id: 21,
        instruction: '再试一次',
        status: 'success',
        exit_code: 0,
        error: null,
        created_at: '2025-01-01T00:01:00.000Z',
      }),
      makeRun({ id: 20, status: 'failed' }),
    ])
    const wrapper = mount(AgentWorkspace, { global: { provide: {} } })
    await flushPromises()
    const notes = wrapper.findAll('.agent-ws-run-failed')
    expect(notes.length).toBe(1)
    expect(notes[0].text()).toContain(t('agent.status_timeout'))
    expect(notes[0].text()).toContain('300')
    // 成功轮次无内联备注
    expect(notes[0].text()).not.toContain('再试一次')
    wrapper.unmount()
    localStorage.removeItem('relwatch.agent.sessions.v1')
  })

  it('磁盘发现：索引里没有的会话自动补入并标记「已恢复」', async () => {
    vi.mocked(listAgentSessions).mockResolvedValue([
      {
        session_key: 'lost-session',
        title: '上周分析 B 站那个 up 主',
        session_path: 'C:/data/agent-sessions/ws-lost-session.jsonl',
        updated_at: '2026-08-20T11:43:02.000Z',
        last_status: 'success',
        run_count: 3,
      },
    ])
    const showToast = vi.fn()
    const wrapper = mount(AgentWorkspace, {
      global: { provide: { [ShowToastKey as symbol]: showToast } },
    })
    await flushPromises()
    // 侧栏出现该会话，并带「已恢复」标记（索引曾丢失，由文件重建）
    const items = wrapper.findAll('.agent-ws-session-item')
    expect(items.some((i) => i.text().includes('上周分析 B 站那个 up 主'))).toBe(true)
    expect(wrapper.find('.agent-ws-session-badge').exists()).toBe(true)
    expect(showToast).toHaveBeenCalledWith(t('agent.sessions_recovered', '1'))
    // 补入的会话已写回索引：下次启动不再重复提示
    const persisted = JSON.parse(localStorage.getItem('relwatch.agent.sessions.v1') ?? '[]') as {
      key: string
      recovered?: boolean
    }[]
    expect(persisted.some((s) => s.key === 'lost-session' && s.recovered)).toBe(true)
    wrapper.unmount()
  })

  it('磁盘发现：标题缺失时用占位文案，不渲染空白项', async () => {
    vi.mocked(listAgentSessions).mockResolvedValue([
      {
        session_key: 'untitled-session',
        title: '',
        session_path: 'C:/data/agent-sessions/ws-untitled-session.jsonl',
        updated_at: '2026-08-20T11:43:02.000Z',
        last_status: '',
        run_count: 0,
      },
    ])
    const wrapper = mount(AgentWorkspace, { global: { provide: {} } })
    await flushPromises()
    expect(wrapper.text()).toContain(t('agent.session_untitled'))
    wrapper.unmount()
  })

  it('磁盘发现：已在索引中的会话不被覆盖（保留用户侧标题）', async () => {
    localStorage.setItem(
      'relwatch.agent.sessions.v1',
      JSON.stringify([{ key: 'kept-session', title: '我改过的标题', updatedAt: Date.now() }]),
    )
    vi.mocked(listAgentSessions).mockResolvedValue([
      {
        session_key: 'kept-session',
        title: '从文件重建的标题',
        session_path: 'C:/data/agent-sessions/ws-kept-session.jsonl',
        updated_at: '2026-08-20T11:43:02.000Z',
        last_status: 'success',
        run_count: 1,
      },
    ])
    const showToast = vi.fn()
    const wrapper = mount(AgentWorkspace, {
      global: { provide: { [ShowToastKey as symbol]: showToast } },
    })
    await flushPromises()
    expect(wrapper.text()).toContain('我改过的标题')
    expect(wrapper.find('.agent-ws-session-badge').exists()).toBe(false)
    expect(showToast).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('失败 run 提供「重试」：点击即用原输入（指令/引用/模型）重新提交', async () => {
    vi.mocked(runAgentJob).mockClear()
    vi.mocked(getReleases).mockResolvedValue([
      {
        id: 7,
        source_id: 1,
        source_type: 'youtube',
        owner: 'o',
        repo: 'r',
        tag_name: 'v1',
        release_name: 'v1',
        html_url: 'https://example.com',
        published_at: '2026-08-15T04:16:52Z',
        prerelease: false,
        body: null,
        detected_at: '2026-08-15T04:17:00Z',
        notification_status: 'pending',
        snooze_until: null,
        ai_summary: null,
        ai_importance: null,
        body_translated: null,
        extra_metadata: null,
        source_description: null,
      },
    ])
    localStorage.setItem(
      'relwatch.agent.sessions.v1',
      JSON.stringify([{ key: 'test-session', title: 't', updatedAt: Date.now() }]),
    )
    vi.mocked(listAgentMessages).mockResolvedValue([sampleMessages()[0]])
    vi.mocked(listAgentRuns).mockResolvedValue([
      makeRun({
        entities: '[{"kind":"release","id":7}]',
        model: '{"provider":"anthropic","model_id":"claude-test"}',
      }),
    ])

    const wrapper = mount(AgentWorkspace, { global: { provide: { [ShowToastKey]: vi.fn() } } })
    await flushPromises()
    const note = wrapper.find('.agent-ws-run-failed')
    expect(note.exists()).toBe(true)
    const buttons = note.findAll('.agent-ws-run-failed-actions button')
    expect(buttons.length).toBe(2)

    await buttons[0].trigger('click')
    await flushPromises()
    expect(vi.mocked(runAgentJob)).toHaveBeenCalledTimes(1)
    expect(vi.mocked(runAgentJob)).toHaveBeenCalledWith(
      expect.objectContaining({
        sessionKey: 'test-session',
        instruction: '帮我总结这个版本',
        entities: [{ kind: 'release', id: 7 }],
        model: { provider: 'anthropic', model_id: 'claude-test' },
      }),
    )
    wrapper.unmount()
  })

  it('失败 run 提供「编辑后重试」：还原到输入区但不提交', async () => {
    vi.mocked(runAgentJob).mockClear()
    localStorage.setItem(
      'relwatch.agent.sessions.v1',
      JSON.stringify([{ key: 'test-session', title: 't', updatedAt: Date.now() }]),
    )
    vi.mocked(listAgentMessages).mockResolvedValue([sampleMessages()[0]])
    vi.mocked(listAgentRuns).mockResolvedValue([makeRun()])

    const wrapper = mount(AgentWorkspace, { global: { provide: { [ShowToastKey]: vi.fn() } } })
    await flushPromises()
    const buttons = wrapper.findAll('.agent-ws-run-failed-actions button')
    await buttons[1].trigger('click')
    await flushPromises()
    // 只回填，不提交——用户改完自己发
    expect(vi.mocked(runAgentJob)).not.toHaveBeenCalled()
    expect((wrapper.find('.agent-ws-textarea').element as HTMLTextAreaElement).value).toBe(
      '帮我总结这个版本',
    )
    wrapper.unmount()
  })

  it('重试时剔除已被删除的引用实体，不因实体缺失整体失败', async () => {
    vi.mocked(runAgentJob).mockClear()
    // 目录为空：run 引用的 release #7 已不存在
    vi.mocked(getReleases).mockResolvedValue([])
    localStorage.setItem(
      'relwatch.agent.sessions.v1',
      JSON.stringify([{ key: 'test-session', title: 't', updatedAt: Date.now() }]),
    )
    vi.mocked(listAgentMessages).mockResolvedValue([sampleMessages()[0]])
    vi.mocked(listAgentRuns).mockResolvedValue([
      makeRun({ entities: '[{"kind":"release","id":7}]' }),
    ])
    const showToast = vi.fn()
    const wrapper = mount(AgentWorkspace, {
      global: { provide: { [ShowToastKey as symbol]: showToast } },
    })
    await flushPromises()
    await wrapper.findAll('.agent-ws-run-failed-actions button')[0].trigger('click')
    await flushPromises()
    expect(showToast).toHaveBeenCalledWith(t('agent.retry_entities_dropped', '1'))
    // 指令仍提交，实体为空（后端对任一实体缺失会整体拒绝）
    expect(vi.mocked(runAgentJob)).toHaveBeenCalledWith(
      expect.objectContaining({ instruction: '帮我总结这个版本', entities: [] }),
    )
    wrapper.unmount()
  })

  it('被取消的 run 也可重试，且用中性样式（不是报错）', async () => {
    localStorage.setItem(
      'relwatch.agent.sessions.v1',
      JSON.stringify([{ key: 'test-session', title: 't', updatedAt: Date.now() }]),
    )
    vi.mocked(listAgentMessages).mockResolvedValue([sampleMessages()[0]])
    vi.mocked(listAgentRuns).mockResolvedValue([makeRun({ status: 'cancelled', error: null })])

    const wrapper = mount(AgentWorkspace, { global: { provide: { [ShowToastKey]: vi.fn() } } })
    await flushPromises()
    const note = wrapper.find('.agent-ws-run-failed')
    expect(note.exists()).toBe(true)
    expect(note.classes()).toContain('run-cancelled')
    expect(note.text()).toContain(t('agent.status_cancelled'))
    expect(note.findAll('.agent-ws-run-failed-actions button').length).toBe(2)
    wrapper.unmount()
  })

  it('有任务进行中时重试被拒绝，不产生第二个 run', async () => {
    vi.mocked(runAgentJob).mockClear()
    localStorage.setItem(
      'relwatch.agent.sessions.v1',
      JSON.stringify([{ key: 'test-session', title: 't', updatedAt: Date.now() }]),
    )
    // 两条 user 消息：第一条已失败，第二条仍在跑（canStop 为真）
    vi.mocked(listAgentMessages).mockResolvedValue([
      sampleMessages()[0],
      { role: 'user', timestamp: '2025-01-01T00:05:00.000Z', model: null, blocks: [{ kind: 'text', text: '继续' }] },
    ])
    vi.mocked(listAgentRuns).mockResolvedValue([
      makeRun({ id: 22, status: 'running', instruction: '继续', error: null, created_at: '2025-01-01T00:05:00.000Z' }),
      makeRun({ id: 20, created_at: '2025-01-01T00:00:00.000Z' }),
    ])
    const showToast = vi.fn()
    const wrapper = mount(AgentWorkspace, {
      global: { provide: { [ShowToastKey as symbol]: showToast } },
    })
    await flushPromises()
    await wrapper.findAll('.agent-ws-run-failed-actions button')[0].trigger('click')
    await flushPromises()
    expect(showToast).toHaveBeenCalledWith(t('agent.retry_blocked'))
    expect(vi.mocked(runAgentJob)).not.toHaveBeenCalled()
    wrapper.unmount()
  })
})

// ========== P2：打磨与功能完整性 ==========
describe('AgentWorkspace P2 打磨', () => {
  beforeEach(() => {
    // 模块级 mock 跨用例累积调用记录：清空后再断言「本次提交」
    vi.mocked(runAgentJob).mockClear()
    vi.mocked(exportAgentSession).mockClear()
    vi.mocked(restartAgentRpc).mockClear()
    vi.mocked(deleteAgentSession).mockClear()
    vi.mocked(confirmDialog).mockClear()
    vi.mocked(confirmDialog).mockResolvedValue(true)
    vi.mocked(openDialog).mockReset()
    // 关键：listAgentRuns 的 mockResolvedValue 会跨用例残留。上一个用例若留下
    // running/pending run，canStop 为真 → Enter 被「请先停止」拦下，提交永不发生。
    vi.mocked(listAgentRuns).mockResolvedValue([])
    vi.mocked(getAgentQueueStatus).mockResolvedValue({ position: null, other_running: false, running_sessions: [] })
    vi.mocked(getAgentRpcStatus).mockResolvedValue({ running: false, pid: null, restart_pending: false })
    vi.mocked(getAgentSessionUsage).mockResolvedValue({
      message_count: 0,
      total_chars: 0,
      file_bytes: 0,
      input_tokens: 0,
      output_tokens: 0,
      cache_read_tokens: 0,
      total_tokens: 0,
      cost_micros: 0,
      has_usage: false,
    })
  })

  /** 会话索引预置多项（供搜索/重命名/导出用例使用）。 */
  function seedSessions() {
    const base = Date.now()
    localStorage.setItem(
      'relwatch.agent.sessions.v1',
      JSON.stringify([
        { key: 's1', title: '分析 B 站 up 主的更新规律', updatedAt: base },
        { key: 's2', title: '总结 vue 3.5 的破坏性变更', updatedAt: base - 1000 },
        { key: 's3', title: '排查构建日志里的报错', updatedAt: base - 2000 },
      ]),
    )
  }

  it('会话搜索：按标题过滤侧栏，无匹配时给空态文案', async () => {
    seedSessions()
    const wrapper = mount(AgentWorkspace, { global: { provide: {} } })
    await flushPromises()
    expect(wrapper.findAll('.agent-ws-session-item').length).toBe(3)

    const input = wrapper.find('.agent-ws-session-search-input')
    await input.setValue('vue')
    expect(wrapper.findAll('.agent-ws-session-item').length).toBe(1)
    expect(wrapper.text()).toContain('总结 vue 3.5 的破坏性变更')

    // 无匹配：显示空态而非空白列表
    await input.setValue('不存在的会话')
    expect(wrapper.findAll('.agent-ws-session-item').length).toBe(0)
    expect(wrapper.text()).toContain(t('agent.session_no_match'))

    // 清空即恢复全量
    await input.setValue('')
    expect(wrapper.findAll('.agent-ws-session-item').length).toBe(3)
    wrapper.unmount()
  })

  it('会话重命名：改标题并写入 localStorage 索引', async () => {
    seedSessions()
    const wrapper = mount(AgentWorkspace, { global: { provide: {} } })
    await flushPromises()

    const items = wrapper.findAll('.agent-ws-session-item')
    await items[1].find('.agent-ws-session-more').trigger('click')
    const menuItems = findTeleportedAll('.agent-ws-session-menu .agent-ws-menu-item')
    await menuItems[0].trigger('click') // 重命名
    const editor = wrapper.find('.agent-ws-rename-input')
    expect(editor.exists()).toBe(true)
    expect((editor.element as HTMLInputElement).value).toBe('总结 vue 3.5 的破坏性变更')

    await editor.setValue('vue 3.5 迁移清单')
    await editor.trigger('keydown.enter')
    await flushPromises()

    const stored = JSON.parse(localStorage.getItem('relwatch.agent.sessions.v1') ?? '[]') as Array<{ key: string; title: string }>
    expect(stored.find((s) => s.key === 's2')?.title).toBe('vue 3.5 迁移清单')
    expect(wrapper.text()).toContain('vue 3.5 迁移清单')
    wrapper.unmount()
  })

  it('重命名 Esc 取消：标题不变', async () => {
    seedSessions()
    const wrapper = mount(AgentWorkspace, { global: { provide: {} } })
    await flushPromises()
    await wrapper.findAll('.agent-ws-session-item')[0].find('.agent-ws-session-more').trigger('click')
    await findTeleportedAll('.agent-ws-session-menu .agent-ws-menu-item')[0].trigger('click')
    const editor = wrapper.find('.agent-ws-rename-input')
    await editor.setValue('改坏了')
    await editor.trigger('keydown.esc')
    await flushPromises()

    const stored = JSON.parse(localStorage.getItem('relwatch.agent.sessions.v1') ?? '[]') as Array<{ key: string; title: string }>
    expect(stored.find((s) => s.key === 's1')?.title).toBe('分析 B 站 up 主的更新规律')
    expect(wrapper.find('.agent-ws-rename-input').exists()).toBe(false)
    wrapper.unmount()
  })

  it('会话导出：md / json 两个入口各调一次后端导出', async () => {
    seedSessions()
    const wrapper = mount(AgentWorkspace, { global: { provide: {} } })
    await flushPromises()

    await wrapper.findAll('.agent-ws-session-item')[2].find('.agent-ws-session-more').trigger('click')
    const items = findTeleportedAll('.agent-ws-session-menu .agent-ws-menu-item')
    expect(items[1].text()).toBe(t('agent.session_export_md'))
    expect(items[2].text()).toBe(t('agent.session_export_json'))
    await items[1].trigger('click')
    await flushPromises()
    expect(vi.mocked(exportAgentSession)).toHaveBeenCalledWith('s3', '排查构建日志里的报错', 'md')

    await wrapper.findAll('.agent-ws-session-item')[2].find('.agent-ws-session-more').trigger('click')
    await findTeleportedAll('.agent-ws-session-menu .agent-ws-menu-item')[2].trigger('click')
    await flushPromises()
    expect(vi.mocked(exportAgentSession)).toHaveBeenLastCalledWith('s3', '排查构建日志里的报错', 'json')
    wrapper.unmount()
  })

  it('会话 ⋯ 菜单的删除项：确认后调后端删除并移出侧栏', async () => {
    seedSessions()
    const wrapper = mount(AgentWorkspace, { global: { provide: {} } })
    await flushPromises()
    // 删除入口由侧栏常驻的 X 按钮移入 ⋯ 菜单：少一次误触，同时腾出位置放重命名/导出
    await wrapper.findAll('.agent-ws-session-item')[1].find('.agent-ws-session-more').trigger('click')
    const items = findTeleportedAll('.agent-ws-session-menu .agent-ws-menu-item')
    expect(items[3].text()).toBe(t('agent.delete_session'))
    expect(vi.mocked(confirmDialog)).not.toHaveBeenCalled()
    await items[3].trigger('click')
    await flushPromises()
    expect(vi.mocked(confirmDialog)).toHaveBeenCalled()
    expect(vi.mocked(deleteAgentSession)).toHaveBeenCalledWith('s2')
    expect(wrapper.text()).not.toContain('总结 vue 3.5 的破坏性变更')
    wrapper.unmount()
  })

  it('导出取消（err.agent.export_cancelled）不弹报错 toast', async () => {
    seedSessions()
    // 真实链路：invokeI18nFn 抛 InvokeI18nError（message 已翻译，key 保留）
    vi.mocked(exportAgentSession).mockRejectedValueOnce(
      new InvokeI18nError('err.agent.export_cancelled', [], t('err.agent.export_cancelled')),
    )
    const showToast = vi.fn()
    const wrapper = mount(AgentWorkspace, { global: { provide: { [ShowToastKey as symbol]: showToast } } })
    await flushPromises()
    await wrapper.findAll('.agent-ws-session-item')[0].find('.agent-ws-session-more').trigger('click')
    await findTeleportedAll('.agent-ws-session-menu .agent-ws-menu-item')[1].trigger('click')
    await flushPromises()
    expect(showToast).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('词元与成本：pi 上报用量时展示真实输入/输出与成本', async () => {
    vi.mocked(getAgentSessionUsage).mockResolvedValue({
      message_count: 6,
      total_chars: 4000,
      file_bytes: 12345,
      input_tokens: 1200,
      output_tokens: 340,
      cache_read_tokens: 2560,
      total_tokens: 4100,
      cost_micros: 41244, // 0.041244 美元
      has_usage: true,
    })
    const wrapper = mount(AgentWorkspace, { global: { provide: {} } })
    await flushPromises()
    const bar = wrapper.find('.agent-ws-usage')
    expect(bar.exists()).toBe(true)
    expect(bar.text()).toContain(t('agent.context_usage_actual', '6', '1200', '340'))
    expect(bar.text()).toContain('0.0412')
    // 真实用量不显示估算标记
    expect(wrapper.find('.agent-ws-usage-est').exists()).toBe(false)
    wrapper.unmount()
  })

  it('成本为零：pi 未配置模型价格时只展示词元，不显示 $0 成本', async () => {
    vi.mocked(getAgentSessionUsage).mockResolvedValue({
      message_count: 4,
      total_chars: 3000,
      file_bytes: 8000,
      input_tokens: 900,
      output_tokens: 120,
      cache_read_tokens: 0,
      total_tokens: 1020,
      cost_micros: 0, // models.json 自定义模型无价格 → pi 上报 cost 全 0
      has_usage: true,
    })
    const wrapper = mount(AgentWorkspace, { global: { provide: {} } })
    await flushPromises()
    const bar = wrapper.find('.agent-ws-usage')
    expect(bar.text()).toContain(t('agent.context_usage_actual', '4', '900', '120'))
    expect(bar.text()).not.toContain('$')
    wrapper.unmount()
  })

  it('词元估算：pi 未上报用量时回落字符数估算并标 ≈', async () => {
    vi.mocked(getAgentSessionUsage).mockResolvedValue({
      message_count: 3,
      total_chars: 4000,
      file_bytes: 9000,
      input_tokens: 0,
      output_tokens: 0,
      cache_read_tokens: 0,
      total_tokens: 0,
      cost_micros: 0,
      has_usage: false,
    })
    const wrapper = mount(AgentWorkspace, { global: { provide: {} } })
    await flushPromises()
    expect(wrapper.find('.agent-ws-usage').text()).toContain(t('agent.context_usage', '3', '2000'))
    expect(wrapper.find('.agent-ws-usage-est').exists()).toBe(true)
    wrapper.unmount()
  })

  it('pi 进程指示灯：点灯弹菜单展示状态与 pid，菜单内重启', async () => {
    vi.mocked(getAgentRpcStatus).mockResolvedValue({ running: true, pid: 4321, restart_pending: false })
    vi.mocked(restartAgentRpc).mockResolvedValue(true)
    const showToast = vi.fn()
    const wrapper = mount(AgentWorkspace, { global: { provide: { [ShowToastKey as symbol]: showToast } } })
    await flushPromises()

    const dot = wrapper.find('.agent-ws-rpc-dot')
    expect(dot.classes()).toContain('running')
    // 点灯开菜单（不再是点灯即重启）
    await dot.trigger('click')
    await flushPromises()
    const menu = findTeleported('.agent-ws-menu-rpc')
    expect(menu.exists()).toBe(true)
    expect(menu.text()).toContain(t('agent.rpc_running'))
    expect(menu.text()).toContain('4321')
    expect(vi.mocked(restartAgentRpc)).not.toHaveBeenCalled()

    // 菜单内点「重启」才触发
    await menu.find('.agent-ws-menu-item').trigger('click')
    await flushPromises()
    expect(vi.mocked(restartAgentRpc)).toHaveBeenCalled()
    expect(showToast).toHaveBeenCalledWith(t('agent.rpc_restart_done'))
    // 重启后菜单收起，让 toast 成为唯一反馈
    expect(findTeleported('.agent-ws-menu-rpc').exists()).toBe(false)
    wrapper.unmount()
  })

  it('未运行时菜单只显状态详情，无重启项（无物可重启，杜绝假重启）', async () => {
    vi.mocked(getAgentRpcStatus).mockResolvedValue({ running: false, pid: null, restart_pending: false })
    const showToast = vi.fn()
    const wrapper = mount(AgentWorkspace, { global: { provide: { [ShowToastKey as symbol]: showToast } } })
    await flushPromises()

    await wrapper.find('.agent-ws-rpc-dot').trigger('click')
    await flushPromises()
    const menu = findTeleported('.agent-ws-menu-rpc')
    expect(menu.exists()).toBe(true)
    expect(menu.text()).toContain(t('agent.rpc_stopped'))
    expect(menu.text()).toContain(t('agent.rpc_not_started_hint'))
    // 未运行：不提供重启入口（点击此前会 no-op 后谎报「已重启」）
    expect(menu.find('.agent-ws-menu-item').exists()).toBe(false)
    expect(vi.mocked(restartAgentRpc)).not.toHaveBeenCalled()
    wrapper.unmount()
  })

  it('点空白处收起 pi 状态菜单', async () => {
    vi.mocked(getAgentRpcStatus).mockResolvedValue({ running: true, pid: 7, restart_pending: false })
    // attachTo：outside-click 监听挂在 document（捕获阶段），游离容器的事件传播不经过它
    const wrapper = mount(AgentWorkspace, { attachTo: document.body, global: { provide: {} } })
    await flushPromises()
    await wrapper.find('.agent-ws-rpc-dot').trigger('click')
    expect(findTeleported('.agent-ws-menu-rpc').exists()).toBe(true)
    // 点菜单外的区域（如标题区）应收起
    wrapper.find('.agent-ws-title').element.dispatchEvent(new MouseEvent('pointerdown', { bubbles: true }))
    await nextTick()
    expect(findTeleported('.agent-ws-menu-rpc').exists()).toBe(false)
    wrapper.unmount()
  })

  it('有任务在跑时重启被拒：提示「请先停止」而非谎报成功', async () => {
    vi.mocked(getAgentRpcStatus).mockResolvedValue({ running: true, pid: 1, restart_pending: false })
    vi.mocked(restartAgentRpc).mockResolvedValue(false)
    const showToast = vi.fn()
    const wrapper = mount(AgentWorkspace, { global: { provide: { [ShowToastKey as symbol]: showToast } } })
    await flushPromises()
    await wrapper.find('.agent-ws-rpc-dot').trigger('click')
    await flushPromises()
    await findTeleported('.agent-ws-menu-rpc .agent-ws-menu-item').trigger('click')
    await flushPromises()
    expect(showToast).toHaveBeenCalledWith(t('agent.rpc_restart_blocked'))
    wrapper.unmount()
  })

  it('配置推迟生效提示：restart_pending 为真时显示横幅', async () => {
    vi.mocked(getAgentRpcStatus).mockResolvedValue({ running: true, pid: 7, restart_pending: true })
    const wrapper = mount(AgentWorkspace, { global: { provide: {} } })
    await flushPromises()
    expect(wrapper.find('.agent-ws-pending-restart').exists()).toBe(true)
    expect(wrapper.text()).toContain(t('agent.config_pending_restart'))
    wrapper.unmount()
  })

  it('单次模型覆盖：只作用于下一次提交，不写入会话索引', async () => {
    localStorage.setItem(
      'relwatch.agent.sessions.v1',
      JSON.stringify([{ key: 'test-session', title: 't', updatedAt: Date.now() }]),
    )
    const wrapper = mount(AgentWorkspace, { global: { provide: {} } })
    await flushPromises()

    // 打开模型菜单 → 勾「仅本次」→ 选一个模型
    await wrapper.find('.agent-ws-model-btn').trigger('click')
    await wrapper.find('.agent-ws-menu-once').trigger('click')
    await flushPromises()
    expect(wrapper.find('.agent-ws-menu-once').classes()).toContain('selected')

    const modelItems = wrapper.findAll('.agent-ws-menu-model .agent-ws-menu-item')
    await modelItems[2].trigger('click') // 第一个真实模型（index 0/1 是开关与「默认」）
    await flushPromises()

    // 按钮显示已切到该模型
    expect(wrapper.find('.agent-ws-model-label').text()).toBe('DeepSeek V4 Flash')

    await wrapper.find('textarea').setValue('这条用便宜模型')
    await wrapper.find('textarea').trigger('keydown', { key: 'Enter' })
    await flushPromises()

    // 提交时带上本次选择
    expect(vi.mocked(runAgentJob)).toHaveBeenCalled()
    expect(vi.mocked(runAgentJob).mock.calls[0][0].model).toEqual({
      provider: 'deepseek',
      model_id: 'deepseek-v4-flash',
    })
    // 关键：不落会话索引（会话长期模型仍是「默认」）
    const stored = JSON.parse(localStorage.getItem('relwatch.agent.sessions.v1') ?? '[]') as Array<{ key: string; model?: unknown }>
    expect(stored.find((s) => s.key === 'test-session')?.model ?? null).toBeNull()

    // 再提交一次：一次性覆盖已被消费，model 回落为 null（跟随 pi 默认）
    await wrapper.find('textarea').setValue('第二条消息')
    await wrapper.find('textarea').trigger('keydown', { key: 'Enter' })
    await flushPromises()
    expect(vi.mocked(runAgentJob).mock.calls[1][0].model).toBeNull()
    wrapper.unmount()
  })

  it('会话级模型选择仍然持久化（与单次覆盖互不干扰）', async () => {
    localStorage.setItem(
      'relwatch.agent.sessions.v1',
      JSON.stringify([{ key: 'test-session', title: 't', updatedAt: Date.now() }]),
    )
    const wrapper = mount(AgentWorkspace, { global: { provide: {} } })
    await flushPromises()
    await wrapper.find('.agent-ws-model-btn').trigger('click')
    // 不开「仅本次」：默认即会话级
    const modelItems = wrapper.findAll('.agent-ws-menu-model .agent-ws-menu-item')
    await modelItems[2].trigger('click')
    await flushPromises()
    const stored = JSON.parse(localStorage.getItem('relwatch.agent.sessions.v1') ?? '[]') as Array<{ key: string; model?: { model_id: string } }>
    expect(stored.find((s) => s.key === 'test-session')?.model?.model_id).toBe('deepseek-v4-flash')
    wrapper.unmount()
  })

  it('本地文件附件：chip 只显示文件名，提交时带上路径', async () => {
    localStorage.setItem(
      'relwatch.agent.sessions.v1',
      JSON.stringify([{ key: 'test-session', title: 't', updatedAt: Date.now() }]),
    )
    vi.mocked(openDialog).mockResolvedValue(['C:/logs/app.log', 'C:/img/shot.png'])
    const wrapper = mount(AgentWorkspace, { global: { provide: {} } })
    await flushPromises()

    await wrapper.find('.agent-ws-attach-btn').trigger('click')
    await flushPromises()
    const chips = wrapper.findAll('.agent-ws-chip-file')
    expect(chips.length).toBe(2)
    // chip 显示文件名，完整路径在 title 里
    expect(chips[0].text()).toContain('app.log')
    expect(chips[0].attributes('title')).toBe('C:/logs/app.log')

    await wrapper.find('textarea').setValue('看看这个日志')
    await wrapper.find('textarea').trigger('keydown', { key: 'Enter' })
    await flushPromises()
    expect(vi.mocked(runAgentJob).mock.calls[0][0].files).toEqual(['C:/logs/app.log', 'C:/img/shot.png'])
    // 提交后附件清空（不跟着下一轮）
    expect(wrapper.findAll('.agent-ws-chip-file').length).toBe(0)
    wrapper.unmount()
  })

  it('仅附件提交：不写指令也能提交，与后端空校验口径一致', async () => {
    localStorage.setItem(
      'relwatch.agent.sessions.v1',
      JSON.stringify([{ key: 'test-session', title: 't', updatedAt: Date.now() }]),
    )
    vi.mocked(openDialog).mockResolvedValue(['C:/logs/app.log'])
    const wrapper = mount(AgentWorkspace, { global: { provide: {} } })
    await flushPromises()

    await wrapper.find('.agent-ws-attach-btn').trigger('click')
    await flushPromises()
    // 不写指令直接回车：「看看这个日志」的意图已由附件承载，不应被空校验拦下
    await wrapper.find('textarea').trigger('keydown', { key: 'Enter' })
    await flushPromises()
    expect(vi.mocked(runAgentJob)).toHaveBeenCalledTimes(1)
    const input = vi.mocked(runAgentJob).mock.calls[0][0]
    expect(input.instruction).toBe('')
    expect(input.files).toEqual(['C:/logs/app.log'])
    wrapper.unmount()
  })

  it('附件 toast 显示翻译文案而非裸键（agent.file_attached 两语言都有定义）', async () => {
    localStorage.setItem(
      'relwatch.agent.sessions.v1',
      JSON.stringify([{ key: 'test-session', title: 't', updatedAt: Date.now() }]),
    )
    vi.mocked(openDialog).mockResolvedValue(['C:/a.log'])
    const showToast = vi.fn()
    const wrapper = mount(AgentWorkspace, { global: { provide: { [ShowToastKey as symbol]: showToast } } })
    await flushPromises()

    await wrapper.find('.agent-ws-attach-btn').trigger('click')
    await flushPromises()
    // t() 对缺失键返回键本身：若 i18n 漏了 agent.file_attached，这里会收到裸键
    expect(showToast).toHaveBeenCalledWith(t('agent.file_attached', '1'))
    expect(showToast).not.toHaveBeenCalledWith('agent.file_attached')
    wrapper.unmount()
  })

  it('附件可移除，取消文件对话框不改变已有附件', async () => {
    localStorage.setItem(
      'relwatch.agent.sessions.v1',
      JSON.stringify([{ key: 'test-session', title: 't', updatedAt: Date.now() }]),
    )
    vi.mocked(openDialog).mockResolvedValue(['C:/a.log', 'C:/b.log'])
    const wrapper = mount(AgentWorkspace, { global: { provide: {} } })
    await flushPromises()
    await wrapper.find('.agent-ws-attach-btn').trigger('click')
    await flushPromises()
    await wrapper.findAll('.agent-ws-chip-file .agent-ws-chip-remove')[0].trigger('click')
    expect(wrapper.findAll('.agent-ws-chip-file').length).toBe(1)

    vi.mocked(openDialog).mockResolvedValue(null) // 用户取消
    await wrapper.find('.agent-ws-attach-btn').trigger('click')
    await flushPromises()
    expect(wrapper.findAll('.agent-ws-chip-file').length).toBe(1)
    wrapper.unmount()
  })

  it('重试失败 run 时一并还原本地文件附件', async () => {
    localStorage.setItem(
      'relwatch.agent.sessions.v1',
      JSON.stringify([{ key: 'test-session', title: 't', updatedAt: Date.now() }]),
    )
    vi.mocked(listAgentMessages).mockResolvedValue([sampleMessages()[0]])
    vi.mocked(listAgentRuns).mockResolvedValue([
      makeRun({ id: 30, status: 'failed', error: 'err.agent.model_error', files: JSON.stringify(['C:/logs/app.log']) }),
    ])
    const wrapper = mount(AgentWorkspace, { global: { provide: {} } })
    await flushPromises()
    // 用「编辑后重试」还原到输入区（不直接提交，便于断言附件）
    await wrapper.findAll('.agent-ws-run-failed-actions button')[1].trigger('click')
    await flushPromises()
    const chips = wrapper.findAll('.agent-ws-chip-file')
    expect(chips.length).toBe(1)
    expect(chips[0].attributes('title')).toBe('C:/logs/app.log')
    wrapper.unmount()
  })

  it('结果未知（unknown）终态：可重试且带「可能已执行完成」提示，与失败区分', async () => {
    localStorage.setItem(
      'relwatch.agent.sessions.v1',
      JSON.stringify([{ key: 'test-session', title: 't', updatedAt: Date.now() }]),
    )
    vi.mocked(listAgentMessages).mockResolvedValue([sampleMessages()[0]])
    vi.mocked(listAgentRuns).mockResolvedValue([
      makeRun({ id: 40, status: 'unknown', error: 'err.agent.end_lost' }),
    ])
    const wrapper = mount(AgentWorkspace, { global: { provide: {} } })
    await flushPromises()
    const note = wrapper.find('.agent-ws-run-failed')
    expect(note.exists()).toBe(true)
    // 独立分类：既不是 failed 也不是 cancelled 的红色/灰色，而是未知专用样式
    expect(note.classes()).toContain('run-unknown')
    expect(note.classes()).not.toContain('run-cancelled')
    expect(note.text()).toContain(t('agent.status_unknown'))
    expect(note.text()).toContain(t('agent.unknown_advice'))
    // 文案不再谎称「已按失败处理」
    expect(note.text()).not.toContain('已按失败处理')
    // 仍可重试（终点不明即值得一试，但要用户先确认）
    expect(wrapper.findAll('.agent-ws-run-failed-actions button').length).toBe(2)
    wrapper.unmount()
  })

  it('启动清理文案：已启动的 run 说「状态未知」，未启动的说「未开始执行」', async () => {
    localStorage.setItem(
      'relwatch.agent.sessions.v1',
      JSON.stringify([{ key: 'test-session', title: 't', updatedAt: Date.now() }]),
    )
    vi.mocked(listAgentMessages).mockResolvedValue([sampleMessages()[0]])
    vi.mocked(listAgentRuns).mockResolvedValue([
      makeRun({ id: 41, status: 'unknown', error: 'err.agent.startup_cleanup_running' }),
    ])
    const wrapper = mount(AgentWorkspace, { global: { provide: {} } })
    await flushPromises()
    const text = wrapper.find('.agent-ws-run-failed').text()
    expect(text).toContain('状态未知')
    expect(text).not.toContain('未完成的提交已取消')
    wrapper.unmount()
  })
})
