<script setup lang="ts">
import { ref, reactive, inject, watch, computed } from 'vue'
import { ShowToastKey } from '../injection-keys'
import { message, confirm } from '@tauri-apps/plugin-dialog'
import { version } from '../../package.json'
import {
  type AppSettings,
  updateSettings,
  setCredential,
  testDeepseekConnection,
  exportBackup,
  importBackup,
} from '../api/settings'
import { openReleaseUrl } from '../api/client'
import { useAppUpdate } from '../api/update'
import { getAgentConfig, saveAgentConfig } from '../api/agent'
import { t, tm, setLocale, languages } from '../i18n'
import { skillShortName } from '../utils'
import { track } from '../composables/useUsageTracking'
import { applyTheme } from '../composables/useTheme'
import { applyFontScale } from '../composables/useFontScale'
import { usePreviewSelect } from '../composables/usePreviewSelect'
import { useBilibiliLogin } from '../composables/useBilibiliLogin'
import UpdateNotesModal from './UpdateNotesModal.vue'

const props = defineProps<{ settings: AppSettings }>()
const emit = defineEmits<{
  update: [pollIntervalChanged: boolean, forceReload?: boolean]
  agentConfigChanged: []
  /** 更新链路（检查/下载/安装）写完操作日志——宿主重拉日志列表 */
  updateLogWritten: []
}>()
const showToast = inject(ShowToastKey)!

const settingsTab = ref<'general' | 'accounts' | 'data' | 'appearance' | 'ai' | 'agent' | 'about'>('general')
const savingSettings = ref(false)
const deepseekApiKey = ref('')
const githubToken = ref('')
const youtubeApiKey = ref('')
const bilibiliCookie = ref('')

const testingDeepseek = ref(false)
const prevPollInterval = ref(props.settings.poll_interval_minutes)

// ── Agent 分区（独立 Tab：后端全局单例配置，随「保存设置」统一提交）───────
const agentEnabled = ref(false)
const agentType = ref('pi')
const agentBinary = ref('')
const agentModel = ref('')
const agentWorkingDir = ref('')
const agentPromptSuffix = ref('')
const agentTimeout = ref(300)
const agentSkills = ref<string[]>([])
const newAgentSkill = ref('')

async function loadAgentConfig() {
  try {
    const cfg = await getAgentConfig()
    agentEnabled.value = cfg.enabled
    agentType.value = cfg.agent_type
    agentBinary.value = cfg.binary ?? ''
    agentModel.value = cfg.model ?? ''
    agentWorkingDir.value = cfg.working_dir ?? ''
    agentPromptSuffix.value = cfg.prompt_suffix ?? ''
    agentTimeout.value = cfg.timeout_seconds
    agentSkills.value = [...cfg.skills]
    // 刷新已保存基线（脏点判定基准）
    agentSavedSnapshot.value = agentSnapshot()
  } catch {
    // 加载失败保持默认空表单
  }
}

/** 超时秒数归一化：输入框清空时 v-model.number 得 NaN，
 *  Math.max(1, NaN) 仍是 NaN → JSON 序列化为 null → 后端 serde 反序列化报错。 */
function normalizedAgentTimeout(): number {
  const v = agentTimeout.value
  return Number.isFinite(v) ? Math.max(1, Math.floor(v)) : 1
}

/** Agent 分区当前表单的快照（与「保存设置」提交值对齐）。 */
function agentSnapshot(): string {
  return JSON.stringify({
    enabled: agentEnabled.value,
    type: agentType.value.trim() || 'pi',
    binary: agentBinary.value.trim() || null,
    model: agentModel.value.trim() || null,
    wd: agentWorkingDir.value.trim() || null,
    suffix: agentPromptSuffix.value.trim() || null,
    timeout: normalizedAgentTimeout(),
    skills: agentSkills.value,
  })
}

/** 已保存的 Agent 配置基线（loadAgentConfig / 保存成功后刷新）。
 * 初始值取当前表单快照：loadAgentConfig 异步完成前 agentDirty 保持 false，
 * 避免打开设置页瞬间误闪「未保存修改」横幅。 */
const agentSavedSnapshot = ref(agentSnapshot())

/** Agent 分区是否有未保存修改（与其他 Tab 的脏点一致）。 */
const agentDirty = computed(() => agentSnapshot() !== agentSavedSnapshot.value)

function addAgentSkill() {
  const p = newAgentSkill.value.trim()
  if (!p) return
  if (agentSkills.value.includes(p)) {
    showToast(t('agent.skill_duplicated'))
    return
  }
  agentSkills.value.push(p)
  newAgentSkill.value = ''
}

function removeAgentSkill(index: number) {
  agentSkills.value.splice(index, 1)
}

void loadAgentConfig()

// 本地 form 副本用于 v-model 双向绑定，避免直接修改 props
const form = reactive({ ...props.settings })
watch(() => props.settings, (s) => {
  Object.assign(form, s)
}, { deep: true })

// ── B 站一键登录（应用内 WebView 扫码，自动读取 SESSDATA） ────────
const { biliLoginBusy, handleBilibiliLogin, handleClearBilibiliCookie } = useBilibiliLogin({
  showToast,
  onLoginSuccess: () => {
    form.bilibili_cookie_set = true
    bilibiliCookie.value = ''
  },
  onCookieCleared: () => {
    form.bilibili_cookie_set = false
    bilibiliCookie.value = ''
  },
})

// ── 软件更新（about tab 展示型分组：无持久化设置项，不入 TAB_SETTING_KEYS /
//    dirty 徽标；设计稿 §4.3）。代理复用既有 proxy_mode/proxy_url（取已持久化的
//    props.settings，而非表单未保存值——更新行为不应受未保存修改影响）。
const {
  status: updateStatus,
  currentVersion: updateCurrentVersion,
  pendingUpdate: pendingUpdate,
  errorKind: updateErrorKind,
  errorText: updateErrorText,
  percent: updatePercent,
  downloadText: updateDownloadText,
  busy: updateBusy,
  showNotes: showUpdateNotes,
  notesVersion: updateNotesVersion,
  notesDate: updateNotesDate,
  notesBody: updateNotesBody,
  checkForUpdate,
  downloadAndInstall,
  retry: retryUpdate,
  openReleaseNotes: openUpdateNotes,
  openDownloadPage,
} = useAppUpdate(() => ({ mode: props.settings.proxy_mode, url: props.settings.proxy_url.trim() }), () => emit('updateLogWritten'))

// dev 构建置灰：插件不区分 debug/release，dev 下 check() 会真实访问线上 endpoint
// 并允许把正式版装进开发版（设计稿 §4.3 开发构建保护）
const isDevBuild = import.meta.env.DEV
// error 态兜底动作（§4.5 错误表）：network/generic → 重试；no_release → 重试（「检查失败」，重跑 check 自救）；signature/targets/format/mount/unsupported → 无动作
// 注：no_release 是 updater 插件对「endpoint 拿不到合法 release JSON」（404/403/500/JSON 解析失败）的统一归类，
// 语义上是检查失败而非「没有更新」——真正的无更新走 upToDate（绿色 ✓）。
const showUpdateRetry = computed(() =>
  updateErrorKind.value === 'network' || updateErrorKind.value === 'generic' || updateErrorKind.value === 'no_release',
)
const showUpdateDownloadPage = computed(
  // no_release 也保留下载页：它是插件对「endpoint 拿不到合法 release JSON」的统一兜底，
  // 除限流/服务端故障外，也可能是当前版本确实没有 latest.json（旧版用户）——
  // 只给重试会堵死手动升级路径，故重试与下载页并存（两者不互斥）。
  // 仅 unsupported（环境不支持应用内更新）不给下载页：跳过去同样装不了。
  () => updateErrorKind.value !== 'unsupported',
)

// ── 固定提示词后缀（不可编辑）───────────────────────
const DEEPSEEK_PROMPT_SUFFIX = '请严格按以下 JSON 格式返回（不要包含其他内容）：\n{"summary":"简短中文摘要","importance":"大|中|小"}'

const themeOptions = [
  { value: 'system', label: 'settings.theme_system' },
  { value: 'light', label: 'settings.theme_light' },
  { value: 'dark', label: 'settings.theme_dark' },
]

// ── 语言/主题选择（悬停预览）─────────────────────────
// 两份下拉共用 usePreviewSelect 状态机：仅预览/恢复/选中动作不同。

function setLangPreview(val: string) {
  setLocale(val)
}

function selectLang(val: string) {
  form.language = val
  track('settings.lang')
  setLocale(val)
}

const {
  dropdownOpen: langDropdownOpen,
  previewValue: previewLang,
  selectRef: langSelectRef,
  toggle: toggleLangDropdown,
  handleKeydown: handleLangDropdownKeydown,
  clearPreview: clearLangPreview,
} = usePreviewSelect({
  preview: setLangPreview,
  restore: () => setLocale(form.language),
  onSelect: selectLang,
})

function setThemePreview(val: string) {
  applyTheme(val)
}

function selectTheme(val: string) {
  form.theme = val
  track('settings.theme')
  applyTheme(val)
}

const {
  dropdownOpen: themeDropdownOpen,
  previewValue: previewTheme,
  selectRef: themeSelectRef,
  toggle: toggleDropdown,
  handleKeydown: handleThemeDropdownKeydown,
  clearPreview: clearThemePreview,
} = usePreviewSelect({
  preview: setThemePreview,
  restore: () => applyTheme(form.theme),
  onSelect: selectTheme,
})

// ── 字体大小（界面缩放档位）─────────────────────────
// 与主题同样走 usePreviewSelect 复用打开/键盘/外点关闭交互，但刻意不做悬停
// 预览：整窗缩放会把下拉面板移出鼠标下方造成 hover 抖动，仅在确认选中时应用。
// UI 档位从 90 起：存储 clamp 下限 80（见 useFontScale）留作余量，不作为档位
const fontScaleOptions = [90, 100, 110, 125, 150]

function selectFontScale(val: string) {
  form.font_scale = Number(val)
  track('settings.font_scale')
  applyFontScale(form.font_scale)
}

const {
  dropdownOpen: fontScaleDropdownOpen,
  selectRef: fontScaleSelectRef,
  toggle: toggleFontScaleDropdown,
  handleKeydown: handleFontScaleDropdownKeydown,
  clearPreview: clearFontScalePreview,
} = usePreviewSelect({
  preview: () => {},
  restore: () => {},
  onSelect: selectFontScale,
})

// form.font_scale 理论上只会是档位之一（唯一写入点在下拉选中）；DB 被手动改动等
// 极端情况下兜底显示默认档，避免 i18n 缺 key 时把原始键名渲染进触发器
const fontScaleLabel = computed(() =>
  t(`settings.font_scale_${fontScaleOptions.includes(form.font_scale) ? form.font_scale : 100}`),
)


async function handleSave() {
  savingSettings.value = true
  track('settings.save')
  try {
    const s = form
    // 验证提示词
    if (s.deepseek_prompt && !s.deepseek_prompt.includes('{}')) {
      showToast(t('settings.deepseek_prompt_validate_failed'))
      savingSettings.value = false
      return
    }
    // 先验证提示词、持久化主设置，再写敏感凭据：保证 updateSettings 失败时凭据不会被误写入，
    // 避免“凭据已持久化但用户以为整体保存失败”的非原子状态。
    setLocale(form.language)
    // payload 与后端 AppSettings 结构一一对应（snake_case）：字段清单不再单独维护，
    // 新增设置项后端加字段后，TS 类型强制此处补齐。
    await updateSettings({
      poll_interval_minutes: s.poll_interval_minutes,
      proxy_mode: s.proxy_mode,
      proxy_url: s.proxy_url.trim(),
      auto_start: s.auto_start,
      minimize_to_tray: s.minimize_to_tray,
      log_retention_days: s.log_retention_days,
      deepseek_enabled: s.deepseek_enabled,
      deepseek_model: s.deepseek_model.trim() || 'deepseek-v4-flash',
      deepseek_base_url: s.deepseek_base_url.trim() || 'https://api.deepseek.com',
      deepseek_api_key_set: s.deepseek_api_key_set,
      deepseek_proxy_bypass: s.deepseek_proxy_bypass,
      deepseek_prompt: s.deepseek_prompt,
      deepseek_min_importance: s.deepseek_min_importance,
      deepseek_translate_release: s.deepseek_translate_release,

      check_prereleases: s.check_prereleases,
      fetch_history: s.fetch_history,
      fetch_history_count: s.fetch_history_count ?? 1,
      language: s.language,
      theme: s.theme,
      font_scale: s.font_scale,
      show_source_type_icons: s.show_source_type_icons,
      enable_usage_stats: s.enable_usage_stats,
      github_token_set: s.github_token_set,
      youtube_api_key_set: s.youtube_api_key_set,
      bilibili_cookie_set: s.bilibili_cookie_set,
    })
    // 主设置持久化成功后再写凭据；若凭据写入失败，走外层 catch 提示 save_failed，
    // 此时主设置已存、凭据未存，用户可重试凭据。
    // 四个 set_* 命令已合并为 setCredential(kind, value)（M2）。
    if (deepseekApiKey.value) {
      await setCredential('deepseek_api_key', deepseekApiKey.value)
      deepseekApiKey.value = ''
      form.deepseek_api_key_set = true
    }
    if (githubToken.value) {
      await setCredential('github_token', githubToken.value)
      githubToken.value = ''
      form.github_token_set = true
    }
    if (youtubeApiKey.value) {
      await setCredential('youtube_api_key', youtubeApiKey.value)
      youtubeApiKey.value = ''
      form.youtube_api_key_set = true
    }
    if (bilibiliCookie.value) {
      await setCredential('bilibili_cookie', bilibiliCookie.value)
      bilibiliCookie.value = ''
      form.bilibili_cookie_set = true
    }
    // Agent 配置（独立 Tab，与主设置共用「保存设置」按钮统一提交）：
    // 有未保存修改才写库；失败走外层 catch（主设置已存、Agent 未存，部分成功语义与凭据一致）
    if (agentDirty.value) {
      await saveAgentConfig({
        enabled: agentEnabled.value,
        agent_type: agentType.value.trim() || 'pi',
        binary: agentBinary.value.trim() || null,
        model: agentModel.value.trim() || null,
        working_dir: agentWorkingDir.value.trim() || null,
        prompt_suffix: agentPromptSuffix.value.trim() || null,
        timeout_seconds: normalizedAgentTimeout(),
        skills: agentSkills.value,
      })
      await loadAgentConfig() // 刷新脏点基线
      // 通知全局刷新 agentEnabled（版本列表/监控源的唤起按钮与拖拽立即生效）
      emit('agentConfigChanged')
    }
    showToast(t('settings.saved'))
    const pollChanged = form.poll_interval_minutes !== prevPollInterval.value
    if (pollChanged) prevPollInterval.value = form.poll_interval_minutes
    emit('update', pollChanged)
  } catch (e: unknown) {
    // updateSettings 失败时回滚「选中即应用」的项到已持久化值，避免实际生效状态
    // 与后端不一致（缩放还连带窗口物理尺寸，残留影响最大）
    setLocale(props.settings.language)
    applyTheme(props.settings.theme)
    applyFontScale(props.settings.font_scale)
    showToast(t('settings.save_failed') + (e instanceof Error ? e.message : String(e)))
  } finally {
    savingSettings.value = false
    // dirty 标记会在 props.settings 更新后自动清除
  }
}

// ── 脏标记 ────────────────────────────────────────────
// 通用比对：遍历 form 全部键（即 AppSettings 全部字段），不再手工维护
// trackedKeys 清单——新增设置项自动纳入脏检查（M2）。
// tab → 设置字段清单（dirty 徽标按 tab 聚合；accounts 为凭据输入键）：
// 新增设置项若属已有 tab，需在此登记；属新 tab 时新增一行。
const TAB_SETTING_KEYS: Record<'general' | 'accounts' | 'appearance' | 'ai', readonly string[]> = {
  general: ['auto_start', 'poll_interval_minutes', 'proxy_mode', 'proxy_url', 'log_retention_days', 'check_prereleases', 'fetch_history', 'fetch_history_count', 'enable_usage_stats'],
  accounts: ['github_token', 'youtube_api_key', 'bilibili_cookie'],
  appearance: ['language', 'theme', 'font_scale', 'minimize_to_tray', 'show_source_type_icons'],
  ai: ['deepseek_enabled', 'deepseek_api_key', 'deepseek_model', 'deepseek_base_url', 'deepseek_proxy_bypass', 'deepseek_prompt', 'deepseek_min_importance', 'deepseek_translate_release'],
}

const dirtyFields = computed(() => {
  const dirty = new Set<string>()
  const f = form as unknown as Record<string, unknown>
  const base = props.settings as unknown as Record<string, unknown>
  for (const key of Object.keys(f)) {
    if (f[key] !== base[key]) dirty.add(key)
  }
  if (deepseekApiKey.value) dirty.add('deepseek_api_key')
  if (githubToken.value) dirty.add('github_token')
  if (youtubeApiKey.value) dirty.add('youtube_api_key')
  if (bilibiliCookie.value) dirty.add('bilibili_cookie')
  return dirty
})

const dirtyCount = computed(() => dirtyFields.value.size + (agentDirty.value ? 1 : 0))

const dirtyByTab = computed(() => {
  const f = dirtyFields.value
  return {
    general: TAB_SETTING_KEYS.general.filter(k => f.has(k)).length,
    accounts: TAB_SETTING_KEYS.accounts.filter(k => f.has(k)).length,
    appearance: TAB_SETTING_KEYS.appearance.filter(k => f.has(k)).length,
    ai: TAB_SETTING_KEYS.ai.filter(k => f.has(k)).length,
    // Agent 分区与主设置共用「保存设置」按钮，脏点单独比较表单与已保存基线
    agent: agentDirty.value ? 1 : 0,
    // 关于 tab 为纯展示页（无持久化设置项），永远无脏点
    about: 0,
  }
})

function discardChanges() {
  track('settings.discard')
  const langDirty = dirtyFields.value.has('language')
  const themeDirty = dirtyFields.value.has('theme')
  const fontScaleDirty = dirtyFields.value.has('font_scale')
  Object.assign(form, props.settings)
  if (langDirty) setLocale(form.language)
  // 使用 clearThemePreview 清除预览状态（previewTheme 置 null），避免残留“预览中”语义
  // 导致下次打开下拉时 .previewed 类错误高亮当前主题选项
  if (themeDirty) clearThemePreview()
  // 选中档位即已应用缩放：放弃后按已保存值恢复
  if (fontScaleDirty) applyFontScale(form.font_scale)
  deepseekApiKey.value = ''
  githubToken.value = ''
  youtubeApiKey.value = ''
  bilibiliCookie.value = ''
  // Agent 表单未保存的修改一并放弃（回到已保存基线）
  void loadAgentConfig()
}

/** 镜像后端 deepseek.rs::resolve_chat_completion_url 的脚本段拼接逻辑，仅用于
 *  设置界面实时预览最终 POST 端点（与后端保持一致，减少逻辑漂移）。
 *  - 已含 /chat/completions → 原样
 *  - 已含 /v1 → 补 /chat/completions
 *  - 否则 → 补 /v1/chat/completions */
function resolveChatCompletionUrl(baseUrl: string): string {
  const base = baseUrl.trim().replace(/\/+$/, '')
  const lower = base.toLowerCase()
  if (lower.endsWith('/chat/completions')) return base
  if (lower.endsWith('/v1')) return `${base}/chat/completions`
  return `${base}/v1/chat/completions`
}

/** 当前表单中 AI 地址最终会拼出的完整端点，用于输入框下方实时预览。空/未填时不显示。 */
const resolvedEndpointPreview = computed(() => {
  const url = form.deepseek_base_url.trim()
  if (!url) return ''
  return resolveChatCompletionUrl(url)
})

async function handleTestDeepseek() {
  testingDeepseek.value = true
  track('settings.test_ai')
  try {
    // 传入表单当前值（含未保存修改）测试：API Key 留空时后端回退到已保存的 key
    await testDeepseekConnection({
      model: form.deepseek_model.trim(),
      baseUrl: form.deepseek_base_url.trim(),
      apiKey: deepseekApiKey.value,
      proxyBypass: form.deepseek_proxy_bypass,
      proxyUrl: form.proxy_url.trim(),
      proxyMode: form.proxy_mode,
    })
    // 命令成功只返回空值，提示语由前端按 i18n key 渲染，避免后端硬编码语言
    await message(t('settings.connection_success'), { title: t('settings.deepseek_test_title'), kind: 'info' })
  } catch (e: unknown) {
    await message(t('settings.connect_failed') + (e instanceof Error ? e.message : String(e)), { title: t('settings.deepseek_test_title'), kind: 'error' })
  } finally {
    testingDeepseek.value = false
  }
}

async function handleExportBackup() {
  track('settings.export')
  try {
    const path = await exportBackup()
    showToast(t('backup.export_success') + ': ' + path)
    emit('update', false)
  } catch (e: unknown) {
    const msg = e instanceof Error ? e.message : String(e)
    // 后端用 `err.backup_cancelled_export` 稳定 key 表示用户取消；invokeI18n 已将其翻译为
    // t('err.backup_cancelled_export')，两侧同走 i18n，故比较结果与 UI 语言一致、不依赖中文子串。
    if (msg === t('err.backup_cancelled_export')) {
      showToast(t('backup.export_cancelled'))
    } else {
      showToast(t('backup.export_failed') + msg)
    }
  }
}

async function handleImportBackup() {
  const confirmed = await confirm(t('backup.import_confirm'), { title: t('backup.import_confirm_title'), kind: 'warning' })
  if (!confirmed) return
  track('settings.import')
  try {
    await importBackup()
    showToast(t('backup.import_success'))
    emit('update', false, true)
  } catch (e: unknown) {
    const msg = e instanceof Error ? e.message : String(e)
    // 同 export：用 `err.backup_cancelled_import` 稳定 key 判定用户取消，不依赖中文子串。
    if (msg === t('err.backup_cancelled_import')) {
      showToast(t('backup.import_cancelled'))
    } else {
      showToast(t('backup.import_failed') + msg)
    }
  }
}
</script>

<template>
  <section class="tab-content">
    <div class="settings-layout">
      <aside class="settings-sidebar">
        <button :class="{ active: settingsTab === 'general' }" @click="settingsTab = 'general'">{{ t('settings.general') }}<span v-if="dirtyByTab.general" class="sidebar-dirty-dot"></span></button>
        <button :class="{ active: settingsTab === 'accounts' }" @click="settingsTab = 'accounts'">{{ t('settings.accounts') }}<span v-if="dirtyByTab.accounts" class="sidebar-dirty-dot"></span></button>
        <button :class="{ active: settingsTab === 'appearance' }" @click="settingsTab = 'appearance'">{{ t('settings.appearance') }}<span v-if="dirtyByTab.appearance" class="sidebar-dirty-dot"></span></button>
        <button :class="{ active: settingsTab === 'ai' }" @click="settingsTab = 'ai'">{{ t('settings.ai') }}<span v-if="dirtyByTab.ai" class="sidebar-dirty-dot"></span></button>
        <button :class="{ active: settingsTab === 'agent' }" @click="settingsTab = 'agent'">{{ t('settings.agent') }}<span v-if="dirtyByTab.agent" class="sidebar-dirty-dot"></span></button>
        <button :class="{ active: settingsTab === 'data' }" @click="settingsTab = 'data'">{{ t('settings.data') }}</button>
        <button :class="{ active: settingsTab === 'about' }" data-testid="settings-tab-about" @click="settingsTab = 'about'">{{ t('settings.about') }}</button>
        <div class="version-row">
          <button class="version-github-btn" @click="openReleaseUrl('https://github.com/hhelibeb/relwatch')" title="GitHub">
            <svg viewBox="0 0 19 19" width="16" height="16" fill="currentColor">
              <path fill-rule="evenodd" d="M9.356 1.85C5.05 1.85 1.57 5.356 1.57 9.694a7.84 7.84 0 0 0 5.324 7.44c.387.079.528-.168.528-.376 0-.182-.013-.805-.013-1.454-2.165.467-2.616-.935-2.616-.935-.349-.91-.864-1.143-.864-1.143-.71-.48.051-.48.051-.48.787.051 1.2.805 1.2.805.695 1.194 1.817.857 2.268.649.064-.507.27-.857.49-1.052-1.728-.182-3.545-.857-3.545-3.87 0-.857.31-1.558.8-2.104-.078-.195-.349-1 .077-2.078 0 0 .657-.208 2.14.805a7.5 7.5 0 0 1 1.946-.26c.657 0 1.328.092 1.946.26 1.483-1.013 2.14-.805 2.14-.805.426 1.078.155 1.883.078 2.078.502.546.799 1.247.799 2.104 0 3.013-1.818 3.675-3.558 3.87.284.247.528.714.528 1.454 0 1.052-.012 1.896-.012 2.156 0 .208.142.455.528.377a7.84 7.84 0 0 0 5.324-7.441c.013-4.338-3.48-7.844-7.773-7.844"/>
            </svg>
          </button>
          <span class="version-text">v{{ version }}</span>
        </div>
      </aside>
      <div class="settings-main">
        <div v-if="dirtyCount > 0" class="settings-banner">
          <span class="settings-banner-text">{{ t('settings.unsaved_banner', String(dirtyCount)) }}</span>
          <div class="settings-banner-actions">
            <button class="btn-secondary" @click="discardChanges">{{ t('settings.discard') }}</button>
            <button class="btn-primary" @click="handleSave">{{ t('settings.save') }}</button>
          </div>
        </div>
        <div v-if="settingsTab === 'general'" class="settings-form">
          <label class="setting-row setting-row-checkbox">
            <input type="checkbox" v-model="form.auto_start" />
            <span class="setting-label" :data-dirty="dirtyFields.has('auto_start') || null">{{ t('settings.auto_start') }}</span>
          </label>
          <label class="setting-row">
            <span class="setting-label" :data-dirty="dirtyFields.has('poll_interval_minutes') || null">{{ t('settings.poll_interval') }}</span>
            <input
              type="number"
              v-model.number="form.poll_interval_minutes"
              min="5"
              max="1440"
              class="setting-input setting-input-narrow"
            />
          </label>
          <label class="setting-row">
            <span class="setting-label" :data-dirty="dirtyFields.has('proxy_mode') || null">{{ t('settings.proxy_mode') }}</span>
            <select v-model="form.proxy_mode" class="setting-input setting-input-narrow" style="width:calc(14ch * 1.25)">
              <option value="none">{{ t('settings.proxy_none') }}</option>
              <option value="system">{{ t('settings.proxy_system') }}</option>
              <option value="custom">{{ t('settings.proxy_custom') }}</option>
            </select>
          </label>
          <label class="setting-row" v-if="form.proxy_mode === 'custom'">
            <span class="setting-label" :data-dirty="dirtyFields.has('proxy_url') || null">{{ t('settings.proxy') }}</span>
            <input
              type="text"
              v-model="form.proxy_url"
              :placeholder="t('settings.proxy_placeholder')"
              class="setting-input"
            />
          </label>
          <label class="setting-row">
            <span class="setting-label" :data-dirty="dirtyFields.has('log_retention_days') || null">{{ t('settings.log_retention') }}</span>
            <input
              type="number"
              v-model.number="form.log_retention_days"
              min="0"
              max="3650"
              class="setting-input setting-input-narrow"
            />
          </label>
          <label class="setting-row setting-row-checkbox">
            <input type="checkbox" v-model="form.check_prereleases" />
            <span class="setting-label" :data-dirty="dirtyFields.has('check_prereleases') || null">{{ t('settings.check_prereleases') }}</span>
          </label>
          <label class="setting-row setting-row-checkbox">
            <input type="checkbox" v-model="form.fetch_history" />
            <span class="setting-label" :data-dirty="dirtyFields.has('fetch_history') || null">{{ t('settings.fetch_history') }}</span>
          </label>
          <label class="setting-row" v-if="form.fetch_history">
            <span class="setting-label" :data-dirty="dirtyFields.has('fetch_history_count') || null">{{ t('settings.fetch_history_count') }}</span>
            <input
              type="number"
              v-model.number="form.fetch_history_count"
              min="0"
              max="100"
              class="setting-input setting-input-narrow"
            />
            <span class="setting-note">{{ t('settings.fetch_history_count_hint') }}</span>
          </label>
          <label class="setting-row setting-row-checkbox">
            <input type="checkbox" v-model="form.enable_usage_stats" />
            <span class="setting-label" :data-dirty="dirtyFields.has('enable_usage_stats') || null">{{ t('settings.enable_usage_stats') }}</span>
            <span class="setting-hint">{{ t('settings.enable_usage_stats_hint') }}</span>
          </label>
        </div>
        <div v-if="settingsTab === 'accounts'" class="settings-form">
          <label class="setting-row">
            <span class="setting-label" :data-dirty="dirtyFields.has('github_token') || null">{{ t('settings.github_token') }}</span>
            <input
              type="password"
              v-model="githubToken"
              :placeholder="form.github_token_set ? t('settings.github_token_set') : t('settings.github_token_input')"
              class="setting-input"
            />
            <span class="setting-note">{{ t('settings.github_token_note') }}</span>
          </label>
          <label class="setting-row">
            <span class="setting-label" :data-dirty="dirtyFields.has('youtube_api_key') || null">{{ t('settings.youtube_api_key') }}</span>
            <input
              type="password"
              v-model="youtubeApiKey"
              :placeholder="form.youtube_api_key_set ? t('settings.youtube_api_key_set') : t('settings.youtube_api_key_input')"
              class="setting-input"
            />
            <span class="setting-note">{{ t('settings.youtube_api_key_note') }}</span>
          </label>
          <label class="setting-row">
            <span class="setting-label" :data-dirty="dirtyFields.has('bilibili_cookie') || null">{{ t('settings.bilibili_cookie') }}</span>
            <div class="bili-cookie-row">
              <input
                type="password"
                v-model="bilibiliCookie"
                :placeholder="form.bilibili_cookie_set ? t('settings.bilibili_cookie_set') : t('settings.bilibili_cookie_input')"
                class="setting-input"
              />
              <button class="btn-secondary bili-login-btn" :disabled="biliLoginBusy" @click="handleBilibiliLogin">
                {{ biliLoginBusy ? t('settings.bilibili_login_waiting') : t('settings.bilibili_login_btn') }}
              </button>
              <button
                v-if="form.bilibili_cookie_set && !bilibiliCookie"
                class="btn-secondary bili-clear-btn"
                @click="handleClearBilibiliCookie"
              >
                {{ t('settings.bilibili_cookie_clear') }}
              </button>
            </div>
            <span class="setting-note">{{ t('settings.bilibili_cookie_note') }}</span>
          </label>
        </div>
        <div v-if="settingsTab === 'data'" class="settings-form" style="gap:13px">
          <h3 class="setting-section-title">{{ t('backup.section_title') }}</h3>
          <p class="setting-section-desc">
            {{ t('backup.section_desc') }}<template v-if="form.github_token_set"><br>{{ t('backup.token_note') }}</template>
          </p>
          <div class="setting-row backup-actions">
            <button class="btn-secondary" @click="handleExportBackup">{{ t('backup.export_btn') }}</button>
            <button class="btn-secondary btn-danger" @click="handleImportBackup">{{ t('backup.import_btn') }}</button>
          </div>
        </div>
        <div v-if="settingsTab === 'ai'" class="settings-form">
          <label class="setting-row setting-row-checkbox">
            <input type="checkbox" v-model="form.deepseek_enabled" />
            <span class="setting-label" :data-dirty="dirtyFields.has('deepseek_enabled') || null">{{ t('settings.enable_ai') }}</span>
          </label>
          <template v-if="form.deepseek_enabled">
          <label class="setting-row setting-row-checkbox">
            <input type="checkbox" v-model="form.deepseek_proxy_bypass" />
            <span class="setting-label" :data-dirty="dirtyFields.has('deepseek_proxy_bypass') || null">{{ t('settings.deepseek_proxy_bypass') }}</span>
          </label>
          <label class="setting-row setting-row-checkbox">
            <input type="checkbox" v-model="form.deepseek_translate_release" />
            <span class="setting-label" :data-dirty="dirtyFields.has('deepseek_translate_release') || null">{{ t('settings.translate_release') }}</span>
            <span class="setting-hint">{{ t('settings.translate_release_desc') }}</span>
          </label>
          <label class="setting-row">
            <span class="setting-label" :data-dirty="dirtyFields.has('deepseek_api_key') || null">{{ t('settings.api_key') }}</span>
            <input
              type="password"
              v-model="deepseekApiKey"
              :placeholder="form.deepseek_api_key_set ? t('settings.api_key_set') : t('settings.api_key_input')"
              class="setting-input"
            />
          </label>
          <label class="setting-row">
            <span class="setting-label" :data-dirty="dirtyFields.has('deepseek_model') || null">{{ t('settings.model') }}</span>
            <input
              type="text"
              v-model="form.deepseek_model"
              :placeholder="t('settings.deepseek_model_placeholder')"
              class="setting-input"
            />
          </label>
          <label class="setting-row">
            <span class="setting-label" :data-dirty="dirtyFields.has('deepseek_base_url') || null">{{ t('settings.api_url') }}</span>
            <input
              type="text"
              v-model="form.deepseek_base_url"
              :placeholder="t('settings.deepseek_base_url_placeholder')"
              class="setting-input"
            />
            <span class="setting-note" v-if="resolvedEndpointPreview">{{ t('settings.endpoint_preview', resolvedEndpointPreview) }}</span>
          </label>
          <label class="setting-row setting-row-textarea">
            <span class="setting-label" :data-dirty="dirtyFields.has('deepseek_prompt') || null">{{ t('settings.deepseek_prompt') }}</span>
            <textarea
              v-model="form.deepseek_prompt"
              :placeholder="t('settings.deepseek_prompt_placeholder')"
              class="setting-input setting-textarea"
              rows="10"
            />
            <div class="setting-prompt-fixed">
              <code>{{ DEEPSEEK_PROMPT_SUFFIX }}</code>
            </div>
          </label>
          <label class="setting-row">
            <span class="setting-label" :data-dirty="dirtyFields.has('deepseek_min_importance') || null">{{ t('settings.notify_threshold') }}</span>
            <select v-model="form.deepseek_min_importance" class="setting-input setting-input-narrow" style="width:calc(14ch * 1.25)">
              <option value="小">{{ t('settings.importance_any') }}</option>
              <option value="中">{{ t('settings.importance_medium_or_above') }}</option>
              <option value="大">{{ t('settings.importance_high_only') }}</option>
            </select>
          </label>

          <div class="setting-row">
            <button class="btn-secondary" :disabled="testingDeepseek" @click="handleTestDeepseek">
              {{ testingDeepseek ? t('settings.testing') : t('settings.test_connection') }}
            </button>
            <span class="setting-hint">{{ t('settings.test_connection_hint') }}</span>
          </div>
          </template>
        </div>
        <div v-if="settingsTab === 'agent'" class="settings-form">
          <div class="setting-section-title">{{ t('agent.section_title') }}</div>
          <p class="setting-section-desc">{{ t('agent.section_desc') }}</p>
          <label class="setting-row setting-row-checkbox">
            <input type="checkbox" v-model="agentEnabled" />
            <span class="setting-label">{{ t('agent.enabled_global') }}</span>
          </label>
          <template v-if="agentEnabled">
          <label class="setting-row">
            <span class="setting-label">{{ t('agent.type') }}</span>
            <select v-model="agentType" class="setting-input setting-input-narrow" style="width:calc(14ch * 1.25)">
              <option value="pi">{{ t('agent.type_pi') }}</option>
            </select>
          </label>
          <label class="setting-row">
            <span class="setting-label">{{ t('agent.binary_path') }}</span>
            <input
              type="text"
              v-model="agentBinary"
              :placeholder="t('agent.binary_path_placeholder')"
              class="setting-input"
            />
          </label>
          <label class="setting-row">
            <span class="setting-label">{{ t('agent.model') }}</span>
            <input
              type="text"
              v-model="agentModel"
              :placeholder="t('agent.model_placeholder')"
              class="setting-input"
            />
          </label>
          <label class="setting-row">
            <span class="setting-label">{{ t('agent.working_dir') }}</span>
            <input
              type="text"
              v-model="agentWorkingDir"
              :placeholder="t('agent.working_dir_placeholder')"
              class="setting-input"
            />
          </label>
          <label class="setting-row">
            <span class="setting-label">{{ t('agent.prompt_suffix') }}</span>
            <input
              type="text"
              v-model="agentPromptSuffix"
              :placeholder="t('agent.prompt_suffix_placeholder')"
              class="setting-input"
            />
          </label>
          <label class="setting-row">
            <span class="setting-label">{{ t('agent.timeout_seconds') }}</span>
            <input
              type="number"
              v-model.number="agentTimeout"
              min="1"
              class="setting-input setting-input-narrow"
              style="width:calc(8ch * 1.25)"
            />
          </label>
          <div class="setting-row setting-row-skills">
            <span class="setting-label">{{ t('agent.skill_path') }}</span>
            <div class="agent-skill-list">
              <div v-for="(sp, i) in agentSkills" :key="sp" class="agent-skill-item">
                <span class="agent-skill-path" :title="sp">{{ skillShortName(sp) }}</span>
                <button type="button" class="agent-skill-remove" :title="t('agent.remove_skill')" @click="removeAgentSkill(i)">
                  <svg viewBox="0 0 16 16"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" fill="none"/></svg>
                </button>
              </div>
              <div class="agent-skill-add">
                <input
                  v-model="newAgentSkill"
                  class="setting-input"
                  :placeholder="t('agent.skill_path_placeholder')"
                  @keydown.enter.prevent="addAgentSkill"
                />
                <button type="button" class="btn-secondary" :disabled="!newAgentSkill.trim()" @click="addAgentSkill">{{ t('agent.add_skill') }}</button>
              </div>
            </div>
          </div>
          </template>
        </div>
        <div v-if="settingsTab === 'appearance'" class="settings-form">
          <div class="setting-row">
            <span class="setting-label" :data-dirty="dirtyFields.has('language') || null">{{ t('settings.language') }}<svg class="label-icon"><use href="/icons.svg#language-icon"/></svg></span>
            <div ref="langSelectRef" class="theme-select" @mouseleave="clearLangPreview">
              <button type="button" class="theme-select-trigger setting-input" @click="toggleLangDropdown" @keydown="handleLangDropdownKeydown" :aria-expanded="langDropdownOpen" aria-haspopup="listbox">
                <span>{{ previewLang ? languages.find(l => l.value === previewLang)?.label : languages.find(l => l.value === form.language)?.label }}</span>
                <svg class="theme-select-arrow" viewBox="0 0 12 12" width="12" height="12"><path fill="none" stroke="currentColor" stroke-width="1.5" d="M3 5l3 3 3-3"/></svg>
              </button>
              <div v-if="langDropdownOpen" class="dropdown-panel theme-select-dropdown" role="listbox" @keydown="handleLangDropdownKeydown">
                <div
                  v-for="lang in languages"
                  :key="lang.value"
                  class="theme-select-option"
                  :class="{ selected: form.language === lang.value && !previewLang, previewed: previewLang === lang.value }"
                  :data-value="lang.value"
                  tabindex="-1"
                  role="option"
                  :aria-selected="form.language === lang.value"
                  @click.stop="selectLang(lang.value)"
                  @mouseenter="setLangPreview(lang.value)"
                >
                  {{ lang.label }}
                </div>
              </div>
            </div>
          </div>
          <div class="setting-row">
            <span class="setting-label" :data-dirty="dirtyFields.has('theme') || null">{{ t('settings.theme') }}</span>
            <div ref="themeSelectRef" class="theme-select" @mouseleave="clearThemePreview">
              <button type="button" class="theme-select-trigger setting-input" @click="toggleDropdown" @keydown="handleThemeDropdownKeydown" :aria-expanded="themeDropdownOpen" aria-haspopup="listbox">
                <span>{{ previewTheme ? t('settings.theme_' + previewTheme) : t('settings.theme_' + form.theme) }}</span>
                <svg class="theme-select-arrow" viewBox="0 0 12 12" width="12" height="12"><path fill="none" stroke="currentColor" stroke-width="1.5" d="M3 5l3 3 3-3"/></svg>
              </button>
              <div v-if="themeDropdownOpen" class="dropdown-panel theme-select-dropdown" role="listbox" @keydown="handleThemeDropdownKeydown">
                <div
                  v-for="opt in themeOptions"
                  :key="opt.value"
                  class="theme-select-option"
                  :class="{ selected: form.theme === opt.value && !previewTheme, previewed: previewTheme === opt.value }"
                  :data-value="opt.value"
                  tabindex="-1"
                  role="option"
                  :aria-selected="form.theme === opt.value"
                  @click.stop="selectTheme(opt.value)"
                  @mouseenter="setThemePreview(opt.value)"
                >
                  {{ t(opt.label) }}
                </div>
              </div>
            </div>
          </div>
          <div class="setting-row">
            <span class="setting-label" :data-dirty="dirtyFields.has('font_scale') || null">{{ t('settings.font_scale') }}</span>
            <div ref="fontScaleSelectRef" class="theme-select" @mouseleave="clearFontScalePreview">
              <button type="button" class="theme-select-trigger setting-input" @click="toggleFontScaleDropdown" @keydown="handleFontScaleDropdownKeydown" :aria-expanded="fontScaleDropdownOpen" aria-haspopup="listbox">
                <span>{{ fontScaleLabel }}</span>
                <svg class="theme-select-arrow" viewBox="0 0 12 12" width="12" height="12"><path fill="none" stroke="currentColor" stroke-width="1.5" d="M3 5l3 3 3-3"/></svg>
              </button>
              <div v-if="fontScaleDropdownOpen" class="dropdown-panel theme-select-dropdown" role="listbox" @keydown="handleFontScaleDropdownKeydown">
                <div
                  v-for="opt in fontScaleOptions"
                  :key="opt"
                  class="theme-select-option"
                  :class="{ selected: form.font_scale === opt }"
                  :data-value="opt"
                  tabindex="-1"
                  role="option"
                  :aria-selected="form.font_scale === opt"
                  @click.stop="selectFontScale(String(opt))"
                >
                  {{ t('settings.font_scale_' + opt) }}
                </div>
              </div>
            </div>
          </div>
          <label class="setting-row setting-row-checkbox">
            <input type="checkbox" v-model="form.minimize_to_tray" />
            <span class="setting-label" :data-dirty="dirtyFields.has('minimize_to_tray') || null">{{ t('settings.minimize_tray') }}</span>
          </label>
          <label class="setting-row setting-row-checkbox">
            <input type="checkbox" v-model="form.show_source_type_icons" />
            <span class="setting-label" :data-dirty="dirtyFields.has('show_source_type_icons') || null">{{ t('settings.show_source_type_icons') }}</span>
          </label>
        </div>
        <div v-if="settingsTab === 'about'" class="settings-form">
          <div class="about-header">
            <div class="about-app-name">{{ t('about.app_name') }}</div>
            <div class="about-version">{{ t('about.version') }} v{{ version }}</div>
            <p class="setting-section-desc">{{ t('about.app_desc') }}</p>
            <div class="setting-row">
              <a class="setting-link" href="https://github.com/hhelibeb/relwatch">{{ t('about.github') }}: github.com/hhelibeb/relwatch</a>
            </div>
          </div>
          <hr class="setting-divider" />
          <!-- 软件更新：展示型分组，无持久化设置项（不入 TAB_SETTING_KEYS / dirty 徽标） -->
          <div class="setting-section-title">{{ t('update.section_title') }}</div>
          <div class="setting-row setting-row-checkbox">
            <span class="setting-label">{{ t('update.current_version') }} <strong>v{{ updateCurrentVersion }}</strong></span>
            <button
              class="btn-secondary"
              :disabled="isDevBuild || updateBusy"
              :title="isDevBuild ? t('update.dev_disabled') : undefined"
              data-testid="update-check-btn"
              @click="checkForUpdate"
            >
              {{ updateStatus === 'checking' ? t('update.checking') : t('update.check') }}
            </button>
            <span v-if="isDevBuild" class="setting-note">{{ t('update.dev_disabled') }}</span>
          </div>
          <div v-if="updateStatus === 'upToDate'" class="setting-row">
            <span class="update-status-ok">✓ {{ tm('update.up_to_date', { version: `v${updateCurrentVersion}` }) }}</span>
          </div>
          <div v-else-if="updateStatus === 'available'" class="setting-row">
            <span class="setting-label">{{ tm('update.available', { version: `v${pendingUpdate?.version ?? ''}` }) }}</span>
            <div class="update-actions">
              <button class="btn-secondary" :disabled="updateBusy" data-testid="update-install-btn" @click="downloadAndInstall">{{ t('update.download_install') }}</button>
              <button class="btn-secondary" :disabled="updateBusy" data-testid="update-notes-btn" @click="openUpdateNotes">{{ t('update.view_notes') }}</button>
            </div>
          </div>
          <div v-else-if="updateStatus === 'downloading'" class="setting-row">
            <div class="update-progress">
              <div class="update-progress-inner" :style="{ width: `${updatePercent ?? 0}%` }"></div>
            </div>
            <span class="setting-note">{{ updateDownloadText }}</span>
          </div>
          <div v-else-if="updateStatus === 'installing'" class="setting-row">
            <span class="setting-label">{{ t('update.installing') }}</span>
          </div>
          <div v-else-if="updateStatus === 'error'" class="setting-row">
            <span class="update-status-error">⚠ {{ updateErrorText }}</span>
            <div v-if="showUpdateRetry || showUpdateDownloadPage" class="update-actions">
              <button v-if="showUpdateRetry" class="btn-secondary" @click="retryUpdate">{{ t('update.retry') }}</button>
              <button v-if="showUpdateDownloadPage" class="btn-secondary" @click="openDownloadPage">{{ t('update.open_download_page') }}</button>
            </div>
          </div>
        </div>
        <div v-if="settingsTab !== 'data' && settingsTab !== 'about'" class="setting-actions">
          <button class="btn-primary" :disabled="savingSettings" @click="handleSave">
            {{ savingSettings ? t('settings.saving') : t('settings.save') }}
          </button>
        </div>
      </div>
    </div>

    <!-- Release Note 弹窗：Teleport 到 body，脱离设置页窄栏布局（组件内部自绘） -->
    <UpdateNotesModal
      v-if="showUpdateNotes && updateNotesBody"
      :version="updateNotesVersion"
      :date="updateNotesDate"
      :body="updateNotesBody"
      @close="showUpdateNotes = false"
    />
  </section>
</template>
<style scoped>
/* 设置 */
.settings-layout {
  display: flex;
  gap: 16px;
  align-items: flex-start;
}

.settings-sidebar {
  display: flex;
  flex-direction: column;
  gap: 2px;
  min-width: 120px;
  position: sticky;
  top: 0;
  align-self: flex-start;
  padding: 0 12px 0 0;
}

.settings-sidebar button {
  position: relative;
  padding: 7px 10px;
  border: none;
  background: transparent;
  color: var(--text-muted);
  font-size: 13px;
  border-radius: var(--radius-sm);
  cursor: pointer;
  text-align: left;
  transition: background 0.15s, color 0.15s;
}

.sidebar-dirty-dot {
  position: absolute;
  right: 10px;
  top: 50%;
  transform: translateY(-50%);
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--warning);
}

.settings-sidebar button:hover {
  background: var(--bg-subtle);
  color: var(--text);
}

.settings-sidebar button.active {
  background: var(--bg-subtle);
  color: var(--text);
  font-weight: 600;
}

.version-row {
  margin-top: auto;
  padding: 8px 14px 4px 11px;
  border-top: 1px solid var(--border);
  display: flex;
  align-items: center;
  gap: 6px;
}

.version-text {
  font-size: 11px;
  color: var(--text-muted);
}

.version-github-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  border-radius: var(--radius-xs);
  transition: color 0.15s, background 0.15s;
}

.version-github-btn:hover {
  color: var(--text);
  background: var(--bg-subtle);
}

.version-github-btn svg {
  width: 18px;
  height: 18px;
}

.settings-sidebar .version-github-btn {
  padding: 0;
}

.settings-main {
  flex: 1;
  min-width: 0;
}

.settings-form {
  display: flex;
  flex-direction: column;
  gap: 16px;
  padding: 2px 0 24px;
  background: transparent;
}

.setting-row {
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.setting-label {
  font-size: 13px;
  font-weight: 500;
  display: inline-flex;
  align-items: center;
  gap: 4px;
}

.setting-label[data-dirty]::after {
  content: '●';
  color: var(--warning);
  font-size: 10px;
  margin-left: 2px;
}

.label-icon {
  width: 14px;
  height: 14px;
  color: var(--text-muted);
}

.setting-input {
  padding: 8px 12px;
  background: var(--input-bg);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  color: var(--text);
  font-size: 13px;
  outline: none;
}

.setting-input-narrow {
  width: 14ch;
}

.bili-cookie-row {
  display: flex;
  align-items: center;
  gap: 8px;
}

.bili-cookie-row .setting-input {
  flex: 1;
  min-width: 0;
}

.bili-login-btn {
  white-space: nowrap;
  flex-shrink: 0;
}

.setting-textarea {
  width: 100%;
  min-height: 120px;
  resize: vertical;
  font-family: 'Consolas', 'Courier New', monospace;
  font-size: 0.85em;
  line-height: 1.5;
}

.setting-row-textarea {
  flex-direction: column;
  align-items: flex-start;
  gap: 6px;
}

.setting-row-textarea .setting-label {
  margin-bottom: 2px;
}

.setting-prompt-fixed {
  width: 100%;
  padding: 8px 12px;
  background: var(--bg-subtle);
  border: 1px dashed var(--border-strong);
  border-radius: var(--radius-sm);
  font-family: 'Consolas', 'Courier New', monospace;
  font-size: 0.85em;
  line-height: 1.5;
  color: var(--text-muted);
  user-select: none;
  cursor: not-allowed;
  white-space: pre-wrap;
  opacity: 0.85;
}

.setting-prompt-fixed code {
  display: block;
  background: transparent;
  padding: 0;
  color: inherit;
}

.setting-input {
  transition: border-color 0.15s ease, box-shadow 0.15s ease;
}

.setting-input:focus {
  border-color: var(--primary);
  box-shadow: var(--focus-ring);
}

.setting-note {
  font-size: 12px;
  color: var(--text-muted);
  line-height: 1.4;
}

.setting-link {
  font-size: 12px;
  color: var(--primary);
  text-decoration: none;
  cursor: pointer;
}

.setting-link:hover {
  text-decoration: underline;
}

select.setting-input {
  appearance: none;
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 12 12'%3E%3Cpath fill='none' stroke='%239ca3af' stroke-width='1.5' d='M3 5l3 3 3-3'/%3E%3C/svg%3E");
  background-repeat: no-repeat;
  background-position: right 10px center;
  padding-right: 28px;
}

.setting-row-checkbox {
  flex-direction: row;
  flex-wrap: wrap;
  align-items: center;
  gap: 8px;
}

.settings-form .btn-primary {
  margin-left: 0;
  align-self: flex-start;
}

.setting-divider {
  border: none;
  border-top: 1px solid var(--border);
  margin: 4px 0;
}

.settings-banner {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 12px;
  padding: 7px 14px;
  margin-bottom: 12px;
  background: var(--warning-soft-bg);
  border: 1px solid var(--border);
  border-radius: var(--radius);
}

.settings-banner-text {
  font-size: 13px;
  color: var(--text);
  line-height: 1.4;
}

.settings-banner-actions {
  display: flex;
  gap: 8px;
  flex-shrink: 0;
}

.settings-banner-actions .btn-primary,
.settings-banner-actions .btn-secondary {
  padding: 6px 14px;
  font-size: 12px;
  white-space: nowrap;
}

.setting-section-title {
  font-size: 15px;
  font-weight: 600;
  margin: 0;
}

.setting-section-sep {
  border: none;
  border-top: 1px solid var(--border);
  margin: 16px 0;
}

.setting-section-desc {
  font-size: 12px;
  color: var(--text-muted);
  margin: 4px 0 12px;
  line-height: 1.5;
}

.setting-hint {
  font-size: 12px;
  color: var(--text-muted);
  line-height: 1.5;
  flex-basis: 100%;
  width: 100%;
  margin-left: 0;
  margin-top: -2px;
}

.backup-actions {
  display: flex;
  gap: 10px;
  margin-top: 8px;
}

/* ── 软件更新分组 ─────────────────────────────── */
.about-header {
  display: flex;
  flex-direction: column;
  gap: 6px;
  padding: 4px 0 8px;
}

.about-app-name {
  font-size: 20px;
  font-weight: 700;
}

.about-version {
  font-size: 13px;
  color: var(--text-muted);
}

.update-actions {
  display: flex;
  gap: 10px;
  margin-top: 4px;
}

.update-progress {
  width: 100%;
  max-width: 320px;
  height: 6px;
  border-radius: 3px;
  background: var(--bg-subtle);
  border: 1px solid var(--border);
  overflow: hidden;
}

.update-progress-inner {
  height: 100%;
  background: var(--primary);
  transition: width 0.2s ease;
}

.update-status-ok {
  font-size: 13px;
  color: var(--text);
}

.update-status-error {
  font-size: 13px;
  color: var(--warning);
  line-height: 1.5;
}

.setting-actions {
  display: flex;
  gap: 8px;
  align-items: center;
  margin-top: 16px;
}

.btn-secondary {
  align-self: flex-start;
  padding: 6px 16px;
  background: var(--surface);
  color: var(--text);
  border: 1px solid var(--border-strong);
  border-radius: var(--radius-sm);
  cursor: pointer;
  font-size: 13px;
  font-weight: 500;
  transition: background 0.12s ease, border-color 0.12s ease;
}

.btn-secondary:hover {
  background: var(--bg-subtle);
  border-color: var(--text-faint);
}

/* ── 自定义主题下拉选择器 ───────────────────────── */
.theme-select {
  position: relative;
  width: 18ch;
}

.theme-select-trigger {
  display: flex;
  align-items: center;
  justify-content: space-between;
  width: 100%;
  cursor: pointer;
  text-align: left;
  font-size: 13px;
}

.theme-select-arrow {
  flex-shrink: 0;
  margin-left: 4px;
  transition: transform 0.2s;
}

/* radius/shadow 覆盖公共基类（选择面板视觉更重） */
.theme-select-dropdown {
  top: calc(100% + 4px);
  left: 0;
  width: 100%;
  border-radius: var(--radius);
  box-shadow: var(--shadow-lg);
  overflow: hidden;
}

.theme-select-option {
  padding: 8px 12px;
  font-size: 13px;
  cursor: pointer;
  color: var(--text);
  transition: background 0.1s;
}

.theme-select-option:hover,
.theme-select-option.previewed {
  background: var(--bg-subtle);
  color: var(--text);
}

.theme-select-option.selected {
  font-weight: 600;
}
.setting-row-skills {
  align-items: flex-start;
}
.setting-row-skills .setting-label {
  padding-top: 8px;
}
.agent-skill-list {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}
.agent-skill-item {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 5px 8px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--bg-subtle);
}
.agent-skill-path {
  flex: 1;
  min-width: 0;
  font-size: 12px;
  font-family: var(--mono-font, monospace);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.agent-skill-remove {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  border-radius: 4px;
}
.agent-skill-remove:hover {
  color: #d64545;
  background: var(--bg-hover);
}
.agent-skill-remove svg {
  width: 12px;
  height: 12px;
}
.agent-skill-add {
  display: flex;
  gap: 6px;
}
.agent-skill-add .setting-input {
  flex: 1;
}
</style>