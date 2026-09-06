import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { shallowMount, mount } from '@vue/test-utils'
import { defineComponent, nextTick, ref } from 'vue'
import ReleaseItem from '../components/ReleaseItem.vue'
import MarkdownContent from '../components/common/MarkdownContent.vue'
import { ShowToastKey, AiEnabledKey, ShowImportanceKey, AgentEnabledKey, AgentWorkspaceKey } from '../injection-keys'
import { t, setLocale } from '../i18n'
import { createRelease } from './helpers'
import type { ReleaseInfo } from '../api/releases'

vi.mock('../api/releases', () => ({
  setNotificationState: vi.fn(),
  deleteRelease: vi.fn(),
  translateRelease: vi.fn(),
  setReleaseFlag: vi.fn(),
}))

vi.mock('../api/client', () => ({
  openReleaseUrl: vi.fn(),
}))


// 仅替换 formatDate（jsdom 无时区/本地化渲染）；isUnreadStatus/statusClass/statusLabel
// 必须引用 utils 真实实现——曾在此处复制实现，utils 语义变化时测试按旧语义放行（见阶段 2-4）
vi.mock('../utils', async importOriginal => {
  const actual = await importOriginal<typeof import('../utils')>()
  return {
    ...actual,
    formatDate: vi.fn(() => '2024-06-15'),
  }
})

// contextMenuBus 为纯内存模块，用真实实现（互斥关闭是菜单行为的一部分）
import { setNotificationState, deleteRelease, setReleaseFlag } from '../api/releases'
import { openReleaseUrl } from '../api/client'
import { closeAllContextMenus } from '../composables/contextMenuBus'

// ContextMenu 自定义 stub：渲染菜单项文本，点击触发 action 事件
const ContextMenuStub = defineComponent({
  name: 'ContextMenu',
  props: { x: Number, y: Number, items: Array },
  emits: ['action', 'close'],
  template: `
    <div class="stub-menu">
      <span v-for="item in items" :key="item.id" class="stub-menu-item" @click="$emit('action', item.id)">{{ item.label }}</span>
    </div>`,
})

function mountRelease(release: ReleaseInfo, provideOverrides: Record<symbol, unknown> = {}) {
  return shallowMount(ReleaseItem, {
    props: { release },
    global: {
      provide: {
        [ShowToastKey as symbol]: vi.fn(),
        ...provideOverrides,
      },
    },
  })
}

// 最近一次挂载注入的 openAgentWorkspace spy（「发送到 Agent」行为断言用）
let lastAgentOpen: ReturnType<typeof vi.fn> | null = null
export function lastOpenAgentWorkspace() {
  return lastAgentOpen
}

// 菜单测试专用：真实渲染 + ContextMenu stub（右键菜单为事件入口）
// agentEnabled=true 时菜单含「发送到 Agent」；openAgentWorkspace 记录调用供行为断言
function mountWithMenus(release: ReleaseInfo, aiEnabled = true, agentEnabled = false) {
  const openAgentWorkspace = vi.fn()
  lastAgentOpen = openAgentWorkspace
  return mount(ReleaseItem, {
    props: { release },
    global: {
      provide: {
        [ShowToastKey as symbol]: vi.fn(),
        [AiEnabledKey as symbol]: ref(aiEnabled),
        [AgentEnabledKey as symbol]: ref(agentEnabled),
        [AgentWorkspaceKey as symbol]: openAgentWorkspace,
      },
      stubs: { MarkdownContent: true, ContextMenu: ContextMenuStub },
    },
  })
}

beforeEach(() => {
  vi.clearAllMocks()
  setLocale('zh-CN')
})

afterEach(() => {
  setLocale('zh-CN')
})

/**
 * ReleaseItem.vue 真实运行场景测试（真实 i18n 字典）
 *
 * 版本列表中的单个版本卡片，提供：
 * - 版本信息展示（仓库/标签/标题/日期/状态/预发布标记）
 * - AI 摘要与截断悬浮提示
 * - 右键菜单（打开链接/复制链接/删除）和摘要右键菜单（阅读全文/复制摘要/翻译）
 * - 通知状态变更（点击链接自动标记、稍后提醒、忽略）
 */
describe('ReleaseItem.vue — 渲染', () => {
  it('显示仓库名和标签名', () => {
    const wrapper = mountRelease(createRelease())

    expect(wrapper.text()).toContain('tauri-apps/tauri')
    expect(wrapper.text()).toContain('v2.0.0')
  })

  it('release_name 与 tag_name 不同时显示标题', () => {
    const wrapper = mountRelease(createRelease({ release_name: 'Tauri 2.0 Stable' }))

    expect(wrapper.text()).toContain('Tauri 2.0 Stable')
  })

  it('release_name 等于 tag_name 时不显示标题', () => {
    const wrapper = mountRelease(createRelease({ release_name: 'v2.0.0' }))

    expect(wrapper.find('.release-title').exists()).toBe(false)
  })

  it('预发布版显示 prerelease badge', () => {
    const wrapper = mountRelease(createRelease({ prerelease: true }))

    expect(wrapper.text()).toContain(t('release.prerelease'))
  })

  it('正式版不显示 prerelease badge', () => {
    const wrapper = mountRelease(createRelease({ prerelease: false }))

    expect(wrapper.text()).not.toContain(t('release.prerelease'))
  })

  it('snoozed 且 snooze_until 有值时显示提醒时间', () => {
    const wrapper = mountRelease(createRelease({
      notification_status: 'snoozed',
      snooze_until: '2025-06-10T00:00:00Z',
    }))

    expect(wrapper.text()).toContain(t('release.snooze_until', '2024-06-15'))
  })

  it('AI 摘要存在时渲染摘要行', () => {
    const wrapper = mountRelease(createRelease({ ai_summary: '修复了关键错误' }))

    expect(wrapper.find('.release-summary-text').exists()).toBe(true)
    expect(wrapper.text()).toContain('修复了关键错误')
  })

  it('无 AI 摘要时不显示摘要行', () => {
    const wrapper = mountRelease(createRelease({ ai_summary: null }))

    expect(wrapper.find('.release-summary-line').exists()).toBe(false)
  })
})

describe('ReleaseItem.vue — 通知状态操作', () => {
  it('未读版本（pending）上显示 Ignore 按钮', () => {
    const wrapper = mountRelease(createRelease({ notification_status: 'pending' }))

    expect(wrapper.text()).toContain(t('release.ignore'))
  })

  it('已读版本（clicked）上显示 Snooze 按钮，无 Ignore', () => {
    const wrapper = mountRelease(createRelease({ notification_status: 'clicked' }))

    expect(wrapper.text()).toContain(t('release.snooze'))
    expect(wrapper.text()).not.toContain(t('release.ignore'))
  })

  it('已读版本（ignored）上显示 Snooze 按钮', () => {
    const wrapper = mountRelease(createRelease({ notification_status: 'ignored' }))

    expect(wrapper.text()).toContain(t('release.snooze'))
  })

  it('点击 Ignore 调用 setNotificationState("ignored")', async () => {
    vi.mocked(setNotificationState).mockResolvedValue(undefined)
    const wrapper = mountRelease(createRelease({ id: 10, notification_status: 'pending' }))

    await wrapper.find('.btn-danger-soft').trigger('click')
    await new Promise(resolve => setTimeout(resolve, 0))

    expect(setNotificationState).toHaveBeenCalledWith(10, 'ignored', undefined)
  })

  it('点击 Ignore 后 emit update', async () => {
    vi.mocked(setNotificationState).mockResolvedValue(undefined)
    const wrapper = mountRelease(createRelease({ id: 10, notification_status: 'pending' }))

    await wrapper.find('.btn-danger-soft').trigger('click')
    await new Promise(resolve => setTimeout(resolve, 0))

    expect(wrapper.emitted('update')).toBeTruthy()
  })

  it('状态更新失败时调用 Toast 报错', async () => {
    const toast = vi.fn()
    vi.mocked(setNotificationState).mockRejectedValue(new Error('err.network'))

    const wrapper = shallowMount(ReleaseItem, {
      props: { release: createRelease({ id: 10, notification_status: 'pending' }) },
      global: { provide: { [ShowToastKey as symbol]: toast } },
    })

    await wrapper.find('.btn-danger-soft').trigger('click')
    await new Promise(resolve => setTimeout(resolve, 0))

    expect(toast).toHaveBeenCalledWith(expect.stringContaining(t('release.status_failed')))
  })

  it('未读版本点击链接 → 打开 URL + 设置 clicked + emit update', async () => {
    vi.mocked(setNotificationState).mockResolvedValue(undefined)
    vi.mocked(openReleaseUrl).mockResolvedValue(undefined)

    const wrapper = mountRelease(createRelease({
      id: 5,
      html_url: 'https://github.com/test/repo/releases/tag/v1.0.0',
      notification_status: 'pending',
    }))

    await wrapper.find('.release-link-action').trigger('click')
    await new Promise(resolve => setTimeout(resolve, 0))

    expect(openReleaseUrl).toHaveBeenCalledWith('https://github.com/test/repo/releases/tag/v1.0.0')
    expect(setNotificationState).toHaveBeenCalledWith(5, 'clicked')
    expect(wrapper.emitted('update')).toBeTruthy()
  })

  it('已读版本点击链接 → 只打开 URL，不设置状态', async () => {
    vi.mocked(openReleaseUrl).mockResolvedValue(undefined)

    const wrapper = mountRelease(createRelease({
      id: 6,
      html_url: 'https://github.com/test/repo/releases/tag/v1.0.0',
      notification_status: 'clicked',
    }))

    await wrapper.find('.release-link-action').trigger('click')

    expect(openReleaseUrl).toHaveBeenCalledWith('https://github.com/test/repo/releases/tag/v1.0.0')
    expect(setNotificationState).not.toHaveBeenCalled()
  })

  it('Snooze 按钮调用 setNotificationState("snoozed", 1440)', async () => {
    vi.mocked(setNotificationState).mockResolvedValue(undefined)
    const wrapper = mountRelease(createRelease({ id: 7, notification_status: 'clicked' }))

    const snoozeBtn = wrapper.findAll('button').find(b => b.text().includes(t('release.snooze')))
    expect(snoozeBtn).toBeTruthy()
    await snoozeBtn!.trigger('click')
    await new Promise(resolve => setTimeout(resolve, 0))

    expect(setNotificationState).toHaveBeenCalledWith(7, 'snoozed', 1440)
  })
})

describe('ReleaseItem.vue — contextMenuBus 集成', () => {
  it('挂载后菜单可被总线统一关闭（真实 registerCloser 链路）', async () => {
    const wrapper = mountWithMenus(createRelease({ ai_summary: '摘要', body: '## Body' }))

    await wrapper.find('.release-link-action').trigger('contextmenu')
    expect(wrapper.find('.stub-menu').exists()).toBe(true)

    closeAllContextMenus()
    await nextTick()

    expect(wrapper.find('.stub-menu').exists()).toBe(false)
  })
})

describe('ReleaseItem.vue — 重要性样式', () => {
  it('ai_importance="大" → release-importance-high', () => {
    const wrapper = mountRelease(createRelease({ ai_importance: '大' }), { [ShowImportanceKey as symbol]: ref(true) })

    expect(wrapper.find('.release-item').classes()).toContain('release-importance-high')
  })

  it('ai_importance="中" → release-importance-medium', () => {
    const wrapper = mountRelease(createRelease({ ai_importance: '中' }), { [ShowImportanceKey as symbol]: ref(true) })

    expect(wrapper.find('.release-item').classes()).toContain('release-importance-medium')
  })

  it('ai_importance="小" → release-importance-low', () => {
    const wrapper = mountRelease(createRelease({ ai_importance: '小' }), { [ShowImportanceKey as symbol]: ref(true) })

    expect(wrapper.find('.release-item').classes()).toContain('release-importance-low')
  })

  it('无重要性时不加重要性 class', () => {
    const wrapper = mountRelease(createRelease({ ai_importance: null }))

    const classes = wrapper.find('.release-item').classes()
    expect(classes).not.toContain('release-importance-high')
    expect(classes).not.toContain('release-importance-medium')
    expect(classes).not.toContain('release-importance-low')
  })
})

describe('ReleaseItem.vue — 点击链接标记时序（P0 #3）', () => {
  it('未读版本标记失败时不打开链接，并提示失败', async () => {
    const toast = vi.fn()
    vi.mocked(setNotificationState).mockRejectedValue(new Error('err.network'))
    vi.mocked(openReleaseUrl).mockResolvedValue(undefined)

    const wrapper = shallowMount(ReleaseItem, {
      props: { release: createRelease({ id: 5, notification_status: 'pending' }) },
      global: { provide: { [ShowToastKey as symbol]: toast } },
    })

    await wrapper.find('.release-link-action').trigger('click')
    await new Promise(r => setTimeout(r, 0))

    expect(setNotificationState).toHaveBeenCalledWith(5, 'clicked')
    // 标记失败 → 不应打开链接，避免“链接已开但列表仍显示未读”
    expect(openReleaseUrl).not.toHaveBeenCalled()
    expect(toast).toHaveBeenCalledWith(expect.stringContaining(t('release.status_failed')))
  })

  it('未读版本标记成功后才打开链接（调用顺序：先标记后打开）', async () => {
    vi.mocked(setNotificationState).mockResolvedValue(undefined)
    vi.mocked(openReleaseUrl).mockResolvedValue(undefined)

    const wrapper = mountRelease(createRelease({ id: 5, notification_status: 'pending' }))

    await wrapper.find('.release-link-action').trigger('click')
    await new Promise(r => setTimeout(r, 0))

    expect(setNotificationState).toHaveBeenCalledWith(5, 'clicked')
    expect(openReleaseUrl).toHaveBeenCalled()
    // 严格校验调用顺序：setNotificationState 必须早于 openReleaseUrl
    const stateOrder = vi.mocked(setNotificationState).mock.invocationCallOrder[0]
    const openOrder = vi.mocked(openReleaseUrl).mock.invocationCallOrder[0]
    expect(stateOrder).toBeLessThan(openOrder)
  })
})

describe('ReleaseItem.vue — 内容预览（摘要 > 译文 > 原文）', () => {
  it('有摘要时显示摘要行；有正文时同时显示「阅读全文」按钮', () => {
    const wrapper = mountRelease(createRelease({ ai_summary: '修复了关键错误', body: '## Full notes' }))

    expect(wrapper.find('.release-summary-text').exists()).toBe(true)
    expect(wrapper.find('.release-body-text').exists()).toBe(false)
    expect(wrapper.find('.release-expand-btn').exists()).toBe(true)
  })

  it('无摘要时优先显示译文预览', () => {
    const wrapper = mountRelease(createRelease({ body: '## Original', body_translated: '## 译文' }))

    expect(wrapper.find('.release-summary-line').exists()).toBe(false)
    expect(wrapper.find('.release-body-text').exists()).toBe(true)
    // shallowMount 下子组件为 stub，通过 props() 断言传入的预览内容
    expect((wrapper.findComponent(MarkdownContent).props() as { content: string }).content).toBe('## 译文')
  })

  it('无摘要无译文时显示原文预览', () => {
    const wrapper = mountRelease(createRelease({ body: '## Original' }))

    expect((wrapper.findComponent(MarkdownContent).props() as { content: string }).content).toBe('## Original')
  })

  it('无任何内容时不渲染内容区与「阅读全文」按钮', () => {
    const wrapper = mountRelease(createRelease())

    expect(wrapper.find('.release-content').exists()).toBe(false)
    expect(wrapper.find('.release-expand-btn').exists()).toBe(false)
  })

  it('只有摘要无正文时不显示「阅读全文」按钮', () => {
    const wrapper = mountRelease(createRelease({ ai_summary: '仅摘要' }))

    expect(wrapper.find('.release-summary-text').exists()).toBe(true)
    expect(wrapper.find('.release-expand-btn').exists()).toBe(false)
  })

  it('卡片不再渲染内容切换标签栏（切换集中在详情弹窗）', () => {
    const wrapper = mountRelease(createRelease({ ai_summary: '摘要', body: 'body', body_translated: '译文' }))

    expect(wrapper.find('.release-view-tabs').exists()).toBe(false)
  })
})

describe('ReleaseItem.vue — 打开详情弹窗', () => {
  it('点击正文预览 → emit open-detail（仅携带 release）', async () => {
    const release = createRelease({ id: 42, body: '## Body' })
    const wrapper = mountRelease(release)

    await wrapper.find('.release-body-text').trigger('click')

    const events = wrapper.emitted('open-detail')
    expect(events).toBeTruthy()
    expect(events![0]).toEqual([release])
  })

  it('摘要卡片点击「阅读全文」→ emit open-detail', async () => {
    const release = createRelease({ id: 43, ai_summary: '摘要', body: '## Body' })
    const wrapper = mountRelease(release)

    await wrapper.find('.release-expand-btn').trigger('click')

    expect(wrapper.emitted('open-detail')![0]).toEqual([release])
  })
})

describe('ReleaseItem.vue — 翻译入口（右键菜单事件入口）', () => {
  it('有原文无译文且 AI 启用时，摘要右键菜单含「翻译」', async () => {
    const wrapper = mountWithMenus(createRelease({ ai_summary: '摘要', body: '## Body' }))

    await wrapper.find('.release-summary-text').trigger('contextmenu')
    const labels = wrapper.findAll('.stub-menu-item').map(i => i.text())

    expect(labels).toContain(t('context.translate'))
    expect(labels).toContain(t('context.copy_content'))
  })

  it('已有译文时不提供「翻译」选项', async () => {
    const wrapper = mountWithMenus(createRelease({ ai_summary: '摘要', body: '## Body', body_translated: '译文' }))

    await wrapper.find('.release-summary-text').trigger('contextmenu')
    const labels = wrapper.findAll('.stub-menu-item').map(i => i.text())

    expect(labels).not.toContain(t('context.translate'))
  })

  it('无原文时不提供「翻译」选项', async () => {
    const wrapper = mountWithMenus(createRelease({ ai_summary: '仅摘要' }))

    await wrapper.find('.release-summary-text').trigger('contextmenu')
    const labels = wrapper.findAll('.stub-menu-item').map(i => i.text())

    expect(labels).not.toContain(t('context.translate'))
  })
})

describe('ReleaseItem.vue — 右键菜单 i18n 响应式（P1 #6）', () => {
  it('语言切换后右键菜单 label 实时更新（真实 setLocale）', async () => {
    const wrapper = mountWithMenus(createRelease({ ai_summary: '摘要', body: '## Body' }))

    // Agent 未启用 → 菜单保持分支前形态（无 Agent 项）
    await wrapper.find('.release-link-action').trigger('contextmenu')
    expect(wrapper.findAll('.stub-menu-item').map(i => i.text())).toEqual([
      t('context.open'),
      t('context.copy_link'),
      '',
      t('context.delete_release'),
    ])

    // 旗标按钮右键专属菜单（zh-CN）：六色（未标记无「取消旗标」项）
    await wrapper.find('.release-flag-action').trigger('contextmenu')
    expect(wrapper.findAll('.stub-menu-item').map(i => i.text())).toEqual([
      t('release.flag_red'),
      t('release.flag_orange'),
      t('release.flag_yellow'),
      t('release.flag_green'),
      t('release.flag_blue'),
      t('release.flag_purple'),
    ])

    // 摘要右键菜单（zh-CN）：无 Agent 项
    await wrapper.find('.release-summary-text').trigger('contextmenu')
    expect(wrapper.findAll('.stub-menu-item').map(i => i.text())).toEqual([
      t('release.read_full'),
      t('context.copy_content'),
      t('context.translate'),
    ])

    // 切换到英文（真实 i18n 模块）
    setLocale('en-US')
    await nextTick()

    await wrapper.find('.release-link-action').trigger('contextmenu')
    expect(wrapper.findAll('.stub-menu-item').map(i => i.text())).toEqual([
      'Open', 'Copy Link', '', 'Delete Version',
    ])

    await wrapper.find('.release-flag-action').trigger('contextmenu')
    expect(wrapper.findAll('.stub-menu-item').map(i => i.text())).toEqual([
      'Red', 'Orange', 'Yellow', 'Green', 'Blue', 'Purple',
    ])

    await wrapper.find('.release-summary-text').trigger('contextmenu')
    expect(wrapper.findAll('.stub-menu-item').map(i => i.text())).toEqual([
      'Read full note', 'Copy Content', 'Translate',
    ])
  })
})

describe('ReleaseItem.vue — 发送到 Agent', () => {
  it('Agent 启用时菜单含「发送到 Agent」，点击唤起工作区并携带版本引用', async () => {
    const release = createRelease({ id: 7, ai_summary: '摘要', body: '## Body' })
    const wrapper = mountWithMenus(release, true, true)

    await wrapper.find('.release-link-action').trigger('contextmenu')
    const labels = wrapper.findAll('.stub-menu-item').map(i => i.text())
    expect(labels).toEqual([
      t('context.open'),
      t('context.copy_link'),
      '',
      t('agent.send_to'),
      t('context.delete_release'),
    ])

    await wrapper.findAll('.stub-menu-item')[3].trigger('click')
    expect(lastOpenAgentWorkspace()).toHaveBeenCalledWith({ entities: [{ kind: 'release', id: 7 }] })
  })

  it('Agent 启用时摘要右键菜单也含「发送到 Agent」', async () => {
    const wrapper = mountWithMenus(createRelease({ ai_summary: '摘要', body: '## Body' }), true, true)

    await wrapper.find('.release-summary-text').trigger('contextmenu')
    const labels = wrapper.findAll('.stub-menu-item').map(i => i.text())
    expect(labels).toEqual([
      t('release.read_full'),
      t('context.copy_content'),
      t('context.translate'),
      t('agent.send_to'),
    ])
  })
})

// ============ YouTube 源展示 ============

describe('ReleaseItem.vue — YouTube 源', () => {
  function ytRelease(overrides: Partial<ReleaseInfo> = {}): ReleaseInfo {
    return createRelease({
      source_type: 'youtube',
      owner: 'UCXuqSBlHAE6Xw-yeJA0Tunw',
      repo: '',
      tag_name: 'abc123',
      release_name: '一段很长的视频标题：用于验证长标题在两行内截断显示的效果测试',
      html_url: 'https://www.youtube.com/watch?v=abc123',
      extra_metadata: JSON.stringify({ kind: 'video', thumbnail: 'https://i.ytimg.com/vi/abc123/hqdefault.jpg' }),
      ...overrides,
    })
  }

  it('显示频道名而非 channel_id，且不显示 videoId 标签', () => {
    const wrapper = mountRelease(ytRelease({ source_description: '时局眼' }))
    expect(wrapper.text()).toContain('时局眼')
    expect(wrapper.text()).not.toContain('UCXuqSBlHAE6Xw')
    expect(wrapper.text()).not.toContain('abc123')
  })

  it('兼容旧版 "YouTube channel: " 前缀描述', () => {
    const wrapper = mountRelease(ytRelease({ source_description: 'YouTube channel: Videos' }))
    expect(wrapper.text()).toContain('Videos')
    expect(wrapper.text()).not.toContain('YouTube channel: Videos')
  })

  it('无频道名时回退 owner', () => {
    const wrapper = mountRelease(ytRelease({ source_description: null }))
    expect(wrapper.text()).toContain('UCXuqSBlHAE6Xw-yeJA0Tunw')
  })

  it('显示视频封面图与直播徽标', () => {
    const wrapper = mountRelease(ytRelease({ extra_metadata: JSON.stringify({ kind: 'live', thumbnail: 'https://i.ytimg.com/vi/abc123/mqdefault.jpg' }) }))
    const img = wrapper.find('img.yt-thumb')
    expect(img.exists()).toBe(true)
    // 封面经 media 网关（Rust 按代理设置下载），不再由浏览器直连远程图床
    expect(img.attributes('src')).toBe(
      'http://media.localhost/' + encodeURIComponent('https://i.ytimg.com/vi/abc123/mqdefault.jpg'),
    )
    // no-referrer：B 站 CDN（hdslb.com）对非 bilibili 域名 Referer 返回 403，封面必须裸 Referer 加载
    expect(img.attributes('referrerpolicy')).toBe('no-referrer')
    expect(wrapper.find('.yt-live-badge').exists()).toBe(true)
  })

  it('显示视频时长角标（ISO 8601 → MM:SS / H:MM:SS）', () => {
    // PT12M34S → 12:34
    const short = mountRelease(ytRelease({ extra_metadata: JSON.stringify({ kind: 'video', duration: 'PT12M34S' }) }))
    expect(short.find('.yt-duration-badge').exists()).toBe(true)
    expect(short.find('.yt-duration-badge').text()).toBe('12:34')
    // PT1H2M3S → 1:02:03
    const long = mountRelease(ytRelease({ extra_metadata: JSON.stringify({ kind: 'video', duration: 'PT1H2M3S' }) }))
    expect(long.find('.yt-duration-badge').text()).toBe('1:02:03')
  })

  it('无时长（RSS 模式）时不显示时长角标', () => {
    const wrapper = mountRelease(ytRelease({ extra_metadata: JSON.stringify({ kind: 'video', thumbnail: 'https://i.ytimg.com/vi/abc123/hqdefault.jpg' }) }))
    expect(wrapper.find('.yt-duration-badge').exists()).toBe(false)
  })

  it('视频标题使用两行截断样式', () => {
    const wrapper = mountRelease(ytRelease())
    expect(wrapper.find('.release-title-yt').exists()).toBe(true)
  })
})

// ============ YouTube B 站风格布局 ============

describe('ReleaseItem.vue — YouTube B 站风格布局', () => {
  function yt(overrides: Partial<ReleaseInfo> = {}) {
    return createRelease({
      source_type: 'youtube',
      owner: 'UCXuqSBlHAE6Xw-yeJA0Tunw',
      repo: '',
      tag_name: 'abc123',
      release_name: '视频标题',
      html_url: 'https://www.youtube.com/watch?v=abc123',
      source_description: '时局眼',
      body: '这是视频简介内容，用于测试阅读全文。',
      extra_metadata: JSON.stringify({ kind: 'video', thumbnail: 'https://i.ytimg.com/vi/abc123/hqdefault.jpg' }),
      ...overrides,
    })
  }

  it('左封面 + 右标题/简介的横排布局', () => {
    const wrapper = mountRelease(yt())
    const layout = wrapper.find('.yt-layout')
    expect(layout.exists()).toBe(true)
    // 封面按钮与信息区并存
    expect(layout.find('.yt-thumb-btn').exists()).toBe(true)
    expect(layout.find('.yt-info').exists()).toBe(true)
    // 标题在信息区
    expect(layout.find('.yt-info .release-title-yt').text()).toBe('视频标题')
    // 简介在信息区且可点击
    expect(layout.find('.yt-info .yt-desc').exists()).toBe(true)
  })

  it('点击简介或阅读全文打开详情弹窗', async () => {
    const wrapper = mountRelease(yt())
    await wrapper.find('.yt-info .yt-desc').trigger('click')
    expect(wrapper.emitted('open-detail')).toBeTruthy()
  })

  it('无简介时只显示封面，无阅读全文按钮', () => {
    const wrapper = mountRelease(yt({ body: null }))
    const layout = wrapper.find('.yt-layout')
    expect(layout.find('.yt-thumb-btn').exists()).toBe(true)
    expect(layout.find('.yt-desc').exists()).toBe(false)
    expect(wrapper.find('.release-expand-btn').exists()).toBe(false)
  })

  it('播放量与阅读全文按钮同行（底部行，不额外占行）', async () => {
    const wrapper = mountRelease(yt({ extra_metadata: JSON.stringify({ kind: 'video', view_count: 1234567 }) }))
    const footer = wrapper.find('.yt-footer-row')
    expect(footer.exists()).toBe(true)
    // 标题独占标题行，播放量不挤占标题空间
    expect(wrapper.find('.yt-title-row').exists()).toBe(false)
    expect(wrapper.find('.release-title-yt').exists()).toBe(true)
    // 播放量与按钮在同一底部行
    expect(footer.find('.yt-view-count').exists()).toBe(true)
    expect(footer.find('.release-expand-btn').exists()).toBe(true)
    // 中文环境：1234567 → 123.5万，渲染真实 i18n 文案
    expect(footer.find('.yt-view-count').text()).toBe(t('release.yt_views', '123.5万'))
  })

  it('无播放量（YouTube RSS 模式）时底部行只有阅读全文按钮', () => {
    const wrapper = mountRelease(yt())
    expect(wrapper.find('.yt-view-count').exists()).toBe(false)
    // 按钮仍存在（左对齐，不因缺播放量丢失）
    expect(wrapper.find('.yt-footer-row .release-expand-btn').exists()).toBe(true)
  })

  it('B 站播放量同样显示在底部行', async () => {
    const wrapper = mountRelease(createRelease({
      source_type: 'bilibili',
      owner: '476599099',
      repo: '',
      tag_name: 'BV1xx',
      release_name: 'B 站视频',
      html_url: 'https://www.bilibili.com/video/BV1xx',
      source_description: '某UP主',
      extra_metadata: JSON.stringify({ kind: 'video', view_count: 99999999 }),
    }))
    const view = wrapper.find('.yt-view-count')
    expect(view.exists()).toBe(true)
    // 中文环境：99999999 → 1亿，渲染真实 i18n 文案
    expect(view.text()).toBe(t('release.yt_views', '1亿'))
  })

  it('http 封面自动升级为 https（兼容 CSP img-src 限制与 B 站旧数据）并经 media 网关', () => {
    const wrapper = mountRelease(yt({ extra_metadata: JSON.stringify({ kind: 'video', thumbnail: 'http://i0.hdslb.com/bfs/archive/abc.jpg' }) }))
    const img = wrapper.find('img.yt-thumb')
    expect(img.exists()).toBe(true)
    expect(img.attributes('src')).toBe(
      'http://media.localhost/' + encodeURIComponent('https://i0.hdslb.com/bfs/archive/abc.jpg'),
    )
  })
})

describe('ReleaseItem.vue — 摘要截断测量（窗口缩放回归）', () => {
  // 伪布局引擎：每行 10 字符、每字符 10px、最多渲染 3 行（模拟 -webkit-line-clamp: 3，
  // 超出部分不渲染；clamp 生效时第三行尾部被省略号占据，文本少渲染 1 字符）。
  // 模拟真实浏览器的关键行为：setEnd 偏移超过文本节点长度时抛 IndexSizeError。
  const CHARS_PER_LINE = 10
  const CHAR_W = 10
  const MAX_LINES = 3
  const CLAMP_RENDER_LIMIT = MAX_LINES * CHARS_PER_LINE - 1
  const BTN_W = 62 // 模拟「阅读全文」按钮真实宽度（jsdom 中 getBoundingClientRect 恒为 0）

  function createFakeRange() {
    let start = 0
    let end = 0
    let len = 0
    return {
      setStart(_node: Node, offset: number) { start = offset },
      setEnd(node: Node, offset: number) {
        len = node.textContent?.length ?? 0
        if (offset > len) throw new DOMException('Index or size is out of bounds', 'IndexSizeError')
        end = offset
      },
      getClientRects() {
        const cap = len > MAX_LINES * CHARS_PER_LINE ? CLAMP_RENDER_LIMIT : len
        const effEnd = Math.min(end, cap)
        if (effEnd <= start) return []
        const firstLine = Math.floor(start / CHARS_PER_LINE)
        const lastLine = Math.floor((effEnd - 1) / CHARS_PER_LINE)
        const rects: { width: number }[] = []
        for (let line = firstLine; line <= lastLine; line++) {
          const lineStart = Math.max(start, line * CHARS_PER_LINE)
          const lineEnd = Math.min(effEnd, (line + 1) * CHARS_PER_LINE)
          rects.push({ width: (lineEnd - lineStart) * CHAR_W })
        }
        return rects
      },
    }
  }

  function setClientWidth(el: Element, w: number) {
    Object.defineProperty(el, 'clientWidth', { configurable: true, value: w })
  }

  it('窗口缩窄触发截断后，重复测量不会在两态间振荡（按钮闪现/盖字回归）', async () => {    let roCallback: (() => void) | null = null
    class FakeResizeObserver {
      constructor(cb: () => void) { roCallback = cb }
      observe() {}
      unobserve() {}
      disconnect() {}
    }
    vi.stubGlobal('ResizeObserver', FakeResizeObserver)
    vi.spyOn(document, 'createRange').mockImplementation(createFakeRange as unknown as () => Range)

    try {
      // 35 字符 → 伪布局下 4 行，需要截断
      const wrapper = mountRelease(createRelease({ ai_summary: 'a'.repeat(35), body: '## Body' }))
      await nextTick()

      const line = () => wrapper.find('.release-summary-line')
      const textEl = () => wrapper.find('.release-summary-text').element as HTMLElement
      expect(line().exists()).toBe(true)
      expect(roCallback).toBeTruthy()
      // 按钮宽度 stub 为 62px（avail = 行宽 - 62 - 8）
      Object.defineProperty(wrapper.find('.release-expand-btn').element, 'getBoundingClientRect', {
        configurable: true,
        value: () => ({ width: BTN_W }),
      })

      // jsdom 无布局：初始测量 clientWidth=0 → 回退「按钮独立成行」
      expect(line().classes()).not.toContain('has-third-line')

      // 模拟窗口缩窄到行宽 100：触发 RO → 截断生效，按钮悬浮第三行右侧
      // avail = 100 - 62 - 8 = 30 → 第三行最多再放 3 字符（20~22）
      setClientWidth(textEl(), 100)
      roCallback!()
      await nextTick()
      expect(line().classes()).toContain('has-third-line')
      expect(textEl().textContent).toBe('a'.repeat(23) + '…')

      // 宽度未变时 RO 回调被守卫跳过，状态保持稳定（不随高度变化反复重测）
      roCallback!()
      await nextTick()
      expect(line().classes()).toContain('has-third-line')
      expect(textEl().textContent).toBe('a'.repeat(23) + '…')

      // 继续缩窄到 92：此时 DOM 渲染的是截断文本，再次测量必须基于完整文本。
      // 回归点：克隆若带走截断文本，setEnd 越界抛异常 → catch 重置 → has-third-line 丢失
      // avail = 92 - 62 - 8 = 22 → 第三行最多再放 2 字符（20~21）
      setClientWidth(textEl(), 92)
      roCallback!()
      await nextTick()
      expect(line().classes()).toContain('has-third-line')
      expect(textEl().textContent).toBe('a'.repeat(22) + '…')
    } finally {
      vi.unstubAllGlobals()
      vi.restoreAllMocks()
    }
  })
})

describe('ReleaseItem.vue — 旗标操作', () => {
  it('未标记卡片不渲染旗标贴纸，已标记渲染', () => {
    expect(mountRelease(createRelease({ flag: 0 })).find('.release-flag-chip').exists()).toBe(false)
    expect(mountRelease(createRelease({ flag: 2 })).find('.release-flag-chip').exists()).toBe(true)
  })

  it('点击旗标按钮：未标记 → 落默认旗标（红 = 1）', async () => {
    vi.mocked(setReleaseFlag).mockResolvedValue(undefined)
    const wrapper = mountRelease(createRelease({ id: 10, flag: 0 }))

    await wrapper.find('.release-flag-action').trigger('click')
    await new Promise(resolve => setTimeout(resolve, 0))

    expect(setReleaseFlag).toHaveBeenCalledWith(10, 1)
    expect(wrapper.emitted('update')).toBeTruthy()
  })

  it('点击旗标按钮：已标记（不论颜色）→ 清除（0）', async () => {
    vi.mocked(setReleaseFlag).mockResolvedValue(undefined)
    const wrapper = mountRelease(createRelease({ id: 10, flag: 3 }))

    await wrapper.find('.release-flag-action').trigger('click')
    await new Promise(resolve => setTimeout(resolve, 0))

    expect(setReleaseFlag).toHaveBeenCalledWith(10, 0)
  })

  it('写库失败时 toast 提示（携带后端错误信息，err.* 已翻译）', async () => {
    const toast = vi.fn()
    vi.mocked(setReleaseFlag).mockRejectedValue(new Error('err.release_flag_invalid|9'))

    const wrapper = shallowMount(ReleaseItem, {
      props: { release: createRelease({ id: 10, flag: 0 }) },
      global: { provide: { [ShowToastKey as symbol]: toast } },
    })

    await wrapper.find('.release-flag-action').trigger('click')
    await new Promise(resolve => setTimeout(resolve, 0))

    expect(toast).toHaveBeenCalledWith(expect.stringContaining(t('release.flag_failed')))
  })

  it('右键旗标按钮弹出颜色菜单，点击颜色项 → setReleaseFlag(id, n)', async () => {
    vi.mocked(setReleaseFlag).mockResolvedValue(undefined)
    const wrapper = mountWithMenus(createRelease({ id: 11 }))

    await wrapper.find('.release-flag-action').trigger('contextmenu')
    // 旗标菜单项顺序：flag-1..6（无 divider）
    await wrapper.findAll('.stub-menu-item')[2].trigger('click') // flag-3（绿）
    await new Promise(resolve => setTimeout(resolve, 0))

    expect(setReleaseFlag).toHaveBeenCalledWith(11, 3)
    expect(wrapper.emitted('update')).toBeTruthy()
  })

  it('键盘唤出旗标颜色菜单：ArrowDown / Shift+F10 / 应用键均可，选择后焦点交还按钮', async () => {
    vi.mocked(setReleaseFlag).mockResolvedValue(undefined)
    const wrapper = mountWithMenus(createRelease({ id: 12, flag: 3 }))
    const flagBtn = wrapper.find('.release-flag-action')

    // ArrowDown 唤出：aria-expanded 同步，且含当前标记色的「取消标记」项
    await flagBtn.trigger('keydown', { key: 'ArrowDown' })
    expect(flagBtn.attributes('aria-expanded')).toBe('true')
    expect(wrapper.findAll('.stub-menu-item').map(i => i.text())).toContain(t('release.flag_clear'))

    // 键盘路径选「取消标记」→ 写 0；菜单关闭后焦点回到按钮
    const labels = wrapper.findAll('.stub-menu-item').map(i => i.text())
    await wrapper.findAll('.stub-menu-item')[labels.indexOf(t('release.flag_clear'))].trigger('click')
    await new Promise(resolve => setTimeout(resolve, 0))
    expect(setReleaseFlag).toHaveBeenCalledWith(12, 0)
    expect(flagBtn.attributes('aria-expanded')).toBe('false')

    // Shift+F10 与应用键（ContextMenu）同样可唤出；props.flag 未变（真实刷新靠 emit update），菜单仍含「取消标记」
    await flagBtn.trigger('keydown', { key: 'F10', shiftKey: true })
    expect(wrapper.findAll('.stub-menu-item')).toHaveLength(7) // 六色 + 取消标记

    // 在键盘唤出的菜单里选色同样生效（第一项 = 红 1）
    await wrapper.findAll('.stub-menu-item')[0].trigger('click')
    await new Promise(resolve => setTimeout(resolve, 0))
    expect(setReleaseFlag).toHaveBeenCalledWith(12, 1)
  })

  it('已标记时旗标菜单追加「取消旗标」项，点击清除', async () => {
    vi.mocked(setReleaseFlag).mockResolvedValue(undefined)
    const wrapper = mountWithMenus(createRelease({ id: 11, flag: 5 }))

    await wrapper.find('.release-flag-action').trigger('contextmenu')
    const labels = wrapper.findAll('.stub-menu-item').map(i => i.text())
    expect(labels).toContain(t('release.flag_clear'))

    // 顺序：flag-1..6（索引 0-5）, flag-clear（索引 6）
    await wrapper.findAll('.stub-menu-item')[6].trigger('click')
    await new Promise(resolve => setTimeout(resolve, 0))

    expect(setReleaseFlag).toHaveBeenCalledWith(11, 0)
  })
})
