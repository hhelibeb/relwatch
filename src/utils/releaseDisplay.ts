import { t } from '../i18n'
import type { ReleaseInfo } from '../api/releases'
import { getSourceTypeDef } from '../api/source-registry'

/**
 * ReleaseItem 卡片与 ReleaseDetailModal 弹窗共用的展示规则。
 * 收敛了原先两处逐段复制的实现——规则一旦分叉就会出现
 * 「卡片能翻译、弹窗不能」这类不一致。
 */

/** 展示名：release_name 非空且不同于 tag_name 时显示（YouTube/HF 源常用）。 */
export function releaseDisplayTitle(release: ReleaseInfo): string {
  const name = release.release_name.trim()
  return name && name !== release.tag_name ? name : ''
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
