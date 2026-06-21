import type { InjectionKey } from 'vue'
import type { Ref } from 'vue'

export const ShowToastKey: InjectionKey<(msg: string) => void> = Symbol('showToast')

/** AI 是否可用（已启用 + 已配置 API key）的全局响应式标记，由 App.vue provide。 */
export const AiEnabledKey: InjectionKey<Ref<boolean>> = Symbol('aiEnabled')
