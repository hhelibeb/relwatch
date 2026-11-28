<script setup lang="ts">
import { storeToRefs } from 'pinia'
import { t } from '../../i18n'
import { formatDate } from '../../utils'
import { useAgentStore } from '../../stores/agent'
import AppIcon from '../ui/AppIcon.vue'

const agent = useAgentStore()
const {
  sidebarOpen,
  sessions,
  visibleSessions,
  activeKey,
  sessionQuery,
  renamingKey,
  renameInput,
  openMenuKey,
  sessionMenuStyle,
} = storeToRefs(agent)
</script>

<template>
  <aside class="agent-ws-sidebar" :class="{ collapsed: !sidebarOpen }">
    <div class="agent-ws-sidebar-title">{{ t('agent.session_list') }}</div>
    <div v-if="sessions.length > 1" class="agent-ws-session-search">
      <input
        v-model="sessionQuery"
        class="agent-ws-session-search-input"
        type="search"
        :placeholder="t('agent.session_search_placeholder')"
        :aria-label="t('agent.session_search')"
      />
    </div>
    <ul class="agent-ws-session-list" @scroll="agent.onSessionListScroll()">
      <li
        v-for="s in visibleSessions"
        :key="s.key"
        class="agent-ws-session-item"
        :class="{ active: s.key === activeKey, draft: s.draft }"
        :title="s.title"
        @click="agent.switchSession(s.key)"
      >
        <input
          v-if="renamingKey === s.key"
          :ref="(el) => agent.setRenameEl(el, s.key)"
          v-model="renameInput"
          class="agent-ws-rename-input"
          type="text"
          data-esc-local
          :placeholder="t('agent.session_rename_placeholder')"
          @click.stop
          @keydown.enter.prevent="agent.commitRename()"
          @keydown.esc.prevent="agent.cancelRename()"
          @blur="agent.commitRename()"
        />
        <template v-else>
          <span class="agent-ws-session-name">
            {{ s.title }}
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
          <button
            :ref="(el) => agent.setSessionMoreEl(el, s.key)"
            class="agent-ws-session-more"
            :title="t('agent.session_menu')"
            @click.stop="agent.toggleSessionMenu(s.key)"
          >
            <svg viewBox="0 0 16 16"><circle cx="3.5" cy="8" r="1.1" fill="currentColor"/><circle cx="8" cy="8" r="1.1" fill="currentColor"/><circle cx="12.5" cy="8" r="1.1" fill="currentColor"/></svg>
          </button>
        </template>
      </li>
      <li v-if="sessions.length === 0" class="agent-ws-session-empty">{{ t('agent.session_empty') }}</li>
      <li v-else-if="visibleSessions.length === 0" class="agent-ws-session-empty">{{ t('agent.session_no_match') }}</li>
    </ul>
    <button v-if="sessions.length > 1" class="agent-ws-session-clear" :title="t('agent.session_clear')" @click="agent.handleClearSessions()">
      <AppIcon name="close-icon" :size="10" />
      {{ t('agent.session_clear') }}
    </button>
  </aside>

  <Teleport to="body">
    <div v-if="openMenuKey" class="agent-ws-menu agent-ws-session-menu" :style="sessionMenuStyle" @click.stop>
      <button class="agent-ws-menu-item" @click="agent.startRename(openMenuKey!)">
        <span class="agent-ws-menu-main">{{ t('agent.session_rename') }}</span>
      </button>
      <button class="agent-ws-menu-item" @click="agent.handleExportSession(openMenuKey!, 'md')">
        <span class="agent-ws-menu-main">{{ t('agent.session_export_md') }}</span>
      </button>
      <button class="agent-ws-menu-item" @click="agent.handleExportSession(openMenuKey!, 'json')">
        <span class="agent-ws-menu-main">{{ t('agent.session_export_json') }}</span>
      </button>
      <button class="agent-ws-menu-item danger" @click="agent.handleDeleteFromMenu()">
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
  transition: width var(--dur-state) var(--ease-out), opacity var(--dur-state) var(--ease-out), border-left-width var(--dur-state) var(--ease-out);
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
  font-size: var(--fs-caption);
  font-weight: 600;
  color: var(--text-muted);
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
  border-radius: var(--radius);
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
  background: var(--primary-soft-bg);
  border-color: var(--primary-soft-border);
}
.agent-ws-session-name {
  font-size: var(--fs-meta);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  padding-right: 16px;
}
/* 未提交草稿会话（新建即登记）：弱化样式以示「还没对话」 */
.agent-ws-session-item.draft .agent-ws-session-name {
  color: var(--text-muted);
  font-style: italic;
}
/* 运行状态点：执行中（蓝）/ 排队第 N 位（橙），全局队列驱动 */
.agent-ws-session-dot {
  display: inline-block;
  margin-left: 4px;
  padding: 0 4px;
  font-size: var(--fs-caption);
  line-height: 14px;
  border-radius: var(--radius-xs);
  vertical-align: 1px;
  white-space: nowrap;
}
.agent-ws-session-dot.st-running {
  color: var(--primary-soft-text);
  background: var(--primary-soft-bg);
}
.agent-ws-session-dot.st-pending {
  color: var(--warning-soft-text);
  background: var(--warning-soft-bg);
}
/* 「已恢复」标记：磁盘发现补入的会话（localStorage 索引曾丢失） */
.agent-ws-session-badge {
  display: inline-block;
  margin-left: 4px;
  padding: 0 4px;
  font-size: var(--fs-caption);
  line-height: 14px;
  vertical-align: 1px;
  color: var(--warning-soft-text);
  background: var(--warning-soft-bg);
  border-radius: var(--radius-xs);
  white-space: nowrap;
}
.agent-ws-session-time {
  font-size: var(--fs-caption);
  color: var(--text-muted);
}
.agent-ws-session-empty {
  padding: 12px 8px;
  font-size: var(--fs-meta);
  color: var(--text-muted);
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
  font-size: var(--fs-caption);
  font-family: inherit;
  color: var(--text);
  background: var(--bg);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  outline: none;
}
.agent-ws-session-search-input:focus {
  border-color: var(--primary);
  box-shadow: var(--focus-ring);
}
.agent-ws-session-search-input::placeholder {
  color: var(--text-muted);
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
 * 脱离文档流盖在聊天区上层完整展示（与 RPC 状态菜单同一档：--z-menu）。
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
  z-index: var(--z-menu);
}
.agent-ws-menu-item.danger .agent-ws-menu-main {
  color: var(--danger);
}
.agent-ws-menu-item.danger:hover {
  background: var(--danger-soft-bg);
}
.agent-ws-rename-input {
  width: 100%;
  box-sizing: border-box;
  height: 22px;
  padding: 0 6px;
  font-size: var(--fs-meta);
  font-family: inherit;
  color: var(--text);
  background: var(--bg);
  border: 1px solid var(--primary);
  border-radius: var(--radius-sm);
  outline: none;
}
.agent-ws-session-clear {
  margin: 2px 8px 10px;
  padding: 6px 8px;
  font-size: var(--fs-caption);
  border: 1px solid var(--border);
  border-radius: var(--radius);
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
  color: var(--danger);
  border-color: color-mix(in srgb, var(--danger) 40%, transparent);
  background: var(--bg-hover);
}
.agent-ws-session-clear svg {
  width: 10px;
  height: 10px;
}
</style>
