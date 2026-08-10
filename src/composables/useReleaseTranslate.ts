import { ref, watch } from 'vue'
import { translateRelease, type ReleaseInfo } from '../api/releases'
import { t } from '../i18n'
import { track } from './useUsageTracking'

/**
 * 「翻译」操作状态机：翻译中状态、调用命令、翻译完成监听。
 *
 * 收敛了 ReleaseItem 卡片与 ReleaseDetailModal 弹窗中逐段复制的实现
 * （注释曾自认「与 ReleaseItem 相同规则」）——翻译中状态的复位、
 * body_translated 从无到有的监听只有这一份，组件差异通过回调注入。
 */
export function useReleaseTranslate(opts: {
  release: () => ReleaseInfo
  showToast?: (msg: string) => void
  /** 翻译开始前（translating 已置 true），如关闭右键菜单、切换视图 */
  onStart?: () => void
  /** 翻译命令成功，如 emit('update') 触发列表刷新 */
  onSuccess?: () => void
  /** 翻译失败（translating 已复位），如弹窗回退视图 */
  onError?: () => void
  /** body_translated 从无到有（成功落库后的响应式生效），如弹窗切到译文视图 */
  onTranslated?: () => void
}) {
  const translating = ref(false)

  async function handleTranslateRelease() {
    const releaseId = opts.release().id
    translating.value = true
    track('release.translate')
    opts.onStart?.()
    try {
      await translateRelease(releaseId)
      opts.onSuccess?.()
    } catch (e: unknown) {
      translating.value = false
      opts.showToast?.(t('release.translate_failed') + (e instanceof Error ? e.message : String(e)))
      opts.onError?.()
    }
  }

  // 翻译完成后清除翻译中状态：无摘要时预览内容由 computed 自动从原文刷新为译文
  watch(() => opts.release().body_translated, (newVal, oldVal) => {
    if (newVal && !oldVal) {
      translating.value = false
      opts.onTranslated?.()
    }
  })

  return { translating, handleTranslateRelease }
}
