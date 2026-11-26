export interface VideoMeta {
  thumbnail: string | null
  kind: 'video' | 'live' | null
  duration: string | null
  viewCount: number | null
}

function trimTrailingZero(v: number): string {
  return v.toFixed(1).replace(/\.0$/, '')
}

export function formatCompactCount(n: number): string {
  if (n >= 1_000_000) return trimTrailingZero(n / 1_000_000) + 'M'
  if (n >= 1_000) return trimTrailingZero(n / 1_000) + 'K'
  return String(n)
}

export function parseVideoMeta(extraMetadata: string | null): VideoMeta | null {
  if (!extraMetadata) return null
  try {
    const obj = JSON.parse(extraMetadata)
    // B 站旧数据封面可能是 http://，升级为 https 兼容 CSP（img-src 仅允许 https）
    const rawThumb = typeof obj.thumbnail === 'string' ? obj.thumbnail : null
    const thumbnail = rawThumb?.startsWith('http://') ? rawThumb.replace(/^http:\/\//, 'https://') : rawThumb
    return {
      thumbnail,
      kind: obj.kind === 'live' ? 'live' : obj.kind === 'video' ? 'video' : null,
      duration: typeof obj.duration === 'string' && obj.duration ? obj.duration : null,
      viewCount: typeof obj.view_count === 'number' ? obj.view_count : null,
    }
  } catch {
    return null
  }
}

export function formatVideoDuration(iso: string | null): string {
  if (!iso) return ''
  const parts = iso.match(/^PT(?:(\d+)H)?(?:(\d+)M)?(?:(\d+)S)?$/)
  if (parts) {
    const h = parts[1] ? parseInt(parts[1], 10) : 0
    const min = parts[2] ? parseInt(parts[2], 10) : 0
    const sec = parts[3] ? parseInt(parts[3], 10) : 0
    if (h > 0) return `${h}:${String(min).padStart(2, '0')}:${String(sec).padStart(2, '0')}`
    return `${min}:${String(sec).padStart(2, '0')}`
  }
  if (/^(?:\d+:)?\d{1,2}:\d{2}$/.test(iso)) return iso
  return ''
}

export function formatViewCount(n: number, locale: string): string {
  // 阈值取四舍五入后不溢出当前单位的下限（如 99,950,000 → 1亿）
  if (locale === 'zh-CN') {
    if (n >= 99_950_000) return trimTrailingZero(n / 100_000_000) + '亿'
    if (n >= 9_950) return trimTrailingZero(n / 10_000) + '万'
    return String(n)
  }
  if (n >= 999_500) return trimTrailingZero(n / 1_000_000) + 'M'
  if (n >= 995) return trimTrailingZero(n / 1_000) + 'K'
  return String(n)
}
