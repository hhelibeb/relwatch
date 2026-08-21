import { t, getLocale } from './i18n'

interface SearchableRelease {
  owner: string
  repo: string
  tag_name: string
  release_name: string
  body: string | null
  source_description?: string | null
}

export function formatDate(dateStr: string): string {
  if (!dateStr) return ''
  const d = new Date(dateStr)
  if (isNaN(d.getTime())) return ''
  return d.toLocaleString(getLocale())
}

export function formatCountdown(secs: number): string {
  if (secs <= 0) return t('app.check_soon')
  const m = Math.floor(secs / 60)
  const s = secs % 60
  return t('app.min_sec', String(m), String(s))
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
    (release.body || '').toLowerCase().includes(q) ||
    (release.source_description || '').toLowerCase().includes(q)
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

/** skill 路径短名：去掉尾部分隔符，取最后一段；
 * 路径指向文件（如 …/commit/SKILL.md）时取所属目录名（skill 名），展示用。 */
export function skillShortName(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, '')
  const segs = trimmed.split(/[\\/]/)
  let seg = segs.pop()
  if (seg && segs.length > 0 && /\.[A-Za-z0-9]+$/.test(seg)) {
    // 末段是文件（带扩展名）：取上一段目录名，避免显示成 SKILL.md
    seg = segs.pop()
  }
  return seg && seg.length > 0 ? seg : trimmed
}
