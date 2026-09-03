<!-- 会话侧栏（A 域展示层）：搜索 / 列表 / 重命名输入 / ⋯菜单（Teleport 到 body）。
     状态与动作由编排层的 useAgentSessions 持有，本组件纯 props/emit 展示。 -->
<script setup lang="ts">
import { t } from '../../i18n'
import { formatDate } from '../../utils'
import type { ComponentPublicInstance } from 'vue'
import type { SessionMeta } from './useAgentSessions'

export interface SidebarSession extends SessionMeta {
  state: { status: string; position: number } | null
}

defineProps<{
  /** 侧栏折叠状态（class 透传给多根模板无根可落，改为显式 prop） */
  sidebarOpen: boolean
  sessions: SessionMeta[]
  visibleSessions: SidebarSession[]
  activeKey: string
  sessionQuery: string
  renamingKey: string | null
  renameInput: string
  openMenuKey: string | null
  sessionMenuStyle: { left: string; top: string }
  /** 重命名输入框 / ⋯按钮元素回填（composable 的 renameEl / sessionMoreEl 锚点），
   *  key 标识属于哪个会话。用普通函数 ref 形式从子组件模板回填。 */
  setRenameEl: (el: Element | ComponentPublicInstance | null, key: string) => void
  setSessionMoreEl: (el: Element | ComponentPublicInstance | null, key: string) => void
}>()

const emit = defineEmits<{
  'update:sessionQuery': [value: string]
  'update:renameInput': [value: string]
  switch: [key: string]
  scrollList: []
  commitRename: []
  cancelRename: []
  toggleMenu: [key: string]
  exportSession: [key: string, format: 'md' | 'json']
  rename: [key: string]
  deleteFromMenu: []
  clearSessions: []
}>()
</script>

<template>
  <aside class="agent-ws-sidebar" :class="{ collapsed: !sidebarOpen }">
    <div class="agent-ws-sidebar-title">{{ t('agent.session_list') }}</div>
    <!-- 会话搜索：标题自动取首条指令前 40 字，高度相似且上限 200 条，
         没有搜索就只能靠「清理旧会话」一刀切 -->
    <div v-if="sessions.length > 1" class="agent-ws-session-search">
      <input
        :value="sessionQuery"
        class="agent-ws-session-search-input"
        type="search"
        :placeholder="t('agent.session_search_placeholder')"
        :aria-label="t('agent.session_search')"
        @input="emit('update:sessionQuery', ($event.target as HTMLInputElement).value)"
      />
    </div>
    <ul class="agent-ws-session-list" @scroll="emit('scrollList')">
      <li
        v-for="s in visibleSessions"
        :key="s.key"
        class="agent-ws-session-item"
        :class="{ active: s.key === activeKey, draft: s.draft }"
        :title="s.title"
        @click="emit('switch', s.key)"
      >
        <!-- 重命名编辑态：Enter 提交 / Esc 取消 / 失焦提交 -->
        <input
          v-if="renamingKey === s.key"
          :ref="(el) => setRenameEl(el, s.key)"
          :value="renameInput"
          class="agent-ws-rename-input"
          type="text"
          :placeholder="t('agent.session_rename_placeholder')"
          @input="emit('update:renameInput', ($event.target as HTMLInputElement).value)"
          @click.stop
          @keydown.enter.prevent="emit('commitRename')"
          @keydown.esc.prevent="emit('cancelRename')"
          @blur="emit('commitRename')"
        />
        <template v-else>
          <span class="agent-ws-session-name">
            {{ s.title }}
            <!-- 运行状态点：执行中（蓝）/ 排队第 N 位（橙）——全局队列驱动（评审 1.3） -->
            <span
              v-if="s.state"
              class="agent-ws-session-dot"
              :class="`st-${s.state.status}`"
              :title="s.state.status === 'running' ? t('agent.session_running_hint') : t('agent.session_queued_hint', String(s.state.position))"
            >{{ s.state.status === 'running' ? t('agent.status_running') : t('agent.queue_position', String(s.state.position)) }}</span>
            <span v-if="s.recovered" class="agent-ws-session-badge" :title="t('agent.session_recovered_hint')">
              {{ t('agent.session_recovered') }}
            </span>
          </span>
          <span class="agent-ws-session-time">{{ formatDate(new Date(s.updatedAt).toISOString()) }}</span>
          <!-- ⋯ 菜单：重命名 / 导出 md / 导出 json / 删除（菜单浮层在下方 Teleport，见 agent-ws-session-menu） -->
          <button
            :ref="(el) => setSessionMoreEl(el, s.key)"
            class="agent-ws-session-more"
            :title="t('agent.session_menu')"
            @click.stop="emit('toggleMenu', s.key)"
          >
            <svg viewBox="0 0 16 16"><circle cx="3.5" cy="8" r="1.1" fill="currentColor"/><circle cx="8" cy="8" r="1.1" fill="currentColor"/><circle cx="12.5" cy="8" r="1.1" fill="currentColor"/></svg>
          </button>
        </template>
      </li>
      <li v-if="sessions.length === 0" class="agent-ws-session-empty">{{ t('agent.session_empty') }}</li>
      <li v-else-if="visibleSessions.length === 0" class="agent-ws-session-empty">{{ t('agent.session_no_match') }}</li>
    </ul>
    <button v-if="sessions.length > 1" class="agent-ws-session-clear" :title="t('agent.session_clear')" @click="emit('clearSessions')">
      <svg viewBox="0 0 16 16"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" fill="none" /></svg>
      {{ t('agent.session_clear') }}
    </button>
  </aside>

  <!-- 会话 ⋯ 菜单：Teleport 到 body 后以 ⋯ 按钮为锚 fixed 定位。
       侧边栏仅 140px 宽且 overflow:hidden，absolute 定位的菜单超宽部分会被裁剪看不到；
       脱离文档流浮在聊天区上层完整展示（与 RPC 状态菜单同一策略，z-index 对齐 10002）。 -->
  <Teleport to="body">
    <div v-if="openMenuKey" class="agent-ws-menu agent-ws-session-menu" :style="sessionMenuStyle" @click.stop>
      <button class="agent-ws-menu-item" @click="emit('rename', openMenuKey!)">
        <span class="agent-ws-menu-main">{{ t('agent.session_rename') }}</span>
      </button>
      <button class="agent-ws-menu-item" @click="emit('exportSession', openMenuKey!, 'md')">
        <span class="agent-ws-menu-main">{{ t('agent.session_export_md') }}</span>
      </button>
      <button class="agent-ws-menu-item" @click="emit('exportSession', openMenuKey!, 'json')">
        <span class="agent-ws-menu-main">{{ t('agent.session_export_json') }}</span>
      </button>
      <button class="agent-ws-menu-item danger" @click="emit('deleteFromMenu')">
        <span class="agent-ws-menu-main">{{ t('agent.delete_session') }}</span>
      </button>
    </div>
  </Teleport>
</template>

<style scoped>
/* 会话侧栏：位于聊天区右侧（次要内容靠边，不挡主界面与对话之间）
 * collapsed 时宽度收缩为 0，聊天区占满整个工作区 */
.agent-ws-sidebar {
  width: 140px;
  flex-shrink: 0;
  border-left: 1px solid var(--border);
  background: var(--bg-subtle);
  display: flex;
  flex-direction: column;
  min-height: 0;
  overflow: hidden;
  transition: width 0.18s ease, opacity 0.18s ease, border-left-width 0.18s ease;
}
.agent-ws-sidebar.collapsed {
  width: 0;
  opacity: 0;
  border-left-width: 0;
}
.agent-ws-sidebar > * {
  flex-shrink: 0;
}
.agent-ws-sidebar .agent-ws-session-list {
  flex: 1;
  min-height: 0;
}
.agent-ws-sidebar-title {
  padding: 10px 12px 6px;
  font-size: 11px;
  font-weight: 600;
  opacity: 0.55;
  letter-spacing: 0.04em;
}
.agent-ws-session-list {
  list-style: none;
  margin: 0;
  padding: 4px 6px 10px;
  overflow-y: auto;
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.agent-ws-session-item {
  position: relative;
  padding: 7px 8px;
  border-radius: 7px;
  cursor: pointer;
  display: flex;
  flex-direction: column;
  gap: 2px;
  border: 1px solid transparent;
}
.agent-ws-session-item:hover {
  background: var(--bg-hover);
}
.agent-ws-session-item.active {
  background: rgba(46, 111, 208, 0.12);
  border-color: rgba(46, 111, 208, 0.35);
}
.agent-ws-session-name {
  font-size: 12px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  padding-right: 16px;
}
/* 未提交草稿会话（新建即登记，评审 1.2）：弱化样式以示「还没对话」 */
.agent-ws-session-item.draft .agent-ws-session-name {
  opacity: 0.65;
  font-style: italic;
}
/* 运行状态点：执行中（蓝）/ 排队第 N 位（橙），全局队列驱动（评审 1.3） */
.agent-ws-session-dot {
  display: inline-block;
  margin-left: 4px;
  padding: 0 4px;
  font-size: 9px;
  line-height: 14px;
  border-radius: 3px;
  vertical-align: 1px;
  white-space: nowrap;
}
.agent-ws-session-dot.st-running {
  color: #2e6fd0;
  background: rgba(46, 111, 208, 0.14);
}
.agent-ws-session-dot.st-pending {
  color: #b0882e;
  background: rgba(214, 158, 46, 0.16);
}
/* 「已恢复」标记：磁盘发现补入的会话（localStorage 索引曾丢失） */
.agent-ws-session-badge {
  display: inline-block;
  margin-left: 4px;
  padding: 0 4px;
  font-size: 9px;
  line-height: 14px;
  vertical-align: 1px;
  color: #8a6d1f;
  background: rgba(214, 158, 46, 0.16);
  border-radius: 3px;
  white-space: nowrap;
}
.agent-ws-session-time {
  font-size: 10px;
  opacity: 0.5;
}
.agent-ws-session-empty {
  padding: 12px 8px;
  font-size: 12px;
  opacity: 0.5;
  text-align: center;
}

/* 会话搜索框 */
.agent-ws-session-search {
  padding: 0 8px 6px;
}
.agent-ws-session-search-input {
  width: 100%;
  box-sizing: border-box;
  height: 24px;
  padding: 0 8px;
  font-size: 11px;
  font-family: inherit;
  color: var(--text);
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: 6px;
  outline: none;
}
.agent-ws-session-search-input:focus {
  border-color: #2e6fd0;
}
.agent-ws-session-search-input::placeholder {
  color: var(--text-muted);
  opacity: 0.6;
}

/* 会话 ⋯ 更多菜单（重命名 / 导出 / 删除）*/
.agent-ws-session-more {
  position: absolute;
  top: 6px;
  right: 6px;
  width: 16px;
  height: 16px;
  display: none;
  align-items: center;
  justify-content: center;
  border: none;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  border-radius: 50%;
  padding: 0;
}
.agent-ws-session-item:hover .agent-ws-session-more {
  display: flex;
}
.agent-ws-session-more:hover {
  color: var(--text);
  background: var(--bg-hover);
}
.agent-ws-session-more svg {
  width: 12px;
  height: 12px;
}
/* 会话 ⋯ 菜单：Teleport 到 body 后 fixed 定位，坐标由 toggleSessionMenu 计算。
 * 侧边栏宽仅 140px 且 overflow:hidden，absolute 定位会被裁剪显示不全；
 * 脱离文档流盖在聊天区上层完整展示（与 RPC 状态菜单同一策略，z-index 对齐 10002）。
 * 双类选择器抬高特异性：基类 .agent-ws-menu 在 agent-shared.css（非 scoped），单类时其
 * bottom/left/right/max-height/overflow 会反杀这里的重置（top 与 bottom 双锚
 * 同样把高度拉伸成 0）——与 agent-ws-menu-rpc 同一策略。 */
.agent-ws-menu.agent-ws-session-menu {
  position: fixed;
  top: auto;
  right: auto;
  bottom: auto;
  left: auto;
  min-width: 148px;
  max-height: none;
  overflow-y: visible;
  z-index: 10002;
}
.agent-ws-menu-item.danger .agent-ws-menu-main {
  color: #d64545;
}
.agent-ws-menu-item.danger:hover {
  background: rgba(214, 69, 69, 0.08);
}
.agent-ws-rename-input {
  width: 100%;
  box-sizing: border-box;
  height: 22px;
  padding: 0 6px;
  font-size: 12px;
  font-family: inherit;
  color: var(--text);
  background: var(--bg);
  border: 1px solid #2e6fd0;
  border-radius: 5px;
  outline: none;
}
.agent-ws-session-clear {
  margin: 2px 8px 10px;
  padding: 6px 8px;
  font-size: 11px;
  border: 1px solid var(--border);
  border-radius: 7px;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 5px;
  flex-shrink: 0;
}
.agent-ws-session-clear:hover {
  color: #d64545;
  border-color: rgba(214, 69, 69, 0.4);
  background: var(--bg-hover);
}
.agent-ws-session-clear svg {
  width: 10px;
  height: 10px;
}
</style>
