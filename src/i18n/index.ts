import { ref } from 'vue'
import zhCN from './zh-CN'
import enUS from './en-US'

const messages: Record<string, Record<string, string>> = {
  'zh-CN': zhCN,
  'en-US': enUS,
}

const locale = ref('zh-CN')

export function setLocale(lang: string) {
  if (messages[lang]) {
    locale.value = lang
  } else {
    console.warn(`[i18n] unsupported locale: ${lang}`)
  }
}

export function getLocale(): string {
  return locale.value
}

/** 使用位置参数 {0} {1} ... 翻译指定 key */
export function t(key: string, ...args: string[]): string {
  const msg = messages[locale.value]
  if (!msg) return key
  let text = msg[key]
  if (text === undefined) return key
  if (args.length) {
    args.forEach((arg, i) => {
      text = text!.replace(`{${i}}`, arg)
    })
  }
  return text
}

const actionKeys: Record<string, string> = {
  pending: 'status.pending',
  ignored: 'status.ignored',
  snoozed: 'status.snoozed',
  clicked: 'status.viewed',
}

/**
 * 将字符串中的 setting.\w+ 引用替换为对应的翻译文本。
 *
 * 协议：token = `setting.` + 字母数字/下划线（单段键，不含点）。
 * 与后端 `resolve_setting_keys`（src-tauri/src/i18n.rs）规则必须保持一致，
 * 两端同步修改，防止渲染结果漂移；未命中的键原样保留。
 */
function resolveSettingKeys(text: string): string {
  return text.replace(/setting\.\w+/g, (match) => t(match))
}

/** 使用命名参数 {key} 翻译指定 key，支持 action 特殊处理和嵌套 setting.\w+ 解析 */
export function tm(key: string, args: Record<string, string>): string {
  const msg = messages[locale.value]
  if (!msg) return key
  let text = msg[key]
  if (text === undefined) return key
  if (args.action) {
    const ak = actionKeys[args.action] || args.action
    args = { ...args, action: t(ak) }
  }
  // 空 repo 兜底：GitHub 风格模板 {owner}/{repo} 在 repo 为空时省略斜杠（YouTube 源）
  if (args.repo === '') {
    text = text!.replace('/{repo}', '')
  }
  Object.entries(args).forEach(([k, v]) => {
    text = text!.replace(`{${k}}`, v)
  })
  return resolveSettingKeys(text)
}

export const languages = [
  { value: 'zh-CN', label: '中文' },
  { value: 'en-US', label: 'English' },
] as const
