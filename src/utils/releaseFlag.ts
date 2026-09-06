import type { ReleaseInfo } from '../api/releases'

/**
 * 旗标（releases.flag）的前端展示规则。
 *
 * 旗标为单选颜色（Outlook 分类式）：0 = 未标记，1-6 = 红/橙/黄/绿/蓝/紫，
 * 颜色不带预设语义，具体含义由用户自行赋予。图标统一用 Outlook 式
 * 「标签贴纸」（icons.svg#flag-tag-icon），与重要度的实心圆点从形状上区分。
 */

export const FLAG_MAX = 6

/** 旗标 1-6 的 i18n label key（右键菜单/弹窗色板共用，顺序与颜色编号一致）。 */
export const FLAG_COLOR_LABEL_KEYS = [
  'release.flag_red',
  'release.flag_orange',
  'release.flag_yellow',
  'release.flag_green',
  'release.flag_blue',
  'release.flag_purple',
]

/** 索引 1-6 对应 CSS 色板变量（style.css 的 --flag-1..6）。 */
const FLAG_COLOR_VARS: Record<number, string> = {
  1: 'var(--flag-1)',
  2: 'var(--flag-2)',
  3: 'var(--flag-3)',
  4: 'var(--flag-4)',
  5: 'var(--flag-5)',
  6: 'var(--flag-6)',
}

export function isFlagged(flag: number): boolean {
  return flag >= 1 && flag <= FLAG_MAX
}

export function releaseFlagged(release: ReleaseInfo): boolean {
  return isFlagged(release.flag)
}

/** 旗标颜色的 CSS var；未标记返回 null。 */
export function releaseFlagColor(release: ReleaseInfo): string | null {
  return isFlagged(release.flag) ? FLAG_COLOR_VARS[release.flag] : null
}

/** 按旗标编号取颜色 CSS var（筛选面板/chips 菜单按编号渲染时用）；越界返回 null。 */
export function flagColorByIndex(flag: number): string | null {
  return isFlagged(flag) ? FLAG_COLOR_VARS[flag] : null
}
