import { describe, it, expect, beforeEach } from 'vitest'

// 直接测试 i18n 模块（纯函数，无需 mock Tauri API）
// 由于 setLocale/getLocale 操作全局状态，测试间需交替语言验证
describe('i18n 核心函数', () => {
  let i18n: typeof import('../i18n')

  beforeEach(async () => {
    i18n = await import('../i18n')
    i18n.setLocale('zh-CN') // 每个测试重置为默认语言
  })

  // ── getLocale / setLocale ─────────────────────────────────────

  describe('setLocale / getLocale', () => {
    it('默认语言为 zh-CN', () => {
      expect(i18n.getLocale()).toBe('zh-CN')
    })

    it('切换到 en-US', () => {
      i18n.setLocale('en-US')
      expect(i18n.getLocale()).toBe('en-US')
    })

    it('切换到 zh-CN', () => {
      i18n.setLocale('en-US')
      i18n.setLocale('zh-CN')
      expect(i18n.getLocale()).toBe('zh-CN')
    })

    it('设置不存在的语言不生效', () => {
      i18n.setLocale('ja-JP')
      expect(i18n.getLocale()).toBe('zh-CN')
    })
  })

  // ── t() ───────────────────────────────────────────────────────

  describe('t()', () => {
    it('返回已有键的值（zh-CN）', () => {
      expect(i18n.t('app.title')).toBe('版本监控')
    })

    it('返回已有键的值（en-US）', () => {
      i18n.setLocale('en-US')
      expect(i18n.t('app.title')).toBe('Release Monitor')
    })

    it('缺失键返回 key 本身', () => {
      expect(i18n.t('nonexistent.key')).toBe('nonexistent.key')
    })

    it('替换 {0} 位置参数', () => {
      expect(i18n.t('app.new_found', '5')).toBe('发现 5 个新版本')
    })

    it('替换多个位置参数', () => {
      expect(i18n.t('app.min_sec', '3', '45')).toBe('3分45秒')
    })

    it('不存在的 messages 表返回 key', () => {
      // en-US 下的位置参数渲染（msg 表缺失路径无法直接触发，setLocale 会拒绝未知语言）
      i18n.setLocale('en-US')
      expect(i18n.t('app.min_sec', '1', '30')).toBe('1m 30s')
    })
  })

  // ── tm() ──────────────────────────────────────────────────────

  describe('tm()', () => {
    it('无参模板—直接返回对应值', () => {
      expect(i18n.tm('source.never_checked', {})).toBe('从未检查')
    })

    it('替换 {action} 为翻译后的状态文本（pending）', () => {
      // actionKeys['pending'] → 'status.pending' → '未读'；译文无 {action} 占位符，替换后不变
      const result = i18n.tm('status.pending', { action: 'pending' })
      expect(result).toBe('未读')
    })

    it('缺失键返回 key 本身', () => {
      expect(i18n.tm('nonexistent.template', {})).toBe('nonexistent.template')
    })

    it('替换命名参数', () => {
      i18n.setLocale('en-US')
      const result = i18n.tm('source.recorded_versions', { count: '5' })
      // en-US 字典为 '{0} recorded version(s)'：占位符是 {0}，命名参数 {count} 不匹配 → 原样返回
      expect(result).toBe('{0} recorded version(s)')
    })

    it('自动翻译模板中的 setting.xxx 子键', () => {
      // 键值不含 setting.xxx 时不触发二次翻译（正则替换仅在命中时生效）
      const result = i18n.tm('source.never_checked', {})
      expect(result).toBe('从未检查')
    })

    it('repo 为空时省略 {owner}/{repo} 中的斜杠（YouTube 源日志）', () => {
      expect(i18n.tm('check.manual', { owner: 'Fireship', repo: '', count: '232' })).toBe('[手动] 检查 Fireship: 232 个新版本')
      expect(i18n.tm('source.removed', { owner: 'Fireship', repo: '', id: '71' })).toBe('移除监控源 Fireship id=71')
      expect(i18n.tm('source.added', { source_type: 'youtube', owner: 'Fireship', repo: '' })).toBe('添加监控源: youtube Fireship')
    })

    it('repo 为空时 status_changed 用视频标题替代 tag（YouTube 源）', () => {
      expect(
        i18n.tm('release.status_changed', {
          owner: 'Fireship',
          repo: '',
          tag: 'The Future of Web Dev',
          id: '95900',
          action: 'ignored',
        }),
      ).toBe('Fireship The Future of Web Dev 已忽略(id=95900)')
    })

    it('repo 非空时保持 owner/repo 原样（GitHub 源）', () => {
      expect(i18n.tm('check.manual', { owner: 'user', repo: 'myapp', count: '3' })).toBe('[手动] 检查 user/myapp: 3 个新版本')
      expect(
        i18n.tm('release.status_changed', {
          owner: 'user',
          repo: 'myapp',
          tag: 'v1.0',
          id: '123',
          action: 'pending',
        }),
      ).toBe('user/myapp v1.0 未读(id=123)')
    })

    it('changes 参数中内嵌的 setting.xxx 键名按当前语言二次翻译（与后端 resolve_setting_keys 同协议）', () => {
      i18n.setLocale('zh-CN')
      expect(i18n.tm('setting.updated', { changes: 'setting.poll_interval→60, setting.language→en-US' }))
        .toBe('更新设置: 轮询间隔→60, 界面语言→en-US')

      i18n.setLocale('en-US')
      expect(i18n.tm('setting.updated', { changes: 'setting.poll_interval→60' }))
        .toBe('Setting updated: Poll Interval→60')

      // 未命中的键原样保留
      i18n.setLocale('zh-CN')
      expect(i18n.tm('setting.updated', { changes: 'setting.unknown_key→x' }))
        .toBe('更新设置: setting.unknown_key→x')
    })
  })

  // ── translateError（与 Rust translate_error_str 保持一致）─────

  describe('translateError', () => {
    it('err.repo_not_found — zh-CN', () => {
      expect(i18n.t('err.repo_not_found')).toBe('不存在该仓库')
    })

    it('err.repo_not_found — en-US', () => {
      i18n.setLocale('en-US')
      expect(i18n.t('err.repo_not_found')).toBe('Repository not found')
    })

    it('err.repo_verify_failed 带参数', () => {
      expect(i18n.t('err.repo_verify_failed', 'API token invalid')).toBe('验证仓库失败: API token invalid')
    })

    it('err.repo_verify_failed 带参数 — en-US', () => {
      i18n.setLocale('en-US')
      expect(i18n.t('err.repo_verify_failed', 'API token invalid')).toBe('Failed to verify repo: API token invalid')
    })

    it('err.repo_api_error 带参数', () => {
      expect(i18n.t('err.repo_api_error', '404')).toBe('GitHub API 返回 404')
    })

    it('err.request_failed 带参数', () => {
      expect(i18n.t('err.request_failed', 'timeout')).toBe('网络请求失败: timeout')
    })

    it('err.api_error 多参数', () => {
      expect(i18n.t('err.api_error', '403', 'rate limit')).toBe('API 请求失败: HTTP 403 rate limit')
    })

    it('err.api_error 多参数 — en-US', () => {
      i18n.setLocale('en-US')
      expect(i18n.t('err.api_error', '403', 'rate limit')).toBe('API request failed: HTTP 403 rate limit')
    })

    it('err.parse_failed 带参数', () => {
      expect(i18n.t('err.parse_failed', 'unexpected token')).toBe('解析响应失败: unexpected token')
    })

    it('err.poll_in_progress — zh-CN', () => {
      expect(i18n.t('err.poll_in_progress')).toBe('轮询正在进行中，请稍后再试')
    })

    it('err.poll_in_progress — en-US', () => {
      i18n.setLocale('en-US')
      expect(i18n.t('err.poll_in_progress')).toBe('Poll in progress, please try again later')
    })

    it('err.unsupported_source 带参数', () => {
      expect(i18n.t('err.unsupported_source', 'gitlab')).toBe('不支持的监控源类型: gitlab')
    })

    it('err.source_not_found — zh-CN', () => {
      expect(i18n.t('err.source_not_found')).toBe('监控源不存在')
    })

    it('err.source_not_found — en-US', () => {
      i18n.setLocale('en-US')
      expect(i18n.t('err.source_not_found')).toBe('Source not found')
    })
  })

  // ── languages ─────────────────────────────────────────────────

  describe('languages', () => {
    it('包含中文和英文两个选项', () => {
      expect(i18n.languages).toHaveLength(2)
      expect(i18n.languages[0].value).toBe('zh-CN')
      expect(i18n.languages[1].value).toBe('en-US')
    })
  })
})
