import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount } from '@vue/test-utils'
import { defineComponent, nextTick, ref } from 'vue'
import { t } from '../i18n'
import { useAgentComposer } from '../components/agent/useAgentComposer'
import { open } from '@tauri-apps/plugin-dialog'
import type { Source } from '../api/sources'
import type { ReleaseInfo } from '../api/releases'

vi.mock('@tauri-apps/plugin-dialog', () => ({
  confirm: vi.fn(),
  open: vi.fn(),
}))

const SKILLS = [
  'E:\\project\\relwatch\\.pi\\skills\\commit\\SKILL.md',
  'E:\\project\\relwatch\\.pi\\skills\\release\\SKILL.md',
]

const SOURCES: Source[] = [
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
]

const RELEASES: ReleaseInfo[] = [
  {
    id: 7,
    source_id: 1,
    source_type: 'youtube',
    owner: 'UCrD39DnkX5QjIvH3yssXqJA',
    repo: '8Pi_1HjBUPU',
    tag_name: '8Pi_1HjBUPU',
    release_name: '白袜轻蹭耳朵柔和触发音',
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
    source_description: '宁静ASMR频道',
    flag: 0,
    version_bump: null,
  },
]

// 挂到 document 上：焦点断言（focusAtEnd / replaceTrigger 的 el.focus()）依赖元素已连接
const wrappers: { unmount: () => void }[] = []
function setup() {
  const showToast = vi.fn()
  let api!: ReturnType<typeof useAgentComposer>
  const wrapper = mount(
    defineComponent({
      setup() {
        api = useAgentComposer({
          showToast,
          skills: ref(SKILLS),
          sources: ref(SOURCES),
          releases: ref(RELEASES),
        })
        return { textareaRef: api.textareaRef }
      },
      template: '<div><textarea ref="textareaRef" /></div>',
    }),
    { attachTo: document.body },
  )
  wrappers.push(wrapper)
  return { wrapper, api, showToast, textarea: () => wrapper.find('textarea').element as HTMLTextAreaElement }
}

beforeEach(() => {
  vi.mocked(open).mockReset()
})
afterEach(() => {
  while (wrappers.length) wrappers.pop()?.unmount()
  vi.useRealTimers()
})

describe('useAgentComposer @/[[ 触发词解析', () => {
  it('输入 @ 开 skill 菜单、[[ 开实体菜单、无触发词都关闭', async () => {
    const { api, textarea } = setup()
    const el = textarea()
    const setInput = (v: string) => {
      el.value = v
      el.setSelectionRange(v.length, v.length)
    }
    setInput('@')
    api.handleInput()
    expect(api.showSkillMenu.value).toBe(true)
    expect(api.showEntityMenu.value).toBe(false)

    setInput('[[')
    api.handleInput()
    expect(api.showEntityMenu.value).toBe(true)
    expect(api.showSkillMenu.value).toBe(false)

    setInput('hello')
    api.handleInput()
    expect(api.showSkillMenu.value).toBe(false)
    expect(api.showEntityMenu.value).toBe(false)
  })

  it('filteredSkills：按短名或全路径模糊匹配', async () => {
    const { api, textarea } = setup()
    const el = textarea()
    const query = async (q: string) => {
      el.value = `@${q}`
      el.setSelectionRange(el.value.length, el.value.length)
      api.handleInput()
      await nextTick()
    }
    await query('com')
    expect(api.filteredSkills.value).toEqual([SKILLS[0]])
    await query('release')
    expect(api.filteredSkills.value).toEqual([SKILLS[1]])
    await query('.pi')
    expect(api.filteredSkills.value.length).toBe(2)
  })

  it('filteredSources / filteredReleases：s: / r: 前缀限定类型', async () => {
    const { api, textarea } = setup()
    const el = textarea()
    const query = async (q: string) => {
      el.value = `[[${q}`
      el.setSelectionRange(el.value.length, el.value.length)
      api.handleInput()
      await nextTick()
    }
    await query('')
    expect(api.filteredSourcesCount.value).toBe(1)
    expect(api.filteredReleasesCount.value).toBe(1)
    expect(api.entityMenuHasMatch.value).toBe(true)

    await query('r:白袜')
    expect(api.filteredSources.value).toEqual([])
    expect(api.filteredReleases.value.length).toBe(1)

    await query('s:宁静')
    expect(api.filteredSources.value.length).toBe(1)
    expect(api.filteredReleases.value).toEqual([])

    await query('不存在的')
    expect(api.entityMenuHasMatch.value).toBe(false)
  })
})

describe('useAgentComposer 菜单选择与输入替换', () => {
  it('pickSkill：skillPath 记完整路径，输入框只插入 @短名（替换触发词）', async () => {
    const { api, textarea } = setup()
    const el = textarea()
    el.value = '帮我 @com'
    // 光标在触发词末尾（真实交互：输入 @com 后从菜单选择）
    el.setSelectionRange(el.value.length, el.value.length)
    api.pickSkill(SKILLS[0])
    expect(api.skillPath.value).toBe(SKILLS[0])
    expect(el.value).toBe('帮我 @commit ')
    expect(api.instruction.value).toBe('帮我 @commit ')
    expect(api.showSkillMenu.value).toBe(false)
  })

  it('pickEntity：替换 [[ 触发词为引用标记', () => {
    const { api, textarea } = setup()
    const el = textarea()
    el.value = '看看 [[宁静'
    el.setSelectionRange(7, 7)
    api.pickEntity('source', 1)
    expect(el.value).toBe('看看 [[source:1]] ')
    expect(api.showEntityMenu.value).toBe(false)
  })

  it('clearSkill 清除已选 skill', () => {
    const { api } = setup()
    api.pickSkill(SKILLS[0])
    api.clearSkill()
    expect(api.skillPath.value).toBeNull()
  })

  it('实体可读名：source/release 展示频道 · 标题，chip 展示 类型 | 可读名', () => {
    const { api } = setup()
    expect(api.sourceDisplayName(SOURCES[0])).toBe('宁静ASMR频道')
    expect(api.releaseDisplayName(RELEASES[0])).toBe('宁静ASMR频道 · 白袜轻蹭耳朵柔和触发音')
    expect(api.entityLabel({ kind: 'source', id: 1 })).toContain('宁静ASMR频道')
    expect(api.entityLabel({ kind: 'release', id: 7 })).toBe('宁静ASMR频道 · 白袜轻蹭耳朵柔和触发音')
    expect(api.entityLabel({ kind: 'source', id: 99 })).toBe('source #99') // 目录缺失回退 id
    expect(api.entityKindLabel('source')).toBe(t('agent.entity_source'))
    expect(api.entityKindLabel('release')).toBe(t('agent.entity_release'))
  })
})

describe('useAgentComposer 引用与 flash 反馈', () => {
  it('addEntity 去重：新增返回 true，重复返回 false 且不追加', () => {
    const { api } = setup()
    expect(api.addEntity({ kind: 'source', id: 1 })).toBe(true)
    expect(api.entities.value).toEqual([{ kind: 'source', id: 1 }])
    expect(api.addEntity({ kind: 'source', id: 1 })).toBe(false)
    expect(api.entities.value.length).toBe(1)
  })

  it('flashEntity：chip 高亮 + 播报文案区分「新加入 / 已存在」，1200ms 后复位', async () => {
    vi.useFakeTimers()
    const { api } = setup()
    api.addEntity({ kind: 'release', id: 7 })
    api.afterAttach({ kind: 'release', id: 7 }, true)
    expect(api.flashKey.value).toBe('release:7')
    expect(api.attachAnnouncement.value).toBe(t('agent.attached'))

    api.afterAttach({ kind: 'release', id: 7 }, false)
    expect(api.attachAnnouncement.value).toBe(t('agent.attached_exists'))

    vi.advanceTimersByTime(1200)
    expect(api.flashKey.value).toBeNull()
    expect(api.attachAnnouncement.value).toBe('')
  })

  it('afterAttach 把光标送到输入框末尾（拖完即可打字发送）', async () => {
    const { api, textarea } = setup()
    const el = textarea()
    el.value = '一段文本'
    api.afterAttach({ kind: 'source', id: 1 }, true)
    await nextTick()
    expect(document.activeElement).toBe(el)
    expect(el.selectionStart).toBe(el.value.length)
    expect(el.selectionEnd).toBe(el.value.length)
  })

  it('removeEntity / removeFile 按索引移除', () => {
    const { api } = setup()
    api.addEntity({ kind: 'source', id: 1 })
    api.addEntity({ kind: 'release', id: 2 })
    api.removeEntity(0)
    expect(api.entities.value).toEqual([{ kind: 'release', id: 2 }])
    api.files.value = ['C:/a.log', 'C:/b.log']
    api.removeFile(1)
    expect(api.files.value).toEqual(['C:/a.log'])
  })

  it('closeMenus(excludeTextarea)：点输入框豁免（不收），点其他区域收起', () => {
    const { api } = setup()
    api.showSkillMenu.value = true
    api.showEntityMenu.value = true
    api.closeMenus(true)
    expect(api.showSkillMenu.value).toBe(true)
    expect(api.showEntityMenu.value).toBe(true)
    api.closeMenus(false)
    expect(api.showSkillMenu.value).toBe(false)
    expect(api.showEntityMenu.value).toBe(false)
  })
})

describe('useAgentComposer 本地文件附件', () => {
  it('handleAttachFiles：对话框选择去重追加 + toast；取消不改动', async () => {
    const { api, showToast } = setup()
    vi.mocked(open).mockResolvedValue(['C:/logs/app.log', 'C:/img/shot.png'])
    await api.handleAttachFiles()
    expect(api.files.value).toEqual(['C:/logs/app.log', 'C:/img/shot.png'])
    expect(showToast).toHaveBeenCalledWith(t('agent.file_attached', '2'))

    vi.mocked(open).mockResolvedValue(['C:/logs/app.log', 'C:/new.log'])
    await api.handleAttachFiles()
    expect(api.files.value).toEqual(['C:/logs/app.log', 'C:/img/shot.png', 'C:/new.log'])
    expect(showToast).toHaveBeenLastCalledWith(t('agent.file_attached', '1'))

    vi.mocked(open).mockResolvedValue(null)
    await api.handleAttachFiles()
    expect(api.files.value.length).toBe(3)
  })

  it('fileDisplayName：chip 只显示文件名（兼容反斜杠路径）', () => {
    const { api } = setup()
    expect(api.fileDisplayName('C:/logs/app.log')).toBe('app.log')
    expect(api.fileDisplayName('C:\\logs\\err.log')).toBe('err.log')
    expect(api.fileDisplayName('')).toBe('')
  })
})
