import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { t, setLocale } from '../i18n'
import type { AgentChatMessage } from '../api/agent'

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

// Agent API：默认空目录 / 空会话 / 空消息
vi.mock('../api/agent', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/agent')>()
  return {
    ...actual,
    getAgentConfig: vi.fn().mockResolvedValue({ enabled: true, pi_binary: null, pi_model: null, prompt_suffix: null, timeout_seconds: 300, skills: [] }),
    listAgentRuns: vi.fn().mockResolvedValue([]),
    listAgentMessages: vi.fn().mockResolvedValue([]),
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
import { listAgentMessages, listAgentRuns, runAgentJob, getAgentConfig } from '../api/agent'
import { listSources } from '../api/sources'
import { getReleases } from '../api/releases'
import type { AgentEntityRefSeed } from '../injection-keys'

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
  vi.mocked(listAgentMessages).mockResolvedValue(sampleMessages())
})

describe('AgentWorkspace 冒烟', () => {
  it('渲染标题、会话边栏、输入区', async () => {
    const wrapper = mount(AgentWorkspace, {
      global: { provide: { [Symbol.for('showToast') as never]: vi.fn() } },
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
      pi_binary: null,
      pi_model: null,
      prompt_suffix: null,
      timeout_seconds: 300,
      skills: [SKILL],
    })
    const wrapper = mount(AgentWorkspace, {
      global: { provide: { [Symbol.for('showToast') as never]: vi.fn() } },
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
      {
        id: 1,
        session_key: 'k',
        skill_path: SKILL,
        entities: '[]',
        instruction: '执行',
        session_path: null,
        status: 'success',
        exit_code: 0,
        stdout: null,
        stderr: null,
        error: null,
        started_at: '2025-01-01T00:00:00.000Z',
        finished_at: '2025-01-01T00:00:05.000Z',
        created_at: '2025-01-01T00:00:00.000Z',
      },
    ])
    const wrapper = mount(AgentWorkspace, {
      global: { provide: { [Symbol.for('showToast') as never]: vi.fn() } },
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
    // 流式：同轮内容以 text_delta 到达
    emitRpc(JSON.stringify({ type: 'message_update', assistantMessageEvent: { type: 'text_delta', delta: '总结：这是 v1.0.0 版本' } }))
    await flushPromises()
    const count = wrapper.text().split('总结：这是 v1.0.0 版本').length - 1
    expect(count).toBe(1) // 流式优先，全量未叠加 → 只出现一次
    // 终态：回落全量校准，仍不重复
    emitRpc(JSON.stringify({ type: 'agent_settled' }))
    await flushPromises()
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
      global: { provide: { [Symbol.for('showToast') as never]: vi.fn() } },
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

  it('user 消息整条被 <用户指令> 包裹时渲染剥离标签，标签在中间时保留', async () => {
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
      global: { provide: { [Symbol.for('showToast') as never]: vi.fn() } },
    })
    await flushPromises()
    const text = wrapper.text()
    // 整条包裹：标签剥离，仅内容可见
    expect(text).toContain('不用，有没有提到努比亚')
    expect(text).not.toContain('<用户指令>\n不用')
    // 标签位于消息中间：保留显示，完整上下文可见
    expect(text).toContain('<用户指令>')
    expect(text).toContain('中间指令')
    wrapper.unmount()
  })

  it('流式期间历史快照与本地回显同屏可见，终态回落全量校准', async () => {
    vi.mocked(runAgentJob).mockClear()
    localStorage.setItem(
      'relwatch.agent.sessions.v1',
      JSON.stringify([{ key: 'test-session', title: 't', updatedAt: Date.now() }]),
    )
    const wrapper = mount(AgentWorkspace, {
      global: { provide: { [Symbol.for('showToast') as never]: vi.fn() } },
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
    await flushPromises()
    // 历史（JSONL 全量）+ 本地回显 user + 流式内容同屏可见
    expect(wrapper.text()).toContain('帮我总结这个版本')
    expect(wrapper.text()).toContain('测试指令')
    expect(wrapper.text()).toContain('流式补充')
    // 终态：清流式，回落全量校准（JSONL 无流式内容）
    emitRpc(JSON.stringify({ type: 'agent_settled' }))
    await flushPromises()
    expect(wrapper.text()).not.toContain('流式补充')
    expect(wrapper.text()).toContain('帮我总结这个版本')
    wrapper.unmount()
    localStorage.removeItem('relwatch.agent.sessions.v1')
  })
})
