// ── pi 常驻进程健康（E 域：指示灯 + 状态菜单 + 重启 + 推迟生效提示）──
// pi 是常驻 RPC 子进程，挂了/卡了 UI 此前毫无感知：提交失败时用户分不清是
// 配置写错还是进程挂了，只能盲改设置重试。指示灯把「进程在不在」变成可见状态，
// 重启入口给出一条不依赖排障知识的自救路径。
//
// 交互形态：点状态灯弹出菜单（状态详情 + 重启项），而非「点灯即重启」——
// 灯的语义是状态展示，重启是低频排障操作，混在一个 8px 热区里既看不懂也易误触
// （未运行时点击更无从「重启」，此前会静默 no-op 并 toast 谎报已重启）。
// 重启项仅在运行中渲染：未运行时首次提交会自动拉起，无需也没有可重启的对象。
import { computed, ref } from 'vue'
import { getAgentRpcStatus, restartAgentRpc, type AgentRpcStatus } from '../../api/agent'
import { t } from '../../i18n'
import { useAnchoredMenu } from './useAnchoredMenu'

export function useAgentRpc(deps: {
  showToast: (msg: string) => void
  /** 状态菜单打开时收起输入区各菜单（同屏叠开会互相遮挡，原 toggleRpcMenu 行为） */
  onMenuOpen?: () => void
}) {
  const { showToast } = deps

  const rpcStatus = ref<AgentRpcStatus | null>(null)
  const rpcRestarting = ref(false)
  const rpcMenuOpen = ref(false)
  // 菜单以灯为锚 fixed 定位（useAnchoredMenu）：header 在窗口顶缘，
  // absolute 上弹会被 Windows 窗口裁剪、下弹会被后续层叠内容遮挡，fixed + 高 z-index 才能稳定盖在最上层
  const rpcDotEl = ref<HTMLElement | null>(null)
  const rpcMenu = useAnchoredMenu({
    width: 216, // min-width 208 + 边缘余量
    align: 'left', // 以灯左缘起弹；面板贴窗口右缘时自动钳制回视口内（菜单右缘贴右缘）
    isOpen: rpcMenuOpen,
    onClose: () => (rpcMenuOpen.value = false),
  })
  const rpcMenuStyle = rpcMenu.style

  async function loadRpcStatus() {
    try {
      rpcStatus.value = await getAgentRpcStatus()
    } catch {
      rpcStatus.value = null
    }
  }

  /** 点状态灯开/关菜单。打开时刷新一次：轮询只在 run 进行期间跑，
   *  空闲期 pid / 存活可能已变化，菜单里的详情要拿新鲜的。 */
  function toggleRpcMenu() {
    rpcMenuOpen.value = !rpcMenuOpen.value
    if (rpcMenuOpen.value) {
      // 以灯为锚往下弹
      rpcMenu.place(rpcDotEl.value)
      void loadRpcStatus()
      // 与其他弹出层互斥：输入区菜单与 rpc 菜单同屏叠开会被此遮彼挡
      deps.onMenuOpen?.()
    }
  }

  /** 配置推迟生效提示（评审 3.8）：改了 pi 路径/模型/skill 后有 run 在跑，
   *  重启被推迟到当前任务结束——此前这段时间 UI 无任何提示，用户以为改了没生效。 */
  const rpcRestartPending = computed<boolean>(() => rpcStatus.value?.restart_pending === true)

  async function handleRestartRpc() {
    if (rpcRestarting.value) return
    rpcRestarting.value = true
    try {
      const restarted = await restartAgentRpc()
      // false = 有 run 正在执行，后端拒绝重启（kill 会中断生成、烧掉已有词元）
      showToast(restarted ? t('agent.rpc_restart_done') : t('agent.rpc_restart_blocked'))
      await loadRpcStatus()
    } catch (e) {
      showToast(String(e))
    } finally {
      rpcRestarting.value = false
      // 重启发出后菜单使命完成，收起让 toast 成为唯一反馈
      rpcMenuOpen.value = false
    }
  }

  return {
    rpcStatus,
    rpcRestarting,
    rpcMenuOpen,
    rpcDotEl,
    rpcMenuStyle,
    loadRpcStatus,
    toggleRpcMenu,
    rpcRestartPending,
    handleRestartRpc,
  }
}
