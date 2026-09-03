// ── 会话管理（A 域：索引持久化 / 磁盘发现 / 重命名 / ⋯菜单 / 删除 / 清理 / 搜索 / 侧栏折叠）──
// 自 AgentWorkspace.vue 出仓。跨域动作不直接 import 其他域：
// - 删除活跃会话后的跨域清空经 onActiveDeleted 回调由编排层接线；
// - pickModel 的会话模型落库经 updateModel 暴露给编排层转调；
// - 侧栏运行状态点接收全局队列 queueActive 只读 ref（由聊天核心刷新）。
import { computed, nextTick, ref, type Ref } from 'vue'
import type { ComponentPublicInstance } from 'vue'
import { confirm } from '@tauri-apps/plugin-dialog'
import {
  deleteAgentSession,
  exportAgentSession,
  listAgentRuns,
  listAgentSessions,
  type AgentModelRef,
  type AgentQueueItem,
  type AgentRunSummary,
} from '../../api/agent'
import { t } from '../../i18n'
import { errorKey as errorKeyOf } from './agentChatUtils'
import { useAnchoredMenu } from './useAnchoredMenu'

// ── 会话元信息（localStorage 持久化，窗口重开可继续对话）──────────
export interface SessionMeta {
  key: string
  title: string
  updatedAt: number
  /** 该会话显式选择的模型（null/缺省 = 跟随 pi 当前/默认模型）。 */
  model?: AgentModelRef | null
  /** 由磁盘文件发现补入（localStorage 索引里没有）：侧栏标记为「已恢复」。 */
  recovered?: boolean
  /** 未提交的草稿会话（新建即登记；提交成功后清除）。 */
  draft?: boolean
}

const SESSIONS_STORAGE_KEY = 'relwatch.agent.sessions.v1'
// 会话侧栏折叠状态（默认折叠，聊天区全宽；localStorage 持久化）
const SIDEBAR_STORAGE_KEY = 'relwatch.agent.sidebar.v1'
// 会话 meta 持久化上限：每条约 150 字节，200 条仅 ~30KB，
// 远低于 localStorage 配额；超出部分由「清理旧会话」入口回收磁盘文件与 DB 记录
const SESSIONS_META_LIMIT = 200

export type SessionSwitchMode = 'switch' | 'new' | 'delete'

export function useAgentSessions(deps: {
  showToast: (msg: string) => void
  /** 全局队列（侧栏运行状态点数据源；由聊天核心 loadQueue 刷新） */
  queueActive: Ref<AgentQueueItem[]>
  /** 删除活跃会话后的跨域清空 + loadChat（编排层接线，替代原 handleDeleteSession 的 if 分支） */
  onActiveDeleted: () => Promise<void>
}) {
  const { showToast, queueActive } = deps

  const sidebarOpen = ref(localStorage.getItem(SIDEBAR_STORAGE_KEY) === '1')

  function toggleSidebar() {
    sidebarOpen.value = !sidebarOpen.value
    localStorage.setItem(SIDEBAR_STORAGE_KEY, sidebarOpen.value ? '1' : '0')
  }

  function loadSessions(): SessionMeta[] {
    try {
      const raw = localStorage.getItem(SESSIONS_STORAGE_KEY)
      const parsed = raw ? (JSON.parse(raw) as SessionMeta[]) : []
      return Array.isArray(parsed) ? parsed : []
    } catch {
      return []
    }
  }

  function persistSessions() {
    localStorage.setItem(SESSIONS_STORAGE_KEY, JSON.stringify(sessions.value.slice(0, SESSIONS_META_LIMIT)))
  }

  /** 磁盘发现：会话索引只存在于 localStorage（WebView2 缓存目录树，清缓存即失联），
   * 而会话文件在 Roaming 数据目录里完好无损 —— 文件即索引，标题从首条 user 消息重建。
   *
   * 合并策略：localStorage 为准（用户改过的标题/模型优先），磁盘上有而索引中没有的
   * 会话自动补入并标记为「恢复的会话」。用户点开后标记清除（已确认，不再是异常态）。 */
  async function discoverSessions(): Promise<number> {
    let found: Awaited<ReturnType<typeof listAgentSessions>>
    try {
      found = await listAgentSessions()
    } catch {
      return 0 // 发现失败不阻塞（localStorage 索引仍可用）
    }
    const known = new Set(sessions.value.map((s) => s.key))
    const recovered: SessionMeta[] = []
    for (const s of found) {
      if (known.has(s.session_key)) continue
      recovered.push({
        key: s.session_key,
        title: s.title.trim() || t('agent.session_untitled'),
        updatedAt: new Date(s.updated_at).getTime() || Date.now(),
        recovered: true,
      })
    }
    if (recovered.length === 0) return 0
    sessions.value = [...sessions.value, ...recovered].sort((a, b) => b.updatedAt - a.updatedAt)
    persistSessions()
    return recovered.length
  }

  const sessions = ref<SessionMeta[]>(loadSessions())
  // 「新建即登记」：无历史会话时立即登记一个草稿会话（标题「新会话」）——
  // 任何时刻 activeKey 都对应索引中的一项，未提交的会话不因重启/关面板丢失。
  // （此前「点新会话→拖实体→写半句话→关闭」的 key 永久丢失，见评审 1.2）
  if (sessions.value.length === 0) {
    sessions.value = [{ key: newSessionKey(), title: t('agent.session_new'), updatedAt: Date.now(), draft: true }]
  }
  persistSessions()

  // 激活会话：最近一个优先
  const activeKey = ref(sessions.value[0].key)
  const sessionTitle = computed(() => {
    const meta = sessions.value.find((s) => s.key === activeKey.value)
    return meta ? meta.title : t('agent.session_new')
  })

  function newSessionKey(): string {
    return crypto.randomUUID()
  }

  /** 清除会话的「已恢复」标记（用户打开过即视为已确认），变更时写回索引。 */
  function clearRecoveredFlag(key: string) {
    const idx = sessions.value.findIndex((s) => s.key === key && s.recovered)
    if (idx < 0) return
    sessions.value[idx] = { ...sessions.value[idx], recovered: false }
    persistSessions()
  }

  /** 切换激活会话的会话域部分（activeKey 赋值 + 恢复标记清除）；
   * 各域状态清空与 loadChat 由编排层按 mode 组合（§4.2 三处清空差异表）。 */
  function switchTo(key: string) {
    activeKey.value = key
    clearRecoveredFlag(key)
  }

  /** 新建即登记：立即写入索引并持久化、切换 activeKey，未提交的会话也可见、可恢复（评审 1.2）。 */
  function registerNew(): string {
    const key = newSessionKey()
    sessions.value.unshift({ key, title: t('agent.session_new'), updatedAt: Date.now(), draft: true })
    persistSessions()
    activeKey.value = key
    return key
  }

  /** 当前激活会话的 meta（startNewSession 的「已是空草稿不重复新建」判断用）。 */
  function currentMeta(): SessionMeta | undefined {
    return sessions.value.find((s) => s.key === activeKey.value)
  }

  /** 会话切换时会话域自己的清空：switch / new 收起重命名与 ⋯ 菜单；
   * delete 后切换不清（原实现即如此，§4.2 三处清空差异表）。 */
  function resetForSessionSwitch(mode: SessionSwitchMode) {
    if (mode === 'delete') return
    renamingKey.value = null
    openMenuKey.value = null
  }

  // ── 会话搜索（标题模糊匹配）──
  // 会话上限 200 条，标题又自动取首条指令前 40 字（往往高度相似），
  // 没有搜索就只能靠「清理旧会话」一刀切（评审「会话重命名 / 搜索」）。
  const sessionQuery = ref('')

  // ── 会话重命名 / 导出（侧栏 ⋯ 菜单）──
  /** 正在重命名的会话 key（null = 无）；同时只允许编辑一项。 */
  const renamingKey = ref<string | null>(null)
  const renameInput = ref('')

  function startRename(key: string) {
    renamingKey.value = key
    renameInput.value = sessions.value.find((s) => s.key === key)?.title ?? ''
    openMenuKey.value = null
    nextTick(() => renameEl.value?.focus())
  }

  function commitRename() {
    const key = renamingKey.value
    if (!key) return
    const title = renameInput.value.trim()
    const idx = sessions.value.findIndex((s) => s.key === key)
    if (idx >= 0 && title) {
      sessions.value[idx] = { ...sessions.value[idx], title }
      persistSessions()
    }
    renamingKey.value = null
  }

  function cancelRename() {
    renamingKey.value = null
  }

  /** 会话项 ⋯ 菜单展开的 key（null = 全部收起）。 */
  const openMenuKey = ref<string | null>(null)

  // 重命名输入框在 v-for 内部，字符串 ref 会被收集成数组；用函数 ref 精确绑定
  // 当前正在编辑的那一个（同一时刻最多一个），focus 才有确定目标。
  const renameEl = ref<HTMLInputElement | null>(null)
  function setRenameEl(el: Element | ComponentPublicInstance | null, key: string) {
    if (key === renamingKey.value) renameEl.value = (el as HTMLInputElement | null) ?? null
  }

  /** 会话 ⋯ 菜单：Teleport 到 body 后以触发按钮为锚 fixed 定位（useAnchoredMenu）。
   * 侧边栏宽仅 140px 且 overflow:hidden，absolute 定位的菜单超宽部分会被裁剪显示不全；
   * 脱离文档流盖在最上层（与 RPC 状态菜单同一策略，z-index 对齐 10002）。 */
  const sessionMoreEls = new Map<string, HTMLElement>()
  function setSessionMoreEl(el: Element | ComponentPublicInstance | null, key: string) {
    if (el) sessionMoreEls.set(key, el as HTMLElement)
    else sessionMoreEls.delete(key)
  }
  const sessionMenu = useAnchoredMenu({
    width: 148, // 与 .agent-ws-session-menu 的 min-width 保持一致
    align: 'right', // 右对齐按钮右缘防视口溢出
    isOpen: computed(() => openMenuKey.value !== null),
    onClose: () => (openMenuKey.value = null),
  })
  const sessionMenuStyle = sessionMenu.style

  function toggleSessionMenu(key: string) {
    openMenuKey.value = openMenuKey.value === key ? null : key
    // 重命名与菜单互斥：同时开着会互相遮挡（菜单浮层盖住输入框）
    if (openMenuKey.value) {
      renamingKey.value = null
      // 以 ⋯ 按钮为锚往下弹
      sessionMenu.place(sessionMoreEls.get(key) ?? null)
    }
  }

  /** 会话列表滚动时收起 ⋯ 菜单：菜单 fixed 定位不随列表滚动，继续悬浮会与锚点按钮错位。 */
  function onSessionListScroll() {
    if (openMenuKey.value) openMenuKey.value = null
  }

  /** 删除入口：先取 key 再关菜单。菜单浮层移到 Teleport 后模板无法像原来那样
   * 「先置 null 再传循环变量」，这里保证取到的 key 在关闭菜单前仍有效。 */
  function handleDeleteFromMenu() {
    const key = openMenuKey.value
    openMenuKey.value = null
    if (key) void deleteSession(key, deps.onActiveDeleted)
  }

  /** 导出会话：后端弹保存对话框并写成 md / json，返回实际路径。 */
  async function handleExportSession(key: string, format: 'md' | 'json') {
    openMenuKey.value = null
    const title = sessions.value.find((s) => s.key === key)?.title ?? t('agent.session_untitled')
    try {
      const path = await exportAgentSession(key, title, format)
      showToast(t('agent.export_done', path))
    } catch (e) {
      // 用户取消保存对话框也走 Err 分支（err.agent.export_cancelled）——不算失败，不弹报错。
      // 必须按错误 **key** 判断：错误经 invokeI18nFn 翻译后 message 已是本地化文案，
      // 用 String(e) 比对 key 会永远不相等（取消导出会被误报成失败）。
      const errKey = errorKeyOf(e)
      if (errKey !== 'err.agent.export_cancelled') showToast(String(e))
    }
  }

  /** pickModel 的会话级落库（编排层经 onPersistModel 转调；模型域不 import 会话域）。 */
  function updateModel(key: string, model: AgentModelRef | null) {
    const now = Date.now()
    const idx = sessions.value.findIndex((s) => s.key === key)
    if (idx >= 0) {
      sessions.value[idx] = { ...sessions.value[idx], updatedAt: now, model }
    } else {
      sessions.value.unshift({ key, title: sessionTitle.value, model, updatedAt: now })
    }
    persistSessions()
  }

  /** 提交成功后的会话登记固化（标题取首次指令前 40 字 / 固化本次模型 / 清 draft 标记）。
   *  新建即登记后 key 恒在索引中；draft 清除 = 已提交，不再是「新会话」。 */
  function persistSessionMeta(key: string, title: string, model: AgentModelRef | null) {
    const now = Date.now()
    const idx = sessions.value.findIndex((s) => s.key === key)
    if (idx >= 0) {
      sessions.value[idx] = { ...sessions.value[idx], title, updatedAt: now, model, draft: false }
    } else {
      sessions.value.unshift({ key, title, model, updatedAt: now, draft: false })
    }
    persistSessions()
  }

  /** 删除会话（确认对话框 + 后端删除 + 索引维护）。删除的是活跃会话时先把
   *  activeKey 切到剩余第一个（空则登记新草稿），再经 onActiveDeleted 回调让
   *  编排层做跨域清空与 loadChat（原 handleDeleteSession 的 if (key === activeKey) 分支）。 */
  async function deleteSession(key: string, onActiveDeleted: () => Promise<void>) {
    // 检查该会话是否有活跃 run（pending/running）：删除 = 移除会话文件 + 全部 run 记录，
    // 若正在运行，pi 进程会继续烧 token 直到自然结束或超时，产出写入已删除记录后静默丢弃。
    // 用户直觉是「删除=停止」，因此先提示「将同时停止」，后端 delete_agent_session 统一
    // 先取消活跃 run 再删除。
    let activeRunForSession: AgentRunSummary | undefined
    try {
      const sessionRuns = await listAgentRuns(key, 50)
      activeRunForSession = sessionRuns.find((r) => r.status === 'pending' || r.status === 'running')
    } catch {
      // 查询失败不阻塞删除（按无活跃 run 处理）
    }
    const confirmed = await confirm(
      activeRunForSession ? t('agent.delete_session_running_confirm') : t('agent.delete_session_confirm'),
      {
        title: t('agent.delete_session'),
        kind: 'warning',
      },
    )
    if (!confirmed) return
    try {
      // 后端统一处理：先取消活跃 run（若有），再删除会话记录
      await deleteAgentSession(key)
      const idx = sessions.value.findIndex((s) => s.key === key)
      if (idx >= 0) sessions.value.splice(idx, 1)
      persistSessions()
      if (key === activeKey.value) {
        // 全部会话删除后：立即登记一个新草稿会话（activeKey 恒对应索引中的一项）
        if (sessions.value.length === 0) {
          sessions.value = [{ key: newSessionKey(), title: t('agent.session_new'), updatedAt: Date.now(), draft: true }]
          persistSessions()
        }
        activeKey.value = sessions.value[0].key
        await onActiveDeleted()
      }
      showToast(t('agent.session_deleted'))
    } catch (e) {
      showToast(String(e))
    }
  }

  // 一键清理：删除除当前会话外的全部历史会话（文件 + DB 记录），带确认
  async function handleClearSessions() {
    const targets = sessions.value.filter((s) => s.key !== activeKey.value)
    if (targets.length === 0) return
    // 检查目标会话中是否有活跃 run：清理同样会删除运行记录，正在跑的 run 会继续烧 token
    // 直到自然结束或超时，产出写入已删除记录后静默丢弃——先提示并同时停止。
    let runningCount = 0
    try {
      for (const s of targets) {
        const sessionRuns = await listAgentRuns(s.key, 50)
        const active = sessionRuns.find((r) => r.status === 'pending' || r.status === 'running')
        if (active) {
          runningCount++
        }
      }
    } catch {
      // 查询失败不阻塞清理（按无活跃 run 处理）
    }
    const confirmed = await confirm(
      runningCount > 0 ? t('agent.clear_sessions_running_confirm', String(runningCount)) : t('agent.clear_sessions_confirm'),
      {
        title: t('agent.session_clear'),
        kind: 'warning',
      },
    )
    if (!confirmed) return
    let failed = 0
    // 后端 delete_agent_session 统一处理：先取消活跃 run（若有），再删除会话记录
    for (const s of targets) {
      try {
        await deleteAgentSession(s.key)
      } catch {
        failed++
      }
    }
    sessions.value = sessions.value.filter((s) => s.key === activeKey.value)
    persistSessions()
    if (failed > 0) showToast(t('agent.clear_sessions_partial', String(failed)))
    else showToast(t('agent.sessions_cleared', String(targets.length - failed)))
  }

  // ── 侧栏渲染 ──
  /** 某会话的运行状态点：running（执行中）优先，否则取队列最前的 pending。 */
  function sessionRunState(key: string): { status: string; position: number } | null {
    const items = queueActive.value.filter((i) => i.session_key === key)
    if (items.length === 0) return null
    const running = items.find((i) => i.status === 'running')
    if (running) return { status: 'running', position: running.position }
    return { status: 'pending', position: items[0].position }
  }

  /** 侧栏渲染源：sessions 预附运行状态（每项只算一次，避免模板内重复调用 sessionRunState）。 */
  const sessionsWithState = computed(() =>
    sessions.value.map((s) => ({ ...s, state: sessionRunState(s.key) })),
  )

  /** 侧栏实际渲染列表：附状态后再按搜索词过滤标题。 */
  const visibleSessions = computed(() => {
    const q = sessionQuery.value.trim().toLowerCase()
    const list = sessionsWithState.value
    if (!q) return list
    return list.filter((s) => s.title.toLowerCase().includes(q))
  })

  function sessionTitleOf(key: string): string {
    return sessions.value.find((s) => s.key === key)?.title || t('agent.session_untitled')
  }

  return {
    sessions,
    activeKey,
    sessionTitle,
    sessionQuery,
    sidebarOpen,
    toggleSidebar,
    discoverSessions,
    switchTo,
    registerNew,
    currentMeta,
    resetForSessionSwitch,
    renamingKey,
    renameInput,
    startRename,
    commitRename,
    cancelRename,
    setRenameEl,
    openMenuKey,
    setSessionMoreEl,
    sessionMenuStyle,
    toggleSessionMenu,
    onSessionListScroll,
    handleDeleteFromMenu,
    handleExportSession,
    updateModel,
    persistSessionMeta,
    deleteSession,
    handleClearSessions,
    sessionsWithState,
    visibleSessions,
    sessionTitleOf,
  }
}
