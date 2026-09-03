// ── 模型选择（D 域：scope model + 当前激活模型 + 单次覆盖 + 下拉菜单）──
// selectedModel 按会话记住（存 SessionMeta.model，经 onPersistModel 回调由编排层
// 落库，模型域不直接 import 会话域）；null =「默认 - 跟随 pi 当前」。
// oneShotModel 只作用于下一次提交，提交后即清空、自动回落会话默认（消费在提交路径）。
import { computed, ref } from 'vue'
import type { AgentModelRef, RpcAvailableModel } from '../../api/agent'
import { t } from '../../i18n'
import type { SessionSwitchMode } from './useAgentSessions'

export function useAgentModels(deps: {
  /** 会话级选择落库（写 SessionMeta.model 并持久化）；由编排层转调会话域 */
  onPersistModel: (model: AgentModelRef | null) => void
  /** 模型菜单打开时收起 skill / entity 菜单（同屏互斥，原 toggleModelMenu 行为） */
  onMenuOpen?: () => void
}) {
  const availableModels = ref<RpcAvailableModel[]>([])
  const currentModel = ref<RpcAvailableModel | null>(null)
  const selectedModel = ref<AgentModelRef | null>(null)

  // ── 单次模型覆盖（评审「单次模型覆盖」）──
  // 会话级选模型的语义是「这个会话以后都用 X」，改一次会连带影响后续所有轮次；
  // 而真实需求常常只是「这条用便宜模型试一下」。
  // oneShotModel 只作用于下一次提交，提交后即清空、自动回落会话默认——
  // 不写 SessionMeta，因此不会污染会话的长期选择。
  const oneShotModel = ref<AgentModelRef | null>(null)
  const modelOnce = ref(false)

  /** 实际用于下次提交的模型：单次覆盖优先，否则用会话级选择。 */
  const effectiveModel = computed<AgentModelRef | null>(() => oneShotModel.value ?? selectedModel.value)

  const showModelMenu = ref(false)
  const modelMenuIndex = ref(0)

  /** 模型可读名（name 优先，回退 id）。 */
  function modelLabel(m: RpcAvailableModel | AgentModelRef): string {
    const name = 'name' in m ? (m as RpcAvailableModel).name : undefined
    const id = 'model_id' in m ? m.model_id : (m as RpcAvailableModel).id
    return name && name.length > 0 ? name : id
  }

  /** 唯一键（provider + modelId，modelId 可能自带 provider 前缀，故不拼接 id）。 */
  function modelKey(m: RpcAvailableModel | AgentModelRef): string {
    const id = 'model_id' in m ? m.model_id : (m as RpcAvailableModel).id
    return `${m.provider}\u0000${id}`
  }

  /** 菜单高亮跟随实际生效的模型（含单次覆盖），与实际提交口径一致。 */
  function isModelSelected(m: RpcAvailableModel): boolean {
    return effectiveModel.value?.provider === m.provider && effectiveModel.value.model_id === m.id
  }

  /** 下拉按钮展示：显式选择 → 选中模型名；否则「默认」+ pi 当前模型名。
   *  单次覆盖生效时取 effectiveModel（含一次性选择），与实际提交口径一致。
   *
   *  显式选择时用 availableModels 里的 name 回查可读名：AgentModelRef 只带
   *  provider/model_id（无 name），直接走 modelLabel 会把「DeepSeek V4 Flash」
   *  显示成「deepseek-v4-flash」——同一个模型选前选后两个样子。 */
  const activeModelLabel = computed<string>(() => {
    const m = effectiveModel.value
    if (m) {
      const known = availableModels.value.find((x) => x.provider === m.provider && x.id === m.model_id)
      return known ? modelLabel(known) : m.model_id
    }
    return currentModel.value ? modelLabel(currentModel.value) : t('agent.model_default')
  })

  /** 「默认」副标题：当前 pi 实际将用的模型（provider · id）。 */
  const modelDefaultSub = computed<string>(() =>
    currentModel.value ? `${currentModel.value.provider} · ${currentModel.value.id}` : '',
  )

  function toggleModelMenu() {
    if (showModelMenu.value) {
      showModelMenu.value = false
      return
    }
    deps.onMenuOpen?.()
    showModelMenu.value = true
    modelMenuIndex.value = effectiveModel.value ? availableModels.value.findIndex(isModelSelected) + 1 : 0
  }

  function closeMenu() {
    showModelMenu.value = false
  }

  /** 选模型。
   *  - 「仅本次」开启：只写 oneShotModel，不动 SessionMeta（不改变会话长期选择）；
   *  - 否则：写入当前会话 meta（按会话记住，经 onPersistModel 落库）。
   *  null = 默认（跟随 pi 当前）；清空选择时两种模式都清掉对应槽位。 */
  function pickModel(m: RpcAvailableModel | null) {
    const ref_value = m ? { provider: m.provider, model_id: m.id } : null
    showModelMenu.value = false
    if (modelOnce.value) {
      oneShotModel.value = ref_value
      return
    }
    oneShotModel.value = null
    selectedModel.value = ref_value
    deps.onPersistModel(ref_value)
  }

  /** 切换「仅本次」：关掉时丢弃一次性选择，回落会话默认（避免残留一个隐形覆盖）。 */
  function toggleModelOnce() {
    modelOnce.value = !modelOnce.value
    if (!modelOnce.value) oneShotModel.value = null
  }

  /** 会话切换清空（§4.2 三处清空差异对照表，按 mode 逐条复刻）：
   *  selectedModel 由编排层按 mode 查好传入——switch / delete 读目标会话 meta，
   *  new 硬置 null（新会话无历史选择）；一次性覆盖只属于「这一轮的输入」，
   *  switch / new 清空，delete 后切换保留（现状行为）。 */
  function resetForSessionSwitch(mode: SessionSwitchMode, selected: AgentModelRef | null) {
    selectedModel.value = selected
    if (mode !== 'delete') {
      oneShotModel.value = null
      modelOnce.value = false
    }
  }

  return {
    availableModels,
    currentModel,
    selectedModel,
    oneShotModel,
    modelOnce,
    effectiveModel,
    showModelMenu,
    modelMenuIndex,
    modelLabel,
    modelKey,
    isModelSelected,
    activeModelLabel,
    modelDefaultSub,
    toggleModelMenu,
    closeMenu,
    pickModel,
    toggleModelOnce,
    resetForSessionSwitch,
  }
}
