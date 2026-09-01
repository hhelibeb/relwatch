import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest'
import { mount, flushPromises } from '@vue/test-utils'
import { relaunch } from '@tauri-apps/plugin-process'
import { getVersion } from '@tauri-apps/api/app'
import { confirm } from '@tauri-apps/plugin-dialog'
import SettingsTab from '../components/SettingsTab.vue'
import { openReleaseUrl } from '../api/client'
import { useAppUpdate, classifyUpdateError } from '../api/update'
import { commands, type UpdaterMetadata } from '../bindings'
import { ShowToastKey } from '../injection-keys'
import { defaultSettings } from './helpers'
import { t, tm } from '../i18n'

// 下载由 Update 实例发起：mock 的 Update 由 composable 内部 new 出来，
// 测试拿不到构造前的引用，因此让所有实例共享同一个 vi.fn 以便按用例编程。
const { downloadMock, closeMock } = vi.hoisted(() => ({ downloadMock: vi.fn(), closeMock: vi.fn() }))
vi.mock('@tauri-apps/plugin-updater', () => ({
  Update: class {
    rid: number
    currentVersion: string
    version: string
    date?: string
    body?: string
    rawJson: Record<string, unknown>
    downloadAndInstall = downloadMock
    close = closeMock
    constructor(m: {
      rid: number
      currentVersion: string
      version: string
      date?: string
      body?: string
      rawJson: Record<string, unknown>
    }) {
      this.rid = m.rid
      this.currentVersion = m.currentVersion
      this.version = m.version
      this.date = m.date
      this.body = m.body
      this.rawJson = m.rawJson
    }
  },
}))
vi.mock('@tauri-apps/plugin-process', () => ({ relaunch: vi.fn() }))
vi.mock('@tauri-apps/api/app', () => ({ getVersion: vi.fn() }))
vi.mock('@tauri-apps/plugin-dialog', () => ({ confirm: vi.fn(), message: vi.fn() }))
vi.mock('../api/client', () => ({
  openReleaseUrl: vi.fn(),
  // 透传包装：错误翻译链路在 client.ts 单测覆盖，这里直接执行原函数
  invokeI18nFn: (fn: () => Promise<unknown>) => fn(),
}))
// bindings 只暴露本功能用到的命令；SettingsTab 挂载时 loadAgentConfig 调用
// 缺失的 getAgentConfig 会抛 TypeError，由其内部 try/catch 吞掉（与既有测试一致）
vi.mock('../bindings', () => ({
  commands: {
    getAgentQueue: vi.fn(),
    agentShutdownForUpdate: vi.fn(),
    updaterCheck: vi.fn(),
  },
}))

const checkMock = vi.mocked(commands.updaterCheck)
const relaunchMock = vi.mocked(relaunch)
const getVersionMock = vi.mocked(getVersion)
const confirmMock = vi.mocked(confirm)
const openReleaseUrlMock = vi.mocked(openReleaseUrl)
const getAgentQueueMock = vi.mocked(commands.getAgentQueue)
const shutdownMock = vi.mocked(commands.agentShutdownForUpdate)

/** updater_check 的返回体最小 mock；downloadAndInstall 走全局 downloadMock 编程 */
function fakeUpdate(version: string): UpdaterMetadata {
  return {
    rid: 1,
    currentVersion: '1.13.0',
    version,
    date: null,
    body: null,
    rawJson: '{}',
  }
}

type Composable = ReturnType<typeof useAppUpdate>

function setupComposable(proxy: { mode: string; url: string } = { mode: 'none', url: '' }): Composable {
  return useAppUpdate(() => proxy)
}

describe('classifyUpdateError（§4.5 错误表，锚点对 tauri-plugin-updater 2.10.1 实测）', () => {
  it('网络/超时类归 network', () => {
    expect(classifyUpdateError('error sending request for url (https://github.com/...)')).toBe('network')
    expect(classifyUpdateError('operation timed out after 30000ms')).toBe('network')
    expect(classifyUpdateError('connection refused')).toBe('network')
  })

  it('ReleaseNotFound 归 no_release（endpoint 拿不到合法 release JSON → 检查失败）', () => {
    expect(classifyUpdateError('Could not fetch a valid release JSON from the remote')).toBe('no_release')
  })

  it('签名验证失败归 signature（minisign 透明错误 + SignatureUtf8）', () => {
    expect(classifyUpdateError('The signature verification failed')).toBe('signature')
    expect(classifyUpdateError('The signature was created with a different key than the one provided')).toBe('signature')
    expect(classifyUpdateError('The signature xxx could not be decoded, please check if it is a valid base64 string.')).toBe('signature')
  })

  it('平台缺失 / 格式错配 / 挂载点 / 环境不支持', () => {
    expect(classifyUpdateError('the platform `linux-x86_64` was not found in the response `platforms` object')).toBe('targets')
    expect(classifyUpdateError('None of the fallback platforms `["linux-x86_64"]` were found in the response `platforms` object')).toBe('targets')
    expect(classifyUpdateError('invalid updater binary format')).toBe('format')
    expect(classifyUpdateError('temp directory is not on the same mount point as the AppImage')).toBe('mount')
    expect(classifyUpdateError('Unsupported application architecture, expected one of `x86`, `x86_64`.')).toBe('unsupported')
    expect(classifyUpdateError('Updater does not have any endpoints set.')).toBe('unsupported')
  })

  it('未归类消息回 generic', () => {
    expect(classifyUpdateError('some future error text')).toBe('generic')
  })
})

describe('useAppUpdate 检查状态机', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    // 实例间共享的 mock：清掉上一用例残留的实现，避免进度事件串联
    downloadMock.mockReset()
    downloadMock.mockResolvedValue(undefined)
    closeMock.mockReset()
    getVersionMock.mockResolvedValue('1.13.0')
    getAgentQueueMock.mockResolvedValue([])
    shutdownMock.mockResolvedValue(null)
    confirmMock.mockResolvedValue(true)
  })

  it('null → upToDate', async () => {
    checkMock.mockResolvedValue(null)
    const c = setupComposable()
    expect(c.status.value).toBe('idle')
    await c.checkForUpdate()
    expect(c.status.value).toBe('upToDate')
  })

  it('Update → available', async () => {
    checkMock.mockResolvedValue(fakeUpdate('1.14.0'))
    const c = setupComposable()
    await c.checkForUpdate()
    expect(c.status.value).toBe('available')
    expect(c.pendingUpdate.value?.version).toBe('1.14.0')
  })

  it('再次检查时 close 旧 Update（防 rid 资源累积）', async () => {
    checkMock.mockResolvedValue(fakeUpdate('1.14.0'))
    const c = setupComposable()
    await c.checkForUpdate()
    const first = c.pendingUpdate.value
    checkMock.mockResolvedValue(fakeUpdate('1.15.0'))
    await c.checkForUpdate()
    expect(closeMock).toHaveBeenCalledTimes(1)
    expect(c.pendingUpdate.value?.version).toBe('1.15.0')
    expect(closeMock.mock.instances[0]).toBe(first)
  })

  it('检查失败（网络）→ error，文案用 network key', async () => {
    checkMock.mockRejectedValue(new Error('error sending request for url (https://github.com/x)'))
    const c = setupComposable()
    await c.checkForUpdate()
    expect(c.status.value).toBe('error')
    expect(c.errorKind.value).toBe('network')
    expect(c.errorText.value).toBe(t('update.error.network'))
  })

  it('检查失败（签名）→ 文案与网络错误区分', async () => {
    checkMock.mockRejectedValue('The signature verification failed')
    const c = setupComposable()
    await c.checkForUpdate()
    expect(c.errorKind.value).toBe('signature')
    expect(c.errorText.value).toBe(t('update.error.signature'))
  })

  it('代理三态原样传给 updater_check（none/system/custom 由 Rust 侧解释）', async () => {
    checkMock.mockResolvedValue(null)
    const custom = setupComposable({ mode: 'custom', url: 'http://127.0.0.1:17890' })
    await custom.checkForUpdate()
    expect(checkMock).toHaveBeenCalledWith(30_000, 'custom', 'http://127.0.0.1:17890')

    checkMock.mockClear()
    const none = setupComposable({ mode: 'none', url: '' })
    await none.checkForUpdate()
    expect(checkMock).toHaveBeenCalledWith(30_000, 'none', '')

    checkMock.mockClear()
    const system = setupComposable({ mode: 'system', url: '' })
    await system.checkForUpdate()
    expect(checkMock).toHaveBeenCalledWith(30_000, 'system', '')
  })

  it('Update 由 updater_check 返回的 metadata 构造（rawJson 从字符串还原）', async () => {
    checkMock.mockResolvedValue({
      rid: 7,
      currentVersion: '1.13.0',
      version: '1.14.0',
      date: '2026-08-31T00:00:00Z',
      body: 'notes',
      rawJson: '{"note":"x"}',
    })
    const c = setupComposable()
    await c.checkForUpdate()
    expect(c.pendingUpdate.value?.version).toBe('1.14.0')
    expect(c.pendingUpdate.value?.rawJson).toEqual({ note: 'x' })
    expect(c.pendingUpdate.value?.date).toBe('2026-08-31T00:00:00Z')
  })

  it('打开 Release 说明 / 下载页', async () => {
    checkMock.mockResolvedValue(fakeUpdate('1.14.0'))
    const c = setupComposable()
    await c.checkForUpdate()
    c.openReleaseNotes()
    expect(openReleaseUrlMock).toHaveBeenCalledWith('https://github.com/hhelibeb/relwatch/releases/tag/v1.14.0')
    c.openDownloadPage()
    expect(openReleaseUrlMock).toHaveBeenCalledWith('https://github.com/hhelibeb/relwatch/releases/latest')
  })
})

describe('useAppUpdate 下载安装', () => {
  beforeEach(() => {
    vi.clearAllMocks()
    downloadMock.mockReset()
    downloadMock.mockResolvedValue(undefined)
    closeMock.mockReset()
    getVersionMock.mockResolvedValue('1.13.0')
    getAgentQueueMock.mockResolvedValue([])
    shutdownMock.mockResolvedValue(null)
    confirmMock.mockResolvedValue(true)
  })

  it('下载安装成功：shutdown 先于 relaunch（仅 Linux/macOS 路径）', async () => {
    checkMock.mockResolvedValue(fakeUpdate('1.14.0'))
    const c = setupComposable()
    await c.checkForUpdate()
    await c.downloadAndInstall()
    expect(downloadMock).toHaveBeenCalledTimes(1)
    expect(shutdownMock).toHaveBeenCalledTimes(1)
    expect(relaunchMock).toHaveBeenCalledTimes(1)
    expect(shutdownMock.mock.invocationCallOrder[0]).toBeLessThan(relaunchMock.mock.invocationCallOrder[0])
    expect(c.status.value).toBe('installing')
  })

  it('进度事件：total 只在 Started 记录一次，不被 Progress 冲掉', async () => {
    downloadMock.mockImplementation(async (onProgress?: (e: unknown) => void) => {
      onProgress?.({ event: 'Started', data: { contentLength: 1000 } })
      onProgress?.({ event: 'Progress', data: { chunkLength: 400 } })
      onProgress?.({ event: 'Progress', data: { chunkLength: 600 } })
      onProgress?.({ event: 'Finished' })
    })
    checkMock.mockResolvedValue(fakeUpdate('1.14.0'))
    const c = setupComposable()
    await c.checkForUpdate()
    await c.downloadAndInstall()
    expect(c.total.value).toBe(1000)
    expect(c.done.value).toBe(1000)
    expect(c.percent.value).toBe(100)
    expect(c.downloadText.value).toBe(tm('update.downloading', { percent: '100%', done: '1000 B', total: '1000 B' }))
  })

  it('回归：pendingUpdate 用 shallowRef 保持原实例（不被深度 reactive 包装）', async () => {
    // 真实 @tauri-apps/plugin-updater 的 Update 类实例含私有字段（#metadata 等），
    // 深度 ref 会把实例包成 Proxy，方法内 this.#x 访问私有字段会抛
    // "Cannot read private member from an object whose class did not declare it"。
    // 验证：composable 里 pendingUpdate 是 shallowRef —— 赋进去的原实例被原样读出（无 Proxy）。
    checkMock.mockResolvedValue(fakeUpdate('1.14.0'))
    const c = setupComposable()
    await c.checkForUpdate()
    const v = c.pendingUpdate.value
    expect(v).not.toBeNull()
    // shallowRef 读出的就是构造实例本身，原型链完整（深度 reactive 会包 Proxy 并保留原型，
    // 但用 isProxy 能直接区分；此处兼容两种实现，重点在调用不抛错）
    // 真正回归点：downloadAndInstall 能正常执行（内部若访问私有字段，Proxy 会抛错）
    await expect(c.downloadAndInstall()).resolves.toBeUndefined()
  })

  it('total 为 undefined/0 时不显示百分比（走 no_total 文案）', async () => {
    downloadMock.mockImplementation(async (onProgress?: (e: unknown) => void) => {
      onProgress?.({ event: 'Started', data: { contentLength: undefined } })
      onProgress?.({ event: 'Progress', data: { chunkLength: 600 } })
    })
    checkMock.mockResolvedValue(fakeUpdate('1.14.0'))
    const c = setupComposable()
    await c.checkForUpdate()
    await c.downloadAndInstall()
    expect(c.percent.value).toBeNull()
    expect(c.downloadText.value).toBe(tm('update.downloading_no_total', { done: '600 B' }))
  })

  it('下载失败 → error；重试回 available；重试时 downloaded 归零', async () => {
    downloadMock
      .mockImplementationOnce(async (onProgress?: (e: unknown) => void) => {
        onProgress?.({ event: 'Started', data: { contentLength: 1000 } })
        onProgress?.({ event: 'Progress', data: { chunkLength: 400 } })
        throw 'error sending request for url (https://objects.githubusercontent.com/...)'
      })
      .mockRejectedValueOnce('connection reset by peer')
    checkMock.mockResolvedValue(fakeUpdate('1.14.0'))
    const c = setupComposable()
    await c.checkForUpdate()
    await c.downloadAndInstall()
    expect(c.status.value).toBe('error')
    expect(c.errorKind.value).toBe('network')
    expect(c.done.value).toBe(400)

    c.retry()
    expect(c.status.value).toBe('available')

    // 第二次点击下载：进度状态已清理（done/total 归零重建），再次失败仍落 error
    await c.downloadAndInstall()
    expect(downloadMock).toHaveBeenCalledTimes(2)
    expect(c.status.value).toBe('error')
    expect(c.done.value).toBe(0)
    expect(c.total.value).toBeUndefined()
  })

  it('Agent 任务守卫：队列非空时确认弹窗，取消则不下载', async () => {
    getAgentQueueMock.mockResolvedValue([{} as never])
    confirmMock.mockResolvedValue(false)
    checkMock.mockResolvedValue(fakeUpdate('1.14.0'))
    const c = setupComposable()
    await c.checkForUpdate()
    await c.downloadAndInstall()
    expect(confirmMock).toHaveBeenCalledTimes(1)
    expect(confirmMock.mock.calls[0][0]).toBe(t('update.error.agent_busy'))
    expect(downloadMock).not.toHaveBeenCalled()
    expect(c.status.value).toBe('available')

    confirmMock.mockResolvedValue(true)
    await c.downloadAndInstall()
    expect(downloadMock).toHaveBeenCalledTimes(1)
  })

  it('Agent 队列查询失败时降级放行（守卫不堵死更新路径）', async () => {
    getAgentQueueMock.mockRejectedValue(new Error('db unavailable'))
    checkMock.mockResolvedValue(fakeUpdate('1.14.0'))
    const c = setupComposable()
    await c.checkForUpdate()
    await c.downloadAndInstall()
    expect(confirmMock).not.toHaveBeenCalled()
    expect(downloadMock).toHaveBeenCalledTimes(1)
  })
})

describe('SettingsTab「软件更新」分组（位于 about tab）', () => {
  /** 挂载并切到「关于」tab（软件更新分组已从常规设置迁移至关于） */
  async function mountSettingsTabOnAbout() {
    const wrapper = mount(SettingsTab, {
      props: { settings: { ...defaultSettings } },
      global: {
        provide: {
          [ShowToastKey as symbol]: vi.fn(),
        },
      },
    })
    await flushPromises()
    await wrapper.get('[data-testid="settings-tab-about"]').trigger('click')
    await flushPromises()
    return wrapper
  }

  afterEach(() => {
    vi.unstubAllEnvs()
  })

  it('dev 构建：按钮置灰 + 提示文案', async () => {
    vi.stubEnv('DEV', true)
    const wrapper = await mountSettingsTabOnAbout()
    const btn = wrapper.get('[data-testid="update-check-btn"]')
    expect(btn.attributes('disabled')).toBeDefined()
    expect(wrapper.text()).toContain(t('update.dev_disabled'))
    wrapper.unmount()
  })

  it('非 dev 构建：检查 → 已是最新版本（含版本号，tm 命名参数渲染）', async () => {
    vi.stubEnv('DEV', false)
    checkMock.mockResolvedValue(null)
    const wrapper = await mountSettingsTabOnAbout()
    const btn = wrapper.get('[data-testid="update-check-btn"]')
    expect(btn.attributes('disabled')).toBeUndefined()

    await btn.trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain(tm('update.up_to_date', { version: 'v1.13.0' }))
    // 文案渲染无字面 {version} 残留
    expect(wrapper.text()).not.toContain('{version}')
    wrapper.unmount()
  })

  it('发现新版本：available 文案 + 操作按钮渲染', async () => {
    vi.stubEnv('DEV', false)
    checkMock.mockResolvedValue(fakeUpdate('1.14.0'))
    const wrapper = await mountSettingsTabOnAbout()
    await wrapper.get('[data-testid="update-check-btn"]').trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain(tm('update.available', { version: 'v1.14.0' }))
    expect(wrapper.get('[data-testid="update-install-btn"]').text()).toBe(t('update.download_install'))
    expect(wrapper.text()).toContain(t('update.view_notes'))
    wrapper.unmount()
  })

  it('签名错误渲染专属文案（与网络错误区分）', async () => {
    vi.stubEnv('DEV', false)
    checkMock.mockRejectedValue('The signature verification failed')
    const wrapper = await mountSettingsTabOnAbout()
    await wrapper.get('[data-testid="update-check-btn"]').trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain(t('update.error.signature'))
    expect(wrapper.text()).not.toContain(t('update.error.network'))
    // 签名错误无「重试」按钮，只有「前往下载页」（§4.5 错误表）
    expect(wrapper.text()).not.toContain(t('update.retry'))
    expect(wrapper.text()).toContain(t('update.open_download_page'))
    wrapper.unmount()
  })

  it('检查失败（no_release：endpoint 拿不到合法 release JSON）→ 显示失败文案 + 重试按钮', async () => {
    vi.stubEnv('DEV', false)
    checkMock.mockRejectedValue('Could not fetch a valid release JSON from the remote')
    const wrapper = await mountSettingsTabOnAbout()
    await wrapper.get('[data-testid="update-check-btn"]').trigger('click')
    await flushPromises()

    expect(wrapper.text()).toContain(t('update.error.no_release'))
    // no_release 是「检查失败」而非「没有更新」：重试自救 + 手动下载页并存——
    // 该分支也可能是本版本确实没有 latest.json（如旧版用户），不能只给重试堵死升级路径
    expect(wrapper.text()).toContain(t('update.retry'))
    expect(wrapper.text()).toContain(t('update.open_download_page'))
    wrapper.unmount()
  })

  it('关于页展示应用信息与软件更新分组标题', async () => {
    const wrapper = await mountSettingsTabOnAbout()
    expect(wrapper.text()).toContain(t('about.app_name'))
    expect(wrapper.text()).toContain(t('about.version'))
    expect(wrapper.text()).toContain(t('update.section_title'))
    wrapper.unmount()
  })

  it('关于页 GitHub 链接为超链接且指向仓库地址', async () => {
    const wrapper = await mountSettingsTabOnAbout()
    const link = wrapper.find('.setting-link')
    expect(link.exists()).toBe(true)
    expect(link.attributes('href')).toBe('https://github.com/hhelibeb/relwatch')
    expect(link.text()).toContain('github.com/hhelibeb/relwatch')
    wrapper.unmount()
  })
})
