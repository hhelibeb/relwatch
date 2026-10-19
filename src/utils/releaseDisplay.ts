import { t } from '../i18n'
import type { ReleaseInfo } from '../api/releases'
import { getSourceTypeDef } from '../api/source-registry'

/**
 * ReleaseItem 卡片与 ReleaseDetailModal 弹窗共用的展示规则。
 * 两处各自实现会分叉出「卡片能翻译、弹窗不能」这类不一致。
 */

/** 展示名：release_name 非空且不同于 tag_name 时显示（YouTube/HF 源常用）。 */
export function releaseDisplayTitle(release: ReleaseInfo): string {
  const name = release.release_name.trim()
  return name && name !== release.tag_name ? name : ''
}

/**
 * 列表卡片的第二行标题是否只是版本号换了种写法（`v3.6.0` vs `v3.6.0 "Vapor"`）。
 *
 * 不能直接复用 releaseDisplayTitle 的判等：GitHub 的 release name 惯例是
 * 「tag + codename」，两者不相等，但它携带的额外信息只有一个代号。
 * 只剥末尾引号包裹的 codename 与 `v` 前缀；剥完仍不相等（如 `1.2.0 - Hotfix`、
 * `v1.2.0 (2026-09-01)`）就认为标题确实带了版本号之外的信息，保留。
 */
export function releaseTitleRedundant(release: ReleaseInfo): boolean {
  const name = release.release_name.trim()
  if (!name) return true
  return normalizeTitleKey(name) === normalizeTitleKey(release.tag_name)
}

function normalizeTitleKey(title: string): string {
  return title
    .trim()
    .toLowerCase()
    .replace(/\s*(?:"[^"]*"|'[^']*'|\u201c[^\u201d]*\u201d|\u300c[^\u300d]*\u300d)\s*$/, '')
    .replace(/^v(?=\d)/, '')
    .trim()
}

/** ai_importance 存的是中文枚举（大/中/小），展示时映射到 i18n 文案，兼容英文界面。 */
export function releaseImportanceText(release: ReleaseInfo): string {
  switch (release.ai_importance) {
    case '大': return t('release.importance_high')
    case '中': return t('release.importance_medium')
    case '小': return t('release.importance_low')
    default: return ''
  }
}

export function releaseImportanceClass(release: ReleaseInfo): string {
  switch (release.ai_importance) {
    case '大': return 'release-importance-high'
    case '中': return 'release-importance-medium'
    case '小': return 'release-importance-low'
    default: return ''
  }
}

/** 「翻译」可用基础条件（卡片与弹窗共用）：有原文、无译文、源类型支持 AI、AI 已启用。
 *  弹窗侧再叠加视图条件（如仅全文视图可翻译）。 */
export function canTranslateRelease(release: ReleaseInfo, aiEnabled: boolean): boolean {
  return !!release.body
    && !release.body_translated
    && getSourceTypeDef(release.source_type)?.aiSummary !== false
    && aiEnabled
}
