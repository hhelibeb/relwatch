<script setup lang="ts">
import { storeToRefs } from 'pinia'
import { t } from '../../i18n'
import {
  runErrorText,
  runEntities,
  runModelLabel,
  runDurationText,
  runStatusLabel,
} from './agentChatUtils'
import type { AgentRunSummary } from '../../api/agent'
import { useAgentStore } from '../../stores/agent'
import AppIcon from '../ui/AppIcon.vue'

const agent = useAgentStore()
const {
  latestRun,
  queueHint,
  queueOccupiedBy,
  queueOccupiedByTitle,
  sessionTitle,
  historyOpen,
  actionsExpanded,
  runs,
  rpcRestartPending,
} = storeToRefs(agent)

function runEntityCount(run: AgentRunSummary): number {
  return runEntities(run).length
}

function isTerminalRun(run: AgentRunSummary): boolean {
  return run.status !== 'pending' && run.status !== 'running'
}
</script>

<template>
  <div v-if="latestRun" class="agent-ws-banner" :class="`status-${latestRun.status}`">
    <span class="agent-ws-banner-status">{{ runStatusLabel(latestRun.status, t) }}</span>
    <span
      v-if="latestRun.status === 'pending' && queueHint"
      class="agent-ws-banner-queue"
      :class="{ clickable: !!queueOccupiedBy }"
      :title="queueHint"
      @click="queueOccupiedBy && agent.switchSession(queueOccupiedBy)"
    >{{ queueOccupiedBy ? t('agent.queue_occupied_by', queueOccupiedByTitle) : queueHint }}</span>
    <span v-if="runErrorText(latestRun, t)" class="agent-ws-banner-error" :title="runErrorText(latestRun, t) ?? ''">{{ runErrorText(latestRun, t) }}</span>
    <span class="agent-ws-banner-text">{{ latestRun.instruction || sessionTitle }}</span>
    <span v-if="latestRun.status === 'running' || latestRun.status === 'pending'" class="agent-ws-banner-spinner" aria-hidden="true"></span>
    <span class="agent-ws-banner-actions">
      <button class="btn-sm" :class="{ active: historyOpen }" :title="t('agent.run_history_title')" @click="historyOpen = !historyOpen">
        {{ t('agent.run_history_title') }}
      </button>
      <template v-if="latestRun.session_path">
        <template v-if="actionsExpanded">
          <button class="btn-sm" :title="t('agent.open_session')" @click="agent.handleOpenSession(latestRun)">{{ t('agent.open_session') }}</button>
          <button class="btn-sm" :title="t('agent.copy_command_hint')" @click="agent.handleCopySessionCommand(latestRun)">{{ t('agent.copy_command') }}</button>
        </template>
        <button
          class="btn-sm agent-ws-banner-toggle"
          :title="actionsExpanded ? t('agent.collapse_actions') : t('agent.expand_actions')"
          @click="actionsExpanded = !actionsExpanded"
        >{{ actionsExpanded ? '>>' : '<<' }}</button>
      </template>
    </span>
  </div>

  <div v-if="rpcRestartPending" class="agent-ws-pending-restart">
    <span class="agent-ws-pending-restart-icon" aria-hidden="true"></span>
    <span class="agent-ws-pending-restart-text">{{ t('agent.config_pending_restart') }}</span>
  </div>

  <div v-if="historyOpen" class="agent-ws-history">
    <div class="agent-ws-history-head">
      <span class="agent-ws-history-title">{{ t('agent.run_history_title') }}</span>
      <button class="agent-ws-history-close" :title="t('release.detail_close')" @click="historyOpen = false">
        <AppIcon name="close-icon" :size="12" />
      </button>
    </div>
    <ul class="agent-ws-history-list">
      <li v-for="r in runs" :key="r.id" class="agent-ws-history-item">
        <span class="agent-ws-history-status" :class="`st-${r.status}`">{{ runStatusLabel(r.status, t) }}</span>
        <span class="agent-ws-history-main">
          <span class="agent-ws-history-instr" :title="r.instruction">{{ r.instruction || sessionTitle }}</span>
          <span class="agent-ws-history-meta">
            {{ runModelLabel(r, t) }} · {{ runDurationText(r, t) }}
            <template v-if="runEntityCount(r) > 0"> · {{ t('agent.run_entities_n', String(runEntityCount(r))) }}</template>
          </span>
        </span>
        <span class="agent-ws-history-actions">
          <button v-if="isTerminalRun(r)" class="btn-sm" :title="t('agent.retry')" @click="agent.handleRetry(r)">{{ t('agent.retry') }}</button>
        </span>
      </li>
      <li v-if="runs.length === 0" class="agent-ws-history-empty">{{ t('agent.run_history_empty') }}</li>
    </ul>
  </div>
</template>

<style scoped>
/* 状态横幅 */
.agent-ws-banner {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 14px;
  font-size: var(--fs-meta);
  min-height: 38px;
  border-bottom: 1px solid var(--border);
  background: var(--bg-subtle);
}
/* 横幅按钮固定高度：避免中文/ASCII 字体行盒差异导致展开/折叠时条高变化 */
.agent-ws-banner .btn-sm {
  height: 24px;
  line-height: 1;
  display: inline-flex;
  align-items: center;
  white-space: nowrap;
}
.agent-ws-banner-status {
  font-weight: 600;
}
.agent-ws-banner-error {
  color: var(--danger);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 240px;
  flex-shrink: 1;
}
.agent-ws-banner-queue {
  color: var(--warning);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 260px;
  flex-shrink: 1;
}
/* 状态色一律走语义 token：运行中/排队=品牌蓝，成功=绿，失败=红，超时=琥珀，取消=中性灰 */
.agent-ws-banner.status-running .agent-ws-banner-status { color: var(--primary); }
.agent-ws-banner.status-pending .agent-ws-banner-status { color: var(--primary); }
.agent-ws-banner.status-success .agent-ws-banner-status { color: var(--success); }
.agent-ws-banner.status-failed .agent-ws-banner-status { color: var(--danger); }
.agent-ws-banner.status-timeout .agent-ws-banner-status { color: var(--warning); }
.agent-ws-banner.status-cancelled .agent-ws-banner-status { color: var(--text-muted); }
.agent-ws-banner-text {
  flex: 1;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.agent-ws-banner-actions {
  display: flex;
  gap: 6px;
}
/* 折叠/展开按钮是控件，层次用颜色而不是叠透明度（叠完 2.9:1） */
.agent-ws-banner-toggle {
  color: var(--text-muted);
}
.agent-ws-banner-toggle:hover {
  color: var(--text);
}
/* 排队提示可点击（被其他会话占用时）：虚线强调 + hover 变 accent */
.agent-ws-banner-queue.clickable {
  cursor: pointer;
  text-decoration: underline;
  text-decoration-style: dotted;
  text-underline-offset: 2px;
}
.agent-ws-banner-queue.clickable:hover {
  color: var(--primary);
}
.agent-ws-banner-actions .btn-sm.active {
  background: var(--primary-soft-bg);
  border-color: var(--primary-soft-border);
  color: var(--primary-soft-text);
}
.agent-ws-banner-spinner {
  width: 12px;
  height: 12px;
  border: 2px solid var(--primary-soft-border);
  border-top-color: var(--primary);
  border-radius: 50%;
  animation: agent-ws-spin 0.8s linear infinite;
  flex-shrink: 0;
}
@keyframes agent-ws-spin {
  to { transform: rotate(360deg); }
}

/* 配置推迟生效提示 */
.agent-ws-pending-restart {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 4px 14px;
  font-size: var(--fs-caption);
  color: var(--primary-soft-text);
  background: var(--primary-soft-bg);
  border-bottom: 1px solid var(--border);
}
.agent-ws-pending-restart-icon {
  flex-shrink: 0;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--primary);
  animation: agent-ws-pulse 1.4s ease-in-out infinite;
}
.agent-ws-pending-restart-text {
  min-width: 0;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
@keyframes agent-ws-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.3; }
}

/* 运行历史面板：覆盖整个聊天区的浮层（顶部含标题与关闭；依托 .agent-ws-chat 的 relative 定位） */
.agent-ws-history {
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
  bottom: 0;
  z-index: var(--z-sticky);
  background: var(--bg);
  display: flex;
  flex-direction: column;
}
.agent-ws-history-head {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 12px;
  border-bottom: 1px solid var(--border);
  flex-shrink: 0;
}
.agent-ws-history-title {
  font-size: var(--fs-meta);
  font-weight: 600;
}
.agent-ws-history-close {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  padding: 0;
  border: none;
  background: none;
  color: var(--text-muted);
  cursor: pointer;
  border-radius: var(--radius-sm);
}
.agent-ws-history-close:hover {
  background: var(--bg-hover);
  color: var(--text);
}
.agent-ws-history-close svg {
  width: 12px;
  height: 12px;
}
.agent-ws-history-list {
  list-style: none;
  margin: 0;
  padding: 8px 10px;
  overflow-y: auto;
  flex: 1;
  min-height: 0;
  display: flex;
  flex-direction: column;
  gap: 5px;
}
.agent-ws-history-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 7px 9px;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg-subtle);
}
.agent-ws-history-status {
  font-size: var(--fs-caption);
  font-weight: 600;
  flex-shrink: 0;
  min-width: 34px;
}
.agent-ws-history-status.st-running,
.agent-ws-history-status.st-pending {
  color: var(--primary);
}
.agent-ws-history-status.st-success {
  color: var(--success);
}
.agent-ws-history-status.st-failed {
  color: var(--danger);
}
.agent-ws-history-status.st-timeout {
  color: var(--warning);
}
.agent-ws-history-status.st-cancelled {
  color: var(--text-muted);
}
.agent-ws-history-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.agent-ws-history-instr {
  font-size: var(--fs-meta);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.agent-ws-history-meta {
  font-size: var(--fs-caption);
  color: var(--text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font-family: var(--mono-font);
}
.agent-ws-history-actions {
  flex-shrink: 0;
}
.agent-ws-history-actions .btn-sm {
  padding: 1px 7px;
  font-size: var(--fs-caption);
}
.agent-ws-history-empty {
  padding: 20px;
  text-align: center;
  font-size: var(--fs-meta);
  color: var(--text-muted);
}
</style>
