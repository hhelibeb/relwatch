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

export const languages = [
  { value: 'zh-CN', label: '中文' },
  { value: 'en-US', label: 'English' },
]
