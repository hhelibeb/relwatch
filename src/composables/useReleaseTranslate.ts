import { ref, watch, onScopeDispose } from 'vue'
import { events } from '../bindings'
import { translateRelease, type ReleaseInfo } from '../api/releases'
import { t } from '../i18n'
import { track } from './useUsageTracking'

/** 流式分片合帧窗口（ms）：与 Agent 工作区同口径（见 useAgentChat 的 RPC_FLUSH_MS）。
 *  逐分片渲染会让主线程饱和（每片一次全量 Markdown 重解析 + 组件重渲染），
 *  定窗合并把渲染次数从分片频率降到 ≤20 次/秒。 */
const STREAM_FLUSH_MS = 50

/**
 * 「翻译」操作状态机：翻译中状态、调用命令、翻译完成监听。
 *
 * 收敛了 ReleaseItem 卡片与 ReleaseDetailModal 弹窗中逐段复制的实现：
 * 翻译中状态的复位、body_translated 从无到有的监听只有这一份，组件差异通过回调注入。
 *
 * `stream` 打开后额外接收后端实时投递的译文分片（内存缓冲，不落库）：译文未落库前
 * 内容不在 `props.release` 里，调用方需要缓冲才能边收边显示。
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
  /**
   * 是否接收流式译文分片（默认 false）。
   *
   * 只有详情弹窗需要：卡片在虚拟滚动下每行一个监听器，事件分发本身就是开销，
   * 而卡片也没有任何位置显示增量译文。
   */
  stream?: boolean
}) {
  const translating = ref(false)
  /** 流式译文缓冲（未落库）：翻译进行中与失败残留都靠它显示，落库后交回 props。 */
  const streamText = ref('')

  // 发起本次流式的 release id：翻到别的版本后到达的分片必须丢弃，
  // 否则旧条的半截译文会挂在新条上
  let streamId: ReleaseInfo['id'] | null = null
  let unlisten: (() => void) | null = null
  let pending: string[] = []
  let flushTimer: ReturnType<typeof setTimeout> | undefined

  function flushPending() {
    if (flushTimer !== undefined) {
      clearTimeout(flushTimer)
      flushTimer = undefined
    }
    if (pending.length === 0) return
    streamText.value += pending.join('')
    pending = []
  }

  function handleChunk(delta: string) {
    pending.push(delta)
    if (flushTimer === undefined) {
      flushTimer = setTimeout(flushPending, STREAM_FLUSH_MS)
    }
  }

  /** 订阅分片事件并重置缓冲。命令返回前一直有效。 */
  async function startStream() {
    if (!opts.stream) return
    streamText.value = ''
    pending = []
    const id = opts.release().id
    streamId = id
    const off = await events.releaseTranslateChunk.listen((e) => {
      if (streamId === null || e.payload.release_id !== streamId) return
      handleChunk(e.payload.delta)
    })
    // 注册是异步的：await 期间可能已经收尾（组件卸载、切到别的版本），此时没人会
    // 再来退订这个监听器，只能当场退掉
    if (streamId === id) unlisten = off
    else off()
  }

  /** 收尾：先落盘未处理的分片再退订。
   *
   *  失败分支要保留已显示的部分文本，定窗未到就丢弃会让最后几十毫秒的内容凭空消失；
   *  退订 + 清 id 则保证翻版本/关弹窗后不再有事件写进这条的缓冲。
   *  幂等：成功与失败两条出口都调。 */
  function stopStream() {
    flushPending()
    streamId = null
    unlisten?.()
    unlisten = null
  }

  // 切到另一个版本：缓冲属于上一条，必须清空，否则新条的译文视图会显示上一条的残留
  watch(() => opts.release().id, () => {
    streamId = null
    pending = []
    streamText.value = ''
  })

  onScopeDispose(() => stopStream())

  async function handleTranslateRelease() {
    // 重入保护：并发调用会让后一次覆盖 unlisten（前一个监听器再无人退订，
    // 且两路分片会双份累进同一条缓冲）。UI 上按钮已按 translating 禁用，
    // 这里兜住其余入口。
    if (translating.value) return
    const releaseId = opts.release().id
    translating.value = true
    track('release.translate')
    opts.onStart?.()
    try {
      await startStream()
      await translateRelease(releaseId)
      // 译文已落库：退订但保留缓冲，等内容从 props 响应式生效（见下方 watch）后再清，
      // 中间这一小段用缓冲兜住，避免「正在翻译」提示与落库内容之间闪一下空白
      stopStream()
      opts.onSuccess?.()
    } catch (e: unknown) {
      // 先收尾再回调：onError 要能看到完整的部分文本（调用方据此决定是否回退视图）
      stopStream()
      translating.value = false
      opts.showToast?.(t('release.translate_failed') + (e instanceof Error ? e.message : String(e)))
      opts.onError?.()
    }
  }

  // 翻译完成后清除翻译中状态：无摘要时预览内容由 computed 自动从原文刷新为译文
  watch(() => opts.release().body_translated, (newVal, oldVal) => {
    if (newVal && !oldVal) {
      translating.value = false
      // 落库译文生效：流式残留（含未合帧的分片）必须清掉，
      // 否则同一份内容会在缓冲与 props 之间重复渲染
      pending = []
      streamText.value = ''
      opts.onTranslated?.()
    }
  })

  return { translating, streamText, handleTranslateRelease }
}
