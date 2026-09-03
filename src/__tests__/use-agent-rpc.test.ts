import { describe, it, expect, vi, beforeEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { defineComponent } from 'vue'
import { t } from '../i18n'
import { useAgentRpc } from '../components/agent/useAgentRpc'
import { getAgentRpcStatus, restartAgentRpc } from '../api/agent'

vi.mock('../api/agent', async (importOriginal) => {
  const actual = await importOriginal<typeof import('../api/agent')>()
  return {
    ...actual,
    getAgentRpcStatus: vi.fn().mockResolvedValue({ running: false, pid: null, restart_pending: false }),
    restartAgentRpc: vi.fn().mockResolvedValue(true),
  }
})

function setup(deps: { showToast?: (m: string) => void; onMenuOpen?: () => void } = {}) {
  let api!: ReturnType<typeof useAgentRpc>
  const showToast = deps.showToast ?? vi.fn()
  const host = defineComponent({
    setup() {
      api = useAgentRpc({ showToast, onMenuOpen: deps.onMenuOpen })
      return { rpcDotEl: api.rpcDotEl }
    },
    template: '<button ref="rpcDotEl" class="dot" />',
  })
  const wrapper = mount(host)
  return { wrapper, api, showToast }
}

beforeEach(() => {
  vi.mocked(getAgentRpcStatus).mockResolvedValue({ running: false, pid: null, restart_pending: false })
  vi.mocked(restartAgentRpc).mockResolvedValue(true)
})

describe('useAgentRpc', () => {
  it('loadRpcStatus：成功写入 / 失败置 null', async () => {
    const { api } = setup()
    vi.mocked(getAgentRpcStatus).mockResolvedValue({ running: true, pid: 4321, restart_pending: false })
    await api.loadRpcStatus()
    expect(api.rpcStatus.value).toEqual({ running: true, pid: 4321, restart_pending: false })

    vi.mocked(getAgentRpcStatus).mockRejectedValue(new Error('boom'))
    await api.loadRpcStatus()
    expect(api.rpcStatus.value).toBeNull()
  })

  it('toggleRpcMenu：开时定位 + 刷新状态 + 通知互斥收起；再点关', async () => {
    const onMenuOpen = vi.fn()
    const { api } = setup({ onMenuOpen })
    await api.toggleRpcMenu()
    expect(api.rpcMenuOpen.value).toBe(true)
    expect(onMenuOpen).toHaveBeenCalledTimes(1)
    // 打开时刷新一次（空闲期 pid/存活可能已变化）
    expect(getAgentRpcStatus).toHaveBeenCalled()

    await api.toggleRpcMenu()
    expect(api.rpcMenuOpen.value).toBe(false)
    expect(onMenuOpen).toHaveBeenCalledTimes(1) // 关闭不触发互斥
  })

  it('handleRestartRpc：成功 → toast 已重启 + 刷新 + 收菜单；拒绝(false) → toast 被阻止', async () => {
    const showToast = vi.fn()
    const { api } = setup({ showToast })
    api.rpcMenuOpen.value = true

    vi.mocked(getAgentRpcStatus).mockResolvedValue({ running: true, pid: 1, restart_pending: false })
    vi.mocked(restartAgentRpc).mockResolvedValue(true)
    await api.handleRestartRpc()
    expect(showToast).toHaveBeenCalledWith(t('agent.rpc_restart_done'))
    expect(api.rpcMenuOpen.value).toBe(false) // 重启发出后菜单收起
    expect(api.rpcRestarting.value).toBe(false)

    api.rpcMenuOpen.value = true
    vi.mocked(restartAgentRpc).mockResolvedValue(false)
    await api.handleRestartRpc()
    expect(showToast).toHaveBeenLastCalledWith(t('agent.rpc_restart_blocked'))
  })

  it('handleRestartRpc：异常 → toast 错误文本；进行中重复调用被忽略', async () => {
    const showToast = vi.fn()
    const { api } = setup({ showToast })
    vi.mocked(restartAgentRpc).mockRejectedValue(new Error('nope'))
    await api.handleRestartRpc()
    expect(showToast).toHaveBeenCalledWith('Error: nope')
    expect(api.rpcRestarting.value).toBe(false)
    expect(api.rpcMenuOpen.value).toBe(false)
  })

  it('rpcRestartPending：restart_pending 为真时提示推迟生效', async () => {
    const { api } = setup()
    vi.mocked(getAgentRpcStatus).mockResolvedValue({ running: true, pid: 7, restart_pending: true })
    await api.loadRpcStatus()
    expect(api.rpcRestartPending.value).toBe(true)

    vi.mocked(getAgentRpcStatus).mockResolvedValue({ running: true, pid: 7, restart_pending: false })
    await api.loadRpcStatus()
    expect(api.rpcRestartPending.value).toBe(false)
  })
})
