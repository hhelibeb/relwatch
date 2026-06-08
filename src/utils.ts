import { t, getLocale } from './i18n'

interface SearchableRelease {
  owner: string
  repo: string
  tag_name: string
  release_name: string
  body: string | null
}

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
  if (!dateStr) return ''
  const d = new Date(dateStr)
  if (isNaN(d.getTime())) return ''
  return d.toLocaleString(getLocale())
}

export function releaseMatchesSearch(release: SearchableRelease, query: string): boolean {
  const q = query.trim().toLowerCase()
  if (!q) return true

  const repoName = `${release.owner}/${release.repo}`.toLowerCase()
  return repoName.includes(q) ||
    release.owner.toLowerCase().includes(q) ||
    release.repo.toLowerCase().includes(q) ||
    release.tag_name.toLowerCase().includes(q) ||
    release.release_name.toLowerCase().includes(q) ||
    (release.body || '').toLowerCase().includes(q)
}

export function logLevelClass(level: string): string {
  switch (level) {
    case 'ERROR': return 'log-error'
    case 'WARN': return 'log-warn'
    default: return 'log-info'
  }
}

export function statusLabel(status: string, snoozeUntil?: string | null): string {
  if (isUnreadStatus(status, snoozeUntil)) return t('status.pending')
  if (status === 'snoozed') return t('status.snoozed')
  if (isReadStatus(status)) return t('status.viewed')
  return status
}

export function statusClass(status: string, snoozeUntil?: string | null): string {
  if (isUnreadStatus(status, snoozeUntil)) return 'status-unread'
  if (status === 'snoozed') return 'status-snoozed'
  if (isReadStatus(status)) return 'status-read'
  return 'status-unknown'
}

export function isUnreadStatus(status: string, snoozeUntil?: string | null): boolean {
  if (status === 'snoozed' && snoozeUntil) {
    const until = new Date(snoozeUntil).getTime()
    if (!isNaN(until) && until > Date.now()) {
      return false
    }
  }
  return status === 'pending' || status === 'snoozed'
}

export function isReadStatus(status: string): boolean {
  return status === 'clicked' || status === 'ignored'
}
