import { describe, it, expect, vi } from 'vitest'
import { t } from '../i18n'
import type { RpcAvailableModel } from '../api/agent'
import { useAgentModels } from '../components/agent/useAgentModels'

const MODELS: RpcAvailableModel[] = [
  { provider: 'deepseek', id: 'deepseek-v4-flash', name: 'DeepSeek V4 Flash' },
  { provider: 'anthropic', id: 'claude-sonnet-4', name: 'Claude Sonnet 4' },
]

function setup() {
  const onPersistModel = vi.fn()
  const onMenuOpen = vi.fn()
  const api = useAgentModels({ onPersistModel, onMenuOpen })
  return { api, onPersistModel, onMenuOpen }
}

describe('useAgentModels 模型选择', () => {
  it('pickModel：会话级选择写 selectedModel 并经 onPersistModel 落库，一次性槽位清空', () => {
    const { api, onPersistModel } = setup()
    api.pickModel(MODELS[0])
    expect(api.selectedModel.value).toEqual({ provider: 'deepseek', model_id: 'deepseek-v4-flash' })
    expect(api.oneShotModel.value).toBeNull()
    expect(onPersistModel).toHaveBeenCalledWith({ provider: 'deepseek', model_id: 'deepseek-v4-flash' })
    expect(api.showModelMenu.value).toBe(false)
  })

  it('pickModel(null) = 回落默认', () => {
    const { api, onPersistModel } = setup()
    api.pickModel(null)
    expect(api.selectedModel.value).toBeNull()
    expect(onPersistModel).toHaveBeenCalledWith(null)
  })

  it('「仅本次」开启时 pickModel 只写 oneShotModel，不落库不改会话级', () => {
    const { api, onPersistModel } = setup()
    api.modelOnce.value = true
    api.pickModel(MODELS[1])
    expect(api.oneShotModel.value).toEqual({ provider: 'anthropic', model_id: 'claude-sonnet-4' })
    expect(api.selectedModel.value).toBeNull()
    expect(onPersistModel).not.toHaveBeenCalled()
  })

  it('effectiveModel：单次覆盖优先于会话级', () => {
    const { api } = setup()
    api.selectedModel.value = { provider: 'deepseek', model_id: 'deepseek-v4-flash' }
    expect(api.effectiveModel.value).toEqual({ provider: 'deepseek', model_id: 'deepseek-v4-flash' })
    api.oneShotModel.value = { provider: 'anthropic', model_id: 'claude-sonnet-4' }
    expect(api.effectiveModel.value).toEqual({ provider: 'anthropic', model_id: 'claude-sonnet-4' })
  })

  it('toggleModelOnce：关掉时丢弃一次性选择（避免残留隐形覆盖）', () => {
    const { api } = setup()
    api.modelOnce.value = true
    api.oneShotModel.value = { provider: 'p', model_id: 'm' }
    api.toggleModelOnce()
    expect(api.modelOnce.value).toBe(false)
    expect(api.oneShotModel.value).toBeNull()
  })

  it('toggleModelMenu：打开时通知互斥收起 + 高亮定位到实际生效模型；再点关', () => {
    const { api, onMenuOpen } = setup()
    api.availableModels.value = MODELS
    api.toggleModelMenu()
    expect(api.showModelMenu.value).toBe(true)
    expect(onMenuOpen).toHaveBeenCalledTimes(1)
    expect(api.modelMenuIndex.value).toBe(0) // 无显式选择 → 「默认」高亮

    api.selectedModel.value = { provider: 'anthropic', model_id: 'claude-sonnet-4' }
    api.toggleModelMenu() // 关
    api.toggleModelMenu() // 再开
    expect(api.modelMenuIndex.value).toBe(2) // 第二个模型（0=默认，1/2=模型）

    api.toggleModelMenu()
    expect(api.showModelMenu.value).toBe(false)
    expect(onMenuOpen).toHaveBeenCalledTimes(2)
  })

  it('activeModelLabel：显式选择回查可读名；无选择显示 pi 当前模型名，目录为空才落默认文案', () => {
    const { api } = setup()
    api.availableModels.value = MODELS
    // 无显式选择且 currentModel 未知 → 「默认」
    expect(api.activeModelLabel.value).toBe(t('agent.model_default'))

    api.currentModel.value = MODELS[0]
    expect(api.activeModelLabel.value).toBe('DeepSeek V4 Flash')
    expect(api.modelDefaultSub.value).toBe('deepseek · deepseek-v4-flash')

    api.selectedModel.value = { provider: 'anthropic', model_id: 'claude-sonnet-4' }
    expect(api.activeModelLabel.value).toBe('Claude Sonnet 4')

    // 未知模型（目录里没有）：回退 model_id
    api.selectedModel.value = { provider: 'x', model_id: 'y-z' }
    expect(api.activeModelLabel.value).toBe('y-z')
    expect(api.modelDefaultSub.value).toBe('deepseek · deepseek-v4-flash')
  })

  it('modelLabel / modelKey / isModelSelected', () => {
    const { api } = setup()
    expect(api.modelLabel(MODELS[0])).toBe('DeepSeek V4 Flash')
    expect(api.modelLabel({ provider: 'p', model_id: 'm1' })).toBe('m1')
    expect(api.modelKey(MODELS[0])).toBe('deepseek\u0000deepseek-v4-flash')
    // effectiveModel 是 computed（oneShot 优先，否则 selected），经 selectedModel 驱动
    api.selectedModel.value = { provider: 'deepseek', model_id: 'deepseek-v4-flash' }
    expect(api.isModelSelected(MODELS[0])).toBe(true)
    expect(api.isModelSelected(MODELS[1])).toBe(false)
  })
})
