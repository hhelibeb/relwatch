/**
 * 图片 URL → media 网关地址的改写器（纯函数）。
 *
 * 背景：CSP `img-src` 只放行 media 协议来源（不再放行任意 https:）。WebView 里
 * 直接指向远程 https 图片的 `<img>` 会被 CSP 拦截（显式失败，而非静默走系统代理）。
 * 所有远程图片必须改写为 `http://media.localhost/<encodeURIComponent(原始URL)>`，
 * 由 Rust 端 media 网关按应用代理设置下载后返回。
 *
 * 约定：
 * - 仅 http/https 绝对 URL 需要改写；`data:`、`blob:`、相对路径、`/icons.svg#...`
 *   等原地保留；
 * - 空值 / 非字符串返回原值（调用方多为字符串可空字段）；
 * - 改写对已有 media 前缀的 URL 幂等（不重复包一层）。
 */

/** media 网关的固定来源前缀。Windows WebView2 认 http://media.localhost/ 形态。 */
export const MEDIA_BASE = 'http://media.localhost/'

/**
 * 判断一个 URL 是否应走 media 网关（远程 http/https 图片）。
 * data:/blob:/相对路径/自身资源不属于远程图片，无需改写。
 */
export function isRemoteImageUrl(url: string): boolean {
  return /^https?:\/\//i.test(url)
}

/**
 * 把远程图片 URL 改写为 media 网关地址。
 *
 * @returns 改写后的 URL；非远程图片（data:、blob:、相对等）或空输入原样返回。
 */
export function toMediaUrl(url: string): string {
  if (!url) return url
  // 已是 media 网关来源：幂等返回
  if (url.startsWith(MEDIA_BASE)) return url
  if (!isRemoteImageUrl(url)) return url
  return MEDIA_BASE + encodeURIComponent(url)
}

/**
 * 从 media 网关地址还原原始 URL（调试/展示用）。非 media 前缀原样返回。
 */
export function fromMediaUrl(url: string): string {
  if (!url.startsWith(MEDIA_BASE)) return url
  try {
    return decodeURIComponent(url.slice(MEDIA_BASE.length))
  } catch {
    return url
  }
}

/**
 * 处理一个可能为空的图片 URL：空值返回空，否则改写。
 * 供模板 `:src` 绑定的 computed 使用（youtubeThumb 等可能为 null）。
 */
export function mediaUrlOrEmpty(url: string | null | undefined): string {
  if (!url) return ''
  return toMediaUrl(url)
}
