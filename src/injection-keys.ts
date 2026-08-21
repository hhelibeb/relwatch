import type { InjectionKey } from 'vue'
import type { Ref } from 'vue'

export const ShowToastKey: InjectionKey<(msg: string) => void> = Symbol('showToast')

/** AI 是否可用（已启用 + 已配置 API key）的全局响应式标记，由 App.vue provide。 */
export const AiEnabledKey: InjectionKey<Ref<boolean>> = Symbol('aiEnabled')

/** Agent 总开关（设置页「AI → Agent」独立于 DeepSeek），由 App.vue provide。 */
export const AgentEnabledKey: InjectionKey<Ref<boolean>> = Symbol('agentEnabled')

/** 唤起 Agent 工作区：各 Tab 的入口按钮调用。payload 为预置引用（插入输入框的实体）。
 * 语义为「打开或聚焦」：面板已打开时仅更新 seed，不关闭。 */
export const AgentWorkspaceKey: InjectionKey<(seed?: AgentWorkspaceSeed) => void> = Symbol('agentWorkspace')

/** Agent 工作区面板当前是否打开（标题栏按钮箭头方向等 UI 状态），由 App.vue provide。 */
export const AgentPanelOpenKey: InjectionKey<Ref<boolean>> = Symbol('agentPanelOpen')

/** 切换 Agent 工作区面板开合：已打开则收回，未打开则展开（全局标题栏按钮用）。 */
export const AgentToggleKey: InjectionKey<() => void> = Symbol('agentToggle')

/** 工作区唤起时的预置内容：可直接插入输入框的实体引用（按钮入口携带当前上下文）。 */
export interface AgentWorkspaceSeed {
  entities?: AgentEntityRefSeed[]
}

/** 与后端 AgentEntityRef 对齐的实体引用（前端先持有，提交时原样传后端）。 */
export interface AgentEntityRefSeed {
  kind: 'source' | 'release'
  id: number
}
