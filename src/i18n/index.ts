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
  }
}

export function getLocale(): string {
  return locale.value
}

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

export function tm(key: string, args: Record<string, string>): string {
  const msg = messages[locale.value]
  if (!msg) return key
  let text = msg[key]
  if (text === undefined) return key
  if (args.action) {
    const ak = actionKeys[args.action] || args.action
    args = { ...args, action: t(ak) }
  }
  Object.entries(args).forEach(([k, v]) => {
    text = text!.replace(`{${k}}`, v)
  })
  return text.replace(/setting\.\w+/g, (match) => t(match))
}

export const languages = [
  { value: 'zh-CN', label: '中文' },
  { value: 'en-US', label: 'English' },
]
