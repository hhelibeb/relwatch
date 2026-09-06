// ── 引用与输入区（F 域：@skill / [[实体 菜单 / 附件 / chip 悬浮提示 / flash 反馈）──
// 自 AgentWorkspace.vue 出仓。指令草稿（instruction）与引用 chips（entities /
// skillPath / files）由本模块持有；会话切换时的清空经编排层调 resetForSessionSwitch。
// 跨域互斥（模型菜单 / rpc 菜单）不反向依赖：菜单显隐状态与选择动作暴露给编排层，
// 由编排层的键盘导航分发（K）与全局收起逻辑接线。
import { computed, nextTick, onUnmounted, ref, type Ref } from 'vue'
import { open } from '@tauri-apps/plugin-dialog'
import type { Source } from '../../api/sources'
import type { ReleaseInfo } from '../../api/releases'
import { t } from '../../i18n'
import { getSourceTypeDef } from '../../api/source-registry'
import { skillShortName } from '../../utils'
import type { AgentEntityRefSeed } from '../../injection-keys'
import type { SessionSwitchMode } from './useAgentSessions'

const SKILL_TRIGGER = /@([\w\-.\\/]*)$/
const ENTITY_TRIGGER = /\[\[([^\]]*)$/

// 引用变更的就地反馈：chip 短暂高亮 + 无障碍播报。
// 此前这里弹的是全局 Toast，而 Toast 是 fixed 右下角、正好压在发送/附件按钮上——
// 既挡视线又吞点击，鼠标停在按钮上还会触发它的悬浮暂停而永不消失。
// 拖入的视觉焦点本就在落点（输入区），反馈放回落点即可，无需再去右下角播报一次。
const FLASH_DURATION = 1200

export function useAgentComposer(deps: {
  showToast: (msg: string) => void
  /** 全局 skill 列表（loadCatalog 填充，编排层持有） */
  skills: Ref<string[]>
  /** 实体目录：[[ 菜单数据源 + chip 可读名映射 */
  sources: Ref<Source[]>
  releases: Ref<ReleaseInfo[]>
}) {
  const { showToast, skills, sources, releases } = deps

  // ── 输入区草稿 ──
  const instruction = ref('')
  const entities = ref<AgentEntityRefSeed[]>([])
  const skillPath = ref<string | null>(null)
  const textareaRef = ref<HTMLTextAreaElement | null>(null)

  // ── 本地文件附件（评审「本地文件/图片附件」）──
  // 应用内实体（监控源/版本）之外，真实任务常要看本地日志 / 截图。
  // 只传绝对路径、不读内容：内容由 pi 自己的工具按需读取（避免把大文件塞进上下文），
  // 路径走 prompt 的权威指令区，不进不可信外部数据区。
  const files = ref<string[]>([])

  async function handleAttachFiles() {
    try {
      const picked = await open({ multiple: true, directory: false, title: t('agent.attach_file') })
      if (!picked) return
      const list = Array.isArray(picked) ? picked : [picked]
      let added = 0
      for (const p of list) {
        if (!p || files.value.includes(p)) continue
        files.value.push(p)
        added++
      }
      if (added > 0) showToast(t('agent.file_attached', String(added)))
    } catch (e) {
      showToast(String(e))
    }
  }

  function removeFile(index: number) {
    files.value.splice(index, 1)
  }

  /** chip 上只显示文件名（完整路径放 title，悬浮可见）。 */
  function fileDisplayName(path: string): string {
    const name = path.replace(/\\/g, '/').split('/').filter(Boolean).pop()
    return name || path
  }

  // ── 引用菜单状态 ──
  const showSkillMenu = ref(false)
  const skillQuery = ref('')
  const showEntityMenu = ref(false)
  const entityQuery = ref('')
  const skillMenuIndex = ref(0)
  const entityMenuIndex = ref(0)

  // ── 事件桥/拖拽写入 chips 的统一入口 ──
  /** 返回是否真的新增（已存在则原样保留，供调用方决定反馈方式）。 */
  function addEntity(e: AgentEntityRefSeed): boolean {
    if (!entities.value.some((x) => x.kind === e.kind && x.id === e.id)) {
      entities.value.push(e)
      return true
    }
    return false
  }

  const flashKey = ref<string | null>(null)
  let flashTimer: ReturnType<typeof setTimeout> | null = null
  // 屏幕阅读器播报（视觉上不可见）：补回 Toast 原先承担的告知作用。
  // 注意别与上方流式消息集合 liveMessages（差一个 s，语义完全无关）混淆
  const attachAnnouncement = ref('')

  function flashEntity(e: AgentEntityRefSeed, added: boolean) {
    flashKey.value = `${e.kind}:${e.id}`
    // 重复拖入已有引用时 chip 同样高亮（告诉用户「在这儿、已经加过了」），
    // 但播报文案要区分，不能谎称「已加入」
    attachAnnouncement.value = added ? t('agent.attached') : t('agent.attached_exists')
    if (flashTimer) clearTimeout(flashTimer)
    flashTimer = setTimeout(() => {
      flashTimer = null
      flashKey.value = null
      attachAnnouncement.value = ''
    }, FLASH_DURATION)
  }

  /** 拖入/加入引用的统一收尾：高亮对应 chip，并把光标送到输入框末尾（拖完即可打字发送）。 */
  function afterAttach(e: AgentEntityRefSeed, added: boolean) {
    flashEntity(e, added)
    nextTick(() => focusAtEnd())
  }

  function removeEntity(index: number) {
    entities.value.splice(index, 1)
  }

  // 实体 id → 目录项索引（chip 可读名查询 O(1)化：流式期间渲染函数每批重跑，
  // 原逐 chip 的 sources/releases 线性扫描会随目录规模线性放大）
  const sourceById = computed(() => new Map(sources.value.map((s) => [s.id, s])))
  const releaseById = computed(() => new Map(releases.value.map((r) => [r.id, r])))

  function entityLabel(e: AgentEntityRefSeed): string {
    if (e.kind === 'source') {
      const s = sourceById.value.get(e.id)
      return s ? `${s.source_type} | ${sourceDisplayName(s)}` : `source #${e.id}`
    }
    const r = releaseById.value.get(e.id)
    return r ? releaseDisplayName(r) : `release #${e.id}`
  }

  function entityKindLabel(kind: string): string {
    return kind === 'source' ? t('agent.entity_source') : t('agent.entity_release')
  }

  // ── 引用 chip 全文悬浮提示（仅文本被截断时显示，跟随鼠标）──
  const chipTooltip = ref<{ x: number; y: number; text: string } | null>(null)

  function chipTextTruncated(el: HTMLElement): boolean {
    return el.scrollWidth > el.clientWidth + 1
  }

  function placeChipTooltip(x: number, y: number, text: string) {
    const maxWidth = 480
    const margin = 16
    const left = Math.max(margin, Math.min(x + 12, window.innerWidth - maxWidth - margin))
    chipTooltip.value = { x: left, y: y + 12, text }
  }

  function handleChipEnter(e: MouseEvent, text: string) {
    const el = e.currentTarget as HTMLElement
    if (!chipTextTruncated(el)) return
    placeChipTooltip(e.clientX, e.clientY, text)
  }

  function handleChipMove(e: MouseEvent) {
    if (!chipTooltip.value) return
    placeChipTooltip(e.clientX, e.clientY, chipTooltip.value.text)
  }

  function hideChipTooltip() {
    chipTooltip.value = null
  }

  function handleInput() {
    const el = textareaRef.value
    if (!el) return
    const before = el.value.slice(0, el.selectionStart)
    const skillMatch = before.match(SKILL_TRIGGER)
    const entityMatch = before.match(ENTITY_TRIGGER)
    if (skillMatch && !entityMatch) {
      skillQuery.value = skillMatch[1]
      skillMenuIndex.value = 0
      showSkillMenu.value = true
      showEntityMenu.value = false
    } else if (entityMatch) {
      entityQuery.value = entityMatch[1]
      entityMenuIndex.value = 0
      showEntityMenu.value = true
      showSkillMenu.value = false
    } else {
      showSkillMenu.value = false
      showEntityMenu.value = false
    }
  }

  const filteredSkills = computed(() => {
    const q = skillQuery.value.toLowerCase()
    return skills.value.filter((s) => skillShortName(s).toLowerCase().includes(q) || s.toLowerCase().includes(q))
  })

  // [[ 实体：无前缀时两类都模糊搜；s: / r: 前缀限定类型
  const filteredSources = computed(() => {
    const q = entityQuery.value.toLowerCase()
    if (q.startsWith('r:')) return []
    const name = q.startsWith('s:') ? q.slice(2) : q
    return sources.value.filter((s) =>
      `${s.owner}/${s.repo} ${s.source_type} ${s.description ?? ''}`.toLowerCase().includes(name),
    )
  })

  const filteredReleases = computed(() => {
    const q = entityQuery.value.toLowerCase()
    if (q.startsWith('s:')) return []
    const name = q.startsWith('r:') ? q.slice(2) : q
    return releases.value
      .filter((r) =>
        `${r.owner}/${r.repo} ${r.tag_name} ${r.release_name} ${r.source_description ?? ''}`
          .toLowerCase()
          .includes(name),
      )
      .slice(0, 30)
  })

  const filteredSourcesCount = computed(() => filteredSources.value.length)
  const filteredReleasesCount = computed(() => filteredReleases.value.length)
  const entityMenuHasMatch = computed(() => filteredSourcesCount.value > 0 || filteredReleasesCount.value > 0)

  /** 菜单项可读名（可读名优先，回退 ID）：按源类型注册表分发——
   * 视频源 description 存真实频道名/UP 主名；GitHub 等显示 owner/repo
   * （其 description 是仓库描述文字，不能当名称用）。 */
  function sourceDisplayName(s: Source): string {
    const def = getSourceTypeDef(s.source_type)
    return def?.displayName?.(s.owner, s.repo, s.description) ?? (s.repo ? `${s.owner}/${s.repo}` : s.owner)
  }

  function releaseDisplayName(r: ReleaseInfo): string {
    const title = r.release_name && r.release_name !== r.tag_name ? r.release_name : r.tag_name
    const def = getSourceTypeDef(r.source_type)
    // 视频类源：owner 是 channel_id/UID 不可读，displayName 取 description 存的频道名/UP 主名
    // （YouTube 兼容旧版前缀、B 站 UP 主名均由注册表实现）；其余源 owner/repo 即仓库名。
    const name =
      def?.displayName?.(r.owner, r.repo, r.source_description) ?? (r.repo ? `${r.owner}/${r.repo}` : r.owner)
    return `${name} · ${title}`
  }

  function replaceTrigger(replacement: string) {
    const el = textareaRef.value
    if (!el) return
    const before = el.value.slice(0, el.selectionStart)
    const after = el.value.slice(el.selectionStart)
    const skillMatch = before.match(SKILL_TRIGGER)
    const entityMatch = before.match(ENTITY_TRIGGER)
    const start =
      skillMatch && !entityMatch
        ? before.length - skillMatch[1].length - 1
        : entityMatch
          ? before.length - entityMatch[1].length - 2
          : before.length
    el.value = el.value.slice(0, start) + replacement + after
    instruction.value = el.value
    const pos = start + replacement.length
    el.setSelectionRange(pos, pos)
    el.focus()
  }

  function pickSkill(path: string) {
    skillPath.value = path
    // 输入框只插入短名（带 @ 前缀所见即所得；skillPath 独立字段携带完整路径提交）
    replaceTrigger(`@${skillShortName(path)} `)
    showSkillMenu.value = false
  }

  function clearSkill() {
    skillPath.value = null
  }

  /** 会话切换清空（§4.2 三处清空差异对照表，按 mode 逐条复刻）：
   *  引用/指令/技能三种 mode 都清；附件 files 只在 switch / new 清
   *  （delete 后切换保留附件是现状行为）。菜单显隐不清（原实现即如此）。 */
  function resetForSessionSwitch(mode: SessionSwitchMode) {
    entities.value = []
    skillPath.value = null
    instruction.value = ''
    if (mode !== 'delete') files.value = []
  }

  function pickEntity(kind: 'source' | 'release', id: number) {
    replaceTrigger(`[[${kind}:${id}]] `)
    showEntityMenu.value = false
  }

  /** 聚焦输入框（编排层 onMounted / startNewSession 用）。 */
  function focus() {
    textareaRef.value?.focus()
  }

  /** 聚焦并把光标送到文本末尾（afterAttach / 重试回填用：拖完/还原即可打字发送）。 */
  function focusAtEnd() {
    const el = textareaRef.value
    if (!el) return
    el.focus()
    el.setSelectionRange(el.value.length, el.value.length)
  }

  /** 关闭引用菜单（全局 pointerdown 收起用）。
   *  excludeTextarea = true（点击落在输入框上）：skill/entity 菜单跟随输入
   *  （显隐由输入框自身事件管理），保持不动——点输入框不收菜单的豁免语义。 */
  function closeMenus(excludeTextarea: boolean) {
    if (excludeTextarea) return
    if (showSkillMenu.value) showSkillMenu.value = false
    if (showEntityMenu.value) showEntityMenu.value = false
  }

  onUnmounted(() => {
    if (flashTimer) {
      clearTimeout(flashTimer)
      flashTimer = null
    }
  })

  return {
    instruction,
    entities,
    skillPath,
    files,
    textareaRef,
    showSkillMenu,
    showEntityMenu,
    skillMenuIndex,
    entityMenuIndex,
    flashKey,
    attachAnnouncement,
    chipTooltip,
    filteredSkills,
    filteredSources,
    filteredReleases,
    filteredSourcesCount,
    filteredReleasesCount,
    entityMenuHasMatch,
    handleInput,
    handleAttachFiles,
    removeFile,
    fileDisplayName,
    addEntity,
    afterAttach,
    removeEntity,
    entityLabel,
    entityKindLabel,
    handleChipEnter,
    handleChipMove,
    hideChipTooltip,
    sourceDisplayName,
    releaseDisplayName,
    pickSkill,
    clearSkill,
    pickEntity,
    resetForSessionSwitch,
    focus,
    focusAtEnd,
    closeMenus,
  }
}
