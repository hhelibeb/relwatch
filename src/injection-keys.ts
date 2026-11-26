import type { InjectionKey } from 'vue'
import type { Ref } from 'vue'
import type { AutoReadDetailMode } from './utils'
import type { AppCatalogStore } from './composables/useAppCatalog'

export const AppCatalogKey: InjectionKey<AppCatalogStore> = Symbol('appCatalog')

export const ShowToastKey: InjectionKey<(msg: string) => void> = Symbol('showToast')

/** AI 是否可用（已启用 + 已配置 API key）的全局响应式标记，由 App.vue provide。 */
export const AiEnabledKey: InjectionKey<Ref<boolean>> = Symbol('aiEnabled')

/** 是否显示 AI 重要度（大/中/小）徽标与筛选（设置页「显示重要度」），由 App.vue provide。
 *  仅控制 UI 展示，不影响 ai_importance 生成与「按重要度通知」阈值。
 *  缺省（未 provide，如单组件挂载/测试）取 false，与设置项持久化默认一致。 */
export const ShowImportanceKey: InjectionKey<Ref<boolean>> = Symbol('showImportance')

/** 打开版本详情弹窗时自动标为已读的模式（设置页「常规」），由 App.vue provide。
 *  设置项默认档为 open（仅打开弹窗时）；inject 缺省仍取 off——未接线（单组件挂载/测试）
 *  时不应产生「替用户消未读」的副作用，生产路径始终由 App.vue provide 真实值。 */
export const AutoReadDetailKey: InjectionKey<Ref<AutoReadDetailMode>> = Symbol('autoReadDetail')

/** Agent 总开关（设置页「AI → Agent」独立于 DeepSeek），由 App.vue provide。 */
export const AgentEnabledKey: InjectionKey<Ref<boolean>> = Symbol('agentEnabled')

/** 唤起 Agent 工作区：各 Tab 的入口按钮调用。payload 为预置引用（插入输入框的实体）。
 * 语义为「打开或聚焦」：面板已打开时仅更新 seed，不关闭。 */
export const AgentWorkspaceKey: InjectionKey<(seed?: AgentWorkspaceSeed) => void> = Symbol('agentWorkspace')

/** Agent 工作区面板当前是否打开（标题栏按钮箭头方向等 UI 状态），由 App.vue provide。 */
export const AgentPanelOpenKey: InjectionKey<Ref<boolean>> = Symbol('agentPanelOpen')

/** 切换 Agent 工作区面板开合：已打开则收回，未打开则展开（全局标题栏按钮用）。 */
export const AgentToggleKey: InjectionKey<() => void> = Symbol('agentToggle')

/** 主界面当前页签（sources / releases / logs / settings）。供需要区分「注意力在主界面还是
 *  在其它面板」的组件使用：切换页签即视为用户回到主界面（键盘切换不产生鼠标事件）。 */
export const ActiveTabKey: InjectionKey<Ref<string>> = Symbol('activeTab')

/** 工作区唤起时的预置内容：可直接插入输入框的实体引用（按钮入口携带当前上下文）。 */
export interface AgentWorkspaceSeed {
  entities?: AgentEntityRefSeed[]
}

/** 与后端 AgentEntityRef 对齐的实体引用（前端先持有，提交时原样传后端）。 */
export interface AgentEntityRefSeed {
  kind: 'source' | 'release'
  id: number
}
