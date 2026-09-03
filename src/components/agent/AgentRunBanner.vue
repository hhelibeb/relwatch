<!-- 最近 run 状态横幅 + 配置推迟生效提示 + 运行历史面板（B 域展示层）。
     状态由编排层的 useAgentChat（latestRun/queueHint/runs）与 useAgentRpc
     （rpcRestartPending）持有，本组件纯 props/emit 展示。 -->
<script setup lang="ts">
import { t } from '../../i18n'
import {
  runErrorText,
  runEntities,
  runModelLabel,
  runDurationText,
} from './agentChatUtils'
import type { AgentRunSummary } from '../../api/agent'

defineProps<{
  latestRun: AgentRunSummary | undefined
  queueHint: string | null
  queueOccupiedBy: string | null
  /** 占用会话标题（跳转入口的文案）：由编排层用 sessionTitleOf 算好传入 */
  queueOccupiedByTitle: string
  sessionTitle: string
  historyOpen: boolean
  actionsExpanded: boolean
  runs: AgentRunSummary[]
  rpcRestartPending: boolean
  /** 占用会话跳转、打开/复制会话入口等需要完整 run 信息的回调 */
  switchSession: (key: string) => void
  openSession: (run: AgentRunSummary) => void
  copySessionCommand: (run: AgentRunSummary) => void
  retry: (run: AgentRunSummary) => void
}>()

const emit = defineEmits<{
  'update:historyOpen': [value: boolean]
  'update:actionsExpanded': [value: boolean]
}>()

/** 状态文案（agent.status_* i18n 键）。 */
function runStatusLabel(status: string): string {
  return t(`agent.status_${status}`)
}

/** 历史面板引用实体数。 */
function runEntityCount(run: AgentRunSummary): number {
  return runEntities(run).length
}

/** 终态判定（success / failed / timeout / cancelled）：历史面板仅对终态 run 展示「重试」。 */
function isTerminalRun(run: AgentRunSummary): boolean {
  return run.status !== 'pending' && run.status !== 'running'
}
</script>

<template>
  <!-- 最近 run 状态横幅 -->
  <div v-if="latestRun" class="agent-ws-banner" :class="`status-${latestRun.status}`">
    <span class="agent-ws-banner-status">{{ runStatusLabel(latestRun.status) }}</span>
    <!-- 排队提示：被其他会话占用时可点击 → 一键跳到占用会话（在那里点「停止」让路） -->
    <span
      v-if="latestRun.status === 'pending' && queueHint"
      class="agent-ws-banner-queue"
      :class="{ clickable: !!queueOccupiedBy }"
      :title="queueHint"
      @click="queueOccupiedBy && switchSession(queueOccupiedBy)"
    >{{ queueOccupiedBy ? t('agent.queue_occupied_by', queueOccupiedByTitle) : queueHint }}</span>
    <span v-if="runErrorText(latestRun, t)" class="agent-ws-banner-error" :title="runErrorText(latestRun, t) ?? ''">{{ runErrorText(latestRun, t) }}</span>
    <span class="agent-ws-banner-text">{{ latestRun.instruction || sessionTitle }}</span>
    <span v-if="latestRun.status === 'running' || latestRun.status === 'pending'" class="agent-ws-banner-spinner" aria-hidden="true"></span>
    <span class="agent-ws-banner-actions">
      <button class="btn-sm" :class="{ active: historyOpen }" :title="t('agent.run_history_title')" @click="emit('update:historyOpen', !historyOpen)">
        {{ t('agent.run_history_title') }}
      </button>
      <template v-if="latestRun.session_path">
        <template v-if="actionsExpanded">
          <button class="btn-sm" :title="t('agent.open_session')" @click="openSession(latestRun)">{{ t('agent.open_session') }}</button>
          <button class="btn-sm" :title="t('agent.copy_command_hint')" @click="copySessionCommand(latestRun)">{{ t('agent.copy_command') }}</button>
        </template>
        <button
          class="btn-sm agent-ws-banner-toggle"
          :title="actionsExpanded ? t('agent.collapse_actions') : t('agent.expand_actions')"
          @click="emit('update:actionsExpanded', !actionsExpanded)"
        >{{ actionsExpanded ? '>>' : '<<' }}</button>
      </template>
    </span>
  </div>

  <!-- 配置推迟生效提示：改了 pi 路径/模型/skill 后有 run 在跑，
       重启被推迟到当前任务结束——不提示的话用户会以为改了没生效（评审 3.8） -->
  <div v-if="rpcRestartPending" class="agent-ws-pending-restart">
    <span class="agent-ws-pending-restart-icon" aria-hidden="true"></span>
    <span class="agent-ws-pending-restart-text">{{ t('agent.config_pending_restart') }}</span>
  </div>

  <!-- 运行历史面板（浮层）：耗时 / 模型 / 状态 / 引用实体 -->
  <div v-if="historyOpen" class="agent-ws-history">
    <div class="agent-ws-history-head">
      <span class="agent-ws-history-title">{{ t('agent.run_history_title') }}</span>
      <button class="agent-ws-history-close" :title="t('release.detail_close')" @click="emit('update:historyOpen', false)">
        <svg viewBox="0 0 16 16"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" fill="none" /></svg>
      </button>
    </div>
    <ul class="agent-ws-history-list">
      <li v-for="r in runs" :key="r.id" class="agent-ws-history-item">
        <span class="agent-ws-history-status" :class="`st-${r.status}`">{{ runStatusLabel(r.status) }}</span>
        <span class="agent-ws-history-main">
          <span class="agent-ws-history-instr" :title="r.instruction">{{ r.instruction || sessionTitle }}</span>
          <span class="agent-ws-history-meta">
            {{ runModelLabel(r, t) }} · {{ runDurationText(r, t) }}
            <template v-if="runEntityCount(r) > 0"> · {{ t('agent.run_entities_n', String(runEntityCount(r))) }}</template>
          </span>
        </span>
        <span class="agent-ws-history-actions">
          <button v-if="isTerminalRun(r)" class="btn-sm" :title="t('agent.retry')" @click="retry(r)">{{ t('agent.retry') }}</button>
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
  font-size: 12px;
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
  color: #d64545;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 240px;
  flex-shrink: 1;
}
.agent-ws-banner-queue {
  color: #b0882e;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  max-width: 260px;
  flex-shrink: 1;
}
.agent-ws-banner.status-running .agent-ws-banner-status { color: #2e6fd0; }
.agent-ws-banner.status-pending .agent-ws-banner-status { color: #2e6fd0; }
.agent-ws-banner.status-success .agent-ws-banner-status { color: #2e9e5b; }
.agent-ws-banner.status-failed .agent-ws-banner-status { color: #d64545; }
.agent-ws-banner.status-timeout .agent-ws-banner-status { color: #d08a2e; }
.agent-ws-banner.status-cancelled .agent-ws-banner-status { color: #8a8a8a; }
.agent-ws-banner-text {
  flex: 1;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  opacity: 0.75;
}
.agent-ws-banner-actions {
  display: flex;
  gap: 6px;
}
.agent-ws-banner-toggle {
  opacity: 0.6;
}
.agent-ws-banner-toggle:hover {
  opacity: 1;
}
/* 排队提示可点击（被其他会话占用时）：虚线强调 + hover 变 accent */
.agent-ws-banner-queue.clickable {
  cursor: pointer;
  text-decoration: underline;
  text-decoration-style: dotted;
  text-underline-offset: 2px;
}
.agent-ws-banner-queue.clickable:hover {
  color: #2e6fd0;
}
.agent-ws-banner-actions .btn-sm.active {
  background: rgba(46, 111, 208, 0.12);
  border-color: rgba(46, 111, 208, 0.35);
  color: #2e6fd0;
}
.agent-ws-banner-spinner {
  width: 12px;
  height: 12px;
  border: 2px solid rgba(46, 111, 208, 0.25);
  border-top-color: #2e6fd0;
  border-radius: 50%;
  animation: agent-ws-spin 0.8s linear infinite;
  flex-shrink: 0;
}
@keyframes agent-ws-spin {
  to { transform: rotate(360deg); }
}

/* 配置推迟生效提示（评审 3.8）*/
.agent-ws-pending-restart {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 4px 14px;
  font-size: 11px;
  color: #2e6fd0;
  background: rgba(46, 111, 208, 0.08);
  border-bottom: 1px solid var(--border);
}
.agent-ws-pending-restart-icon {
  flex-shrink: 0;
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: #2e6fd0;
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
  z-index: 12;
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
  font-size: 12px;
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
  border-radius: 5px;
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
  border-radius: 8px;
  background: var(--bg-subtle);
}
.agent-ws-history-status {
  font-size: 11px;
  font-weight: 600;
  flex-shrink: 0;
  min-width: 34px;
}
.agent-ws-history-status.st-running,
.agent-ws-history-status.st-pending {
  color: #2e6fd0;
}
.agent-ws-history-status.st-success {
  color: #2e9e5b;
}
.agent-ws-history-status.st-failed {
  color: #d64545;
}
.agent-ws-history-status.st-timeout {
  color: #d08a2e;
}
.agent-ws-history-status.st-cancelled {
  color: #8a8a8a;
}
.agent-ws-history-main {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.agent-ws-history-instr {
  font-size: 12px;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.agent-ws-history-meta {
  font-size: 10px;
  opacity: 0.6;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  font-family: var(--mono-font, monospace);
}
.agent-ws-history-actions {
  flex-shrink: 0;
}
.agent-ws-history-actions .btn-sm {
  padding: 1px 7px;
  font-size: 11px;
}
.agent-ws-history-empty {
  padding: 20px;
  text-align: center;
  font-size: 12px;
  opacity: 0.5;
}
</style>
