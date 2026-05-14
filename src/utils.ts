import { t, getLocale } from './i18n'

export function importanceLabel(imp: string | null): string {
  if (!imp) return ''
  switch (imp) {
    case '大': return '重要度: 🔴 大'
    case '中': return '重要度: 🟡 中'
    case '小': return '重要度: 🟢 小'
    default: return imp
  }
}

export function formatDate(dateStr: string): string {
  return new Date(dateStr).toLocaleString(getLocale())
}

export function logLevelClass(level: string): string {
  switch (level) {
    case 'ERROR': return 'log-error'
    case 'WARN': return 'log-warn'
    default: return 'log-info'
  }
}

export function statusLabel(status: string): string {
  if (isUnreadStatus(status)) return t('status.pending')
  if (isReadStatus(status)) return t('status.viewed')
  return status
}

export function statusClass(status: string): string {
  if (isUnreadStatus(status)) return 'status-unread'
  if (isReadStatus(status)) return 'status-read'
  return 'status-unknown'
}

export function isUnreadStatus(status: string): boolean {
  return status === 'pending' || status === 'snoozed'
}

export function isReadStatus(status: string): boolean {
  return status === 'clicked' || status === 'ignored'
}
