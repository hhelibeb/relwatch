import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { defineComponent, ref } from 'vue'
import { t } from '../i18n'
import { useAgentSessions, type SessionMeta } from '../components/agent/useAgentSessions'
import {
  deleteAgentSession,
  exportAgentSession,
  listAgentRuns,
  listAgentSessions,
  type AgentQueueItem,
} from '../api/agent'
import { confirm } from '@tauri-apps/plugin-dialog'

vi.mock('../api/agent', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/agent')>()
  return {
    ...actual,
    listAgentRuns: vi.fn().mockResolvedValue([]),
    listAgentSessions: vi.fn().mockResolvedValue([]),
    deleteAgentSession: vi.fn().mockResolvedValue(undefined),
    exportAgentSession: vi.fn().mockResolvedValue('C:/tmp/export.md'),
  }
})
vi.mock('@tauri-apps/plugin-dialog', () => ({
  confirm: vi.fn().mockResolvedValue(true),
  open: vi.fn(),
}))

const STORAGE_KEY = 'relwatch.agent.sessions.v1'

function seedStorage(items: Partial<SessionMeta>[]) {
  localStorage.setItem(STORAGE_KEY, JSON.stringify(items))
}

function readStorage(): SessionMeta[] {
  return JSON.parse(localStorage.getItem(STORAGE_KEY) ?? '[]') as SessionMeta[]
}

// 在宿主组件 setup 内调用 composable（内部 useAnchoredMenu 的 watch/onUnmounted 需要组件实例）
const wrappers: { unmount: () => void }[] = []
function setup(queue: AgentQueueItem[] = []) {
  const showToast = vi.fn()
  const onActiveDeleted = vi.fn().mockResolvedValue(undefined)
  let api!: ReturnType<typeof useAgentSessions>
  const wrapper = mount(
    defineComponent({
      setup() {
        api = useAgentSessions({
          showToast,
          queueActive: ref<AgentQueueItem[]>(queue),
          onActiveDeleted,
        })
        return {}
      },
      template: '<div/>',
    }),
    { attachTo: document.body },
  )
  wrappers.push(wrapper)
  return { api, showToast, onActiveDeleted }
}

beforeEach(() => {
  localStorage.clear()
  vi.clearAllMocks()
})
afterEach(() => {
  while (wrappers.length) wrappers.pop()?.unmount()
})

describe('useAgentSessions 索引与持久化', () => {
  it('索引为空时「新建即登记」一个草稿会话（activeKey 恒对应索引中的一项）', () => {
    const { api } = setup()
    expect(api.sessions.value.length).toBe(1)
    expect(api.sessions.value[0].draft).toBe(true)
    expect(api.activeKey.value).toBe(api.sessions.value[0].key)
    expect(readStorage().length).toBe(1) // 立即持久化
  })

  it('从 localStorage 恢复索引，激活最近一个', () => {
    seedStorage([
      { key: 's1', title: '最近', updatedAt: 200 },
      { key: 's2', title: '更早', updatedAt: 100 },
    ])
    const { api } = setup()
    expect(api.sessions.value.length).toBe(2)
    expect(api.activeKey.value).toBe('s1')
    expect(api.sessionTitle.value).toBe('最近')
  })

  it('switchTo：切换 activeKey 并清除「已恢复」标记、写回索引', () => {
    seedStorage([
      { key: 's1', title: 'a', updatedAt: 200 },
      { key: 's2', title: 'b', updatedAt: 100, recovered: true },
    ])
    const { api } = setup()
    api.switchTo('s2')
    expect(api.activeKey.value).toBe('s2')
    expect(readStorage().find((s) => s.key === 's2')?.recovered).toBe(false)
  })

  it('registerNew：立即写入索引、切 activeKey、登记为草稿', () => {
    seedStorage([{ key: 's1', title: 'a', updatedAt: 200 }])
    const { api } = setup()
    const key = api.registerNew()
    expect(api.activeKey.value).toBe(key)
    expect(readStorage()[0]).toMatchObject({ key, draft: true })
  })

  it('resetForSessionSwitch：switch/new 收起重命名与 ⋯ 菜单；delete 不清（§4.2 差异表）', () => {
    const { api } = setup()
    api.renamingKey.value = 's1'
    api.openMenuKey.value = 's1'
    api.resetForSessionSwitch('delete')
    expect(api.renamingKey.value).toBe('s1')
    expect(api.openMenuKey.value).toBe('s1')
    api.resetForSessionSwitch('switch')
    expect(api.renamingKey.value).toBeNull()
    expect(api.openMenuKey.value).toBeNull()
    api.renamingKey.value = 's1'
    api.resetForSessionSwitch('new')
    expect(api.renamingKey.value).toBeNull()
  })
})

describe('useAgentSessions 模型落库与提交登记', () => {
  it('updateModel：写会话 meta.model 并持久化；索引缺失时补登记', () => {
    seedStorage([{ key: 's1', title: 'a', updatedAt: 1 }])
    const { api } = setup()
    const model = { provider: 'anthropic', model_id: 'claude-sonnet-4' }
    api.updateModel('s1', model)
    const stored = readStorage().find((s) => s.key === 's1')
    expect(stored?.model).toEqual(model)

    api.updateModel('fresh-key', null)
    expect(readStorage().find((s) => s.key === 'fresh-key')?.key).toBe('fresh-key')
  })

  it('persistSessionMeta：固化标题/模型并清除 draft；索引缺失时补登记', () => {
    seedStorage([{ key: 's1', title: '旧标题', updatedAt: 1, draft: true }])
    const { api } = setup()
    api.persistSessionMeta('s1', '新标题', null)
    const stored = readStorage().find((s) => s.key === 's1')
    expect(stored).toMatchObject({ title: '新标题', draft: false })
  })
})

describe('useAgentSessions 重命名 / 导出 / 搜索 / 侧栏', () => {
  it('重命名：startRename 预填标题，commitRename 写入索引，cancelRename 不变', async () => {
    seedStorage([{ key: 's1', title: '原标题', updatedAt: 1 }])
    const { api } = setup()
    api.startRename('s1')
    expect(api.renamingKey.value).toBe('s1')
    expect(api.renameInput.value).toBe('原标题')
    // 打开菜单与重命名互斥
    api.openMenuKey.value = 's1'
    api.startRename('s1')
    expect(api.openMenuKey.value).toBeNull()

    api.renameInput.value = '  新标题  '
    api.commitRename()
    expect(readStorage().find((s) => s.key === 's1')?.title).toBe('新标题')
    expect(api.renamingKey.value).toBeNull()

    api.startRename('s1')
    api.renameInput.value = '改坏了'
    api.cancelRename()
    expect(readStorage().find((s) => s.key === 's1')?.title).toBe('新标题')
  })

  it('handleExportSession：用会话标题调后端导出；取消（err.agent.export_cancelled）不弹报错', async () => {
    seedStorage([{ key: 's1', title: '我的会话', updatedAt: 1 }])
    const { api, showToast } = setup()
    await api.handleExportSession('s1', 'md')
    expect(exportAgentSession).toHaveBeenCalledWith('s1', '我的会话', 'md')
    expect(showToast).toHaveBeenCalledWith(t('agent.export_done', 'C:/tmp/export.md'))

    const { InvokeI18nError } = await import('../api/client')
    vi.mocked(exportAgentSession).mockRejectedValueOnce(
      new InvokeI18nError('err.agent.export_cancelled', [], t('err.agent.export_cancelled')),
    )
    await api.handleExportSession('s1', 'json')
    expect(showToast).toHaveBeenCalledTimes(1) // 未新增报错 toast
  })

  it('visibleSessions：按搜索词过滤标题，无匹配时列表为空（空态文案由模板渲染）', () => {
    seedStorage([
      { key: 's1', title: '分析 B 站 up 主', updatedAt: 300 },
      { key: 's2', title: '总结 vue 3.5', updatedAt: 200 },
      { key: 's3', title: '排查构建报错', updatedAt: 100 },
    ])
    const { api } = setup()
    expect(api.visibleSessions.value.length).toBe(3)
    api.sessionQuery.value = 'VUE'
    expect(api.visibleSessions.value.map((s) => s.key)).toEqual(['s2'])
    api.sessionQuery.value = '不存在的'
    expect(api.visibleSessions.value.length).toBe(0)
    api.sessionQuery.value = ''
    expect(api.visibleSessions.value.length).toBe(3)
  })

  it('sessionsWithState：运行状态点 running 优先，否则取队列最前 pending', () => {
    seedStorage([
      { key: 's1', title: 'a', updatedAt: 200 },
      { key: 's2', title: 'b', updatedAt: 100 },
    ])
    const queue: AgentQueueItem[] = [
      { run_id: 1, session_key: 's2', status: 'pending', position: 2 },
      { run_id: 2, session_key: 's1', status: 'running', position: 1 },
    ]
    const { api } = setup(queue)
    const withState = api.sessionsWithState.value
    expect(withState.find((s) => s.key === 's1')?.state).toEqual({ status: 'running', position: 1 })
    expect(withState.find((s) => s.key === 's2')?.state).toEqual({ status: 'pending', position: 2 })
  })

  it('toggleSidebar：切换折叠状态并持久化', () => {
    const { api } = setup()
    const before = api.sidebarOpen.value
    api.toggleSidebar()
    expect(api.sidebarOpen.value).toBe(!before)
    expect(localStorage.getItem('relwatch.agent.sidebar.v1')).toBe(api.sidebarOpen.value ? '1' : '0')
  })
})

describe('useAgentSessions 删除 / 清理 / 磁盘发现', () => {
  it('deleteSession：删除非活跃会话不触发跨域联动；确认取消则不动', async () => {
    seedStorage([
      { key: 's1', title: 'a', updatedAt: 200 },
      { key: 's2', title: 'b', updatedAt: 100 },
    ])
    const { api, onActiveDeleted, showToast } = setup()
    vi.mocked(confirm).mockResolvedValue(false)
    await api.deleteSession('s2', onActiveDeleted)
    expect(deleteAgentSession).not.toHaveBeenCalled()

    vi.mocked(confirm).mockResolvedValue(true)
    await api.deleteSession('s2', onActiveDeleted)
    expect(deleteAgentSession).toHaveBeenCalledWith('s2')
    expect(onActiveDeleted).not.toHaveBeenCalled()
    expect(showToast).toHaveBeenCalledWith(t('agent.session_deleted'))
    expect(readStorage().map((s) => s.key)).toEqual(['s1'])
  })

  it('deleteSession：删除活跃会话 → activeKey 切到剩余第一个 + onActiveDeleted 联动 + toast', async () => {
    seedStorage([
      { key: 's1', title: 'a', updatedAt: 200 },
      { key: 's2', title: 'b', updatedAt: 100 },
    ])
    const { api, onActiveDeleted } = setup()
    vi.mocked(confirm).mockResolvedValue(true)
    await api.deleteSession('s1', onActiveDeleted)
    expect(api.activeKey.value).toBe('s2')
    expect(onActiveDeleted).toHaveBeenCalledTimes(1)
    expect(readStorage().map((s) => s.key)).toEqual(['s2'])
  })

  it('deleteSession：删光后立即登记新草稿（activeKey 恒对应索引中的一项）', async () => {
    seedStorage([{ key: 's1', title: 'a', updatedAt: 1 }])
    const { api, onActiveDeleted } = setup()
    vi.mocked(confirm).mockResolvedValue(true)
    await api.deleteSession('s1', onActiveDeleted)
    expect(api.sessions.value.length).toBe(1)
    expect(api.sessions.value[0].draft).toBe(true)
    expect(api.activeKey.value).toBe(api.sessions.value[0].key)
  })

  it('deleteSession：后端删除失败 → toast 错误文本，索引不变', async () => {
    seedStorage([{ key: 's1', title: 'a', updatedAt: 1 }])
    const { api, showToast } = setup()
    vi.mocked(confirm).mockResolvedValue(true)
    vi.mocked(deleteAgentSession).mockRejectedValueOnce(new Error('db locked'))
    await api.deleteSession('s1', vi.fn().mockResolvedValue(undefined))
    expect(showToast).toHaveBeenCalledWith('Error: db locked')
    expect(readStorage().length).toBe(1)
  })

  it('deleteSession：有活跃 run 时确认文案改为「将同时停止」', async () => {
    seedStorage([{ key: 's1', title: 'a', updatedAt: 1 }])
    const { api } = setup()
    vi.mocked(confirm).mockResolvedValue(false)
    vi.mocked(listAgentRuns).mockResolvedValue([
      {
        id: 1,
        session_key: 's1',
        skill_path: null,
        entities: '[]',
        instruction: 'x',
        model: null,
        session_path: null,
        status: 'running',
        exit_code: null,
        error: null,
        started_at: null,
        finished_at: null,
        created_at: '2025-01-01T00:00:00.000Z',
        files: null,
      },
    ])
    await api.deleteSession('s1', vi.fn().mockResolvedValue(undefined))
    expect(confirm).toHaveBeenCalledWith(t('agent.delete_session_running_confirm'), expect.anything())
  })

  it('handleClearSessions：保留当前会话，删除其余并汇总 toast', async () => {
    seedStorage([
      { key: 's1', title: 'a', updatedAt: 200 },
      { key: 's2', title: 'b', updatedAt: 100 },
      { key: 's3', title: 'c', updatedAt: 50 },
    ])
    const { api, showToast } = setup()
    vi.mocked(confirm).mockResolvedValue(true)
    await api.handleClearSessions()
    expect(readStorage().map((s) => s.key)).toEqual(['s1'])
    expect(showToast).toHaveBeenCalledWith(t('agent.sessions_cleared', '2'))
  })

  it('discoverSessions：索引缺失的磁盘会话补入并标记「已恢复」、按 updatedAt 倒序', async () => {
    seedStorage([{ key: 'kept', title: '我改过的标题', updatedAt: 100 }])
    vi.mocked(listAgentSessions).mockResolvedValue([
      {
        session_key: 'lost',
        title: '上周分析',
        session_path: 'C:/data/ws-lost.jsonl',
        updated_at: '2026-08-20T11:43:02.000Z',
        last_status: 'success',
        run_count: 3,
      },
      {
        session_key: 'kept',
        title: '从文件重建的标题',
        session_path: 'C:/data/ws-kept.jsonl',
        updated_at: '2026-09-01T00:00:00.000Z',
        last_status: 'success',
        run_count: 1,
      },
    ])
    const { api } = setup()
    const recovered = await api.discoverSessions()
    expect(recovered).toBe(1)
    expect(api.sessions.value[0].key).toBe('lost')
    expect(api.sessions.value[0].recovered).toBe(true)
    // localStorage 为准：已在索引中的会话不覆盖用户侧标题
    expect(readStorage().find((s) => s.key === 'kept')?.title).toBe('我改过的标题')
  })

  it('discoverSessions：失败不阻塞（返回 0，索引仍可用）', async () => {
    seedStorage([{ key: 's1', title: 'a', updatedAt: 1 }])
    vi.mocked(listAgentSessions).mockRejectedValue(new Error('boom'))
    const { api } = setup()
    expect(await api.discoverSessions()).toBe(0)
  })
})
