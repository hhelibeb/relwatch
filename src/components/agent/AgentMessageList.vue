<!-- 消息区（B/C 域展示层）：三种气泡形态（user / assistant / toolResult/bash）+
     messageDecorations（实体 chip / skill 徽章 / 失败备注 + 重试 / 超时引导）。
     状态与动作由编排层持有，本组件纯 props/emit 展示；滚动容器经
     scrollRef prop 回填（useAgentChat 的滚动逻辑用）。 -->
<script setup lang="ts">
import { t } from '../../i18n'
import MarkdownContent from '../common/MarkdownContent.vue'
import type { AgentChatMessage, AgentRunSummary } from '../../api/agent'
import type { AgentEntityRefSeed } from '../../injection-keys'
import type { ComponentPublicInstance } from 'vue'
import {
  canRetry,
  isToolError,
  toolCardBody,
  toolCardName,
  bashExitLabel,
} from './agentChatUtils'
import { skillShortName } from '../../utils'

/** user 气泡装饰（useAgentChat.messageDecorations 元素）。 */
export interface MessageDecoration {
  run: AgentRunSummary | undefined
  entities: AgentEntityRefSeed[]
  main: string
  folded: string | null
}

defineProps<{
  /** 滚动容器元素回填（useAgentChat.scrollRef 在编排层，经此函数 ref 绑定） */
  setScrollEl: (el: Element | ComponentPublicInstance | null) => void
  messagesLoading: boolean
  liveCount: number
  displayedMessages: AgentChatMessage[]
  messageDecorations: (MessageDecoration | null)[]
  isLiveMessage: (msg: AgentChatMessage) => boolean
  /** 实体 chip 悬浮提示 + 名称映射（composer 域函数经编排层注入） */
  entityKindLabel: (kind: string) => string
  entityLabel: (e: AgentEntityRefSeed) => string
  handleChipEnter: (e: MouseEvent, text: string) => void
  handleChipMove: (e: MouseEvent) => void
  hideChipTooltip: () => void
  /** 失败备注文案（useAgentChat.runFailedNote） */
  runFailedNote: (run: AgentRunSummary | undefined) => string | null
  retry: (run: AgentRunSummary) => void
  retryEdit: (run: AgentRunSummary) => void
  /** 超时引导（评审 3.6）：输入状态与保存动作在编排层（I 域超时引导） */
  adjustingTimeout: boolean
  timeoutInput: string
}>()

const emit = defineEmits<{
  'update:adjustingTimeout': [value: boolean]
  'update:timeoutInput': [value: string]
  /** 进入调整态：timeoutSecs 在编排层，由编排层把初值写入 timeoutInput */
  startAdjustTimeout: []
  saveTimeout: []
  cancelAdjustTimeout: []
}>()

/** 状态文案（agent.status_* i18n 键）。 */
function runStatusLabel(status: string): string {
  return t(`agent.status_${status}`)
}

function isTimeoutRun(run: AgentRunSummary | undefined): boolean {
  return run?.status === 'timeout'
}

function blockText(blocks: { kind: string; text?: string }[]): string {
  return blocks
    .filter((b) => b.kind === 'text')
    .map((b) => b.text ?? '')
    .join('\n')
}

function toolArgsSummary(args: string): string {
  const t0 = args.trim()
  if (!t0) return t0
  return t0.length > 120 ? t0.slice(0, 120) + '…' : t0
}
</script>

<template>
  <div :ref="(el) => setScrollEl(el)" class="agent-ws-messages">
    <div v-if="messagesLoading && displayedMessages.length === 0 && liveCount === 0" class="agent-ws-hint">{{ t('agent.loading') }}</div>
    <div v-else-if="displayedMessages.length === 0" class="agent-ws-hint agent-ws-hint-empty">
      {{ t('agent.workspace_empty') }}
    </div>
    <template v-else>
      <div
        v-for="(msg, idx) in displayedMessages"
        :key="`${idx}-${msg.timestamp}`"
        class="agent-ws-msg-row"
        :class="`role-${msg.role}`"
      >
        <!-- user 消息：右对齐气泡 -->
        <div v-if="msg.role === 'user'" class="agent-ws-bubble agent-ws-bubble-user">
          <div v-if="messageDecorations[idx]?.run" class="agent-ws-bubble-meta">
            <span v-for="e in messageDecorations[idx]?.entities ?? []" :key="`${e.kind}:${e.id}`" class="agent-ws-chip">
              <span
                class="agent-ws-chip-text"
                @mouseenter="handleChipEnter($event, `${entityKindLabel(e.kind)} · ${entityLabel(e)}`)"
                @mousemove="handleChipMove"
                @mouseleave="hideChipTooltip"
              >{{ entityKindLabel(e.kind) }} · {{ entityLabel(e) }}</span>
            </span>
            <span v-if="messageDecorations[idx]?.run?.skill_path" class="agent-ws-skill-badge">@{{ skillShortName(messageDecorations[idx]?.run?.skill_path ?? '') }}</span>
          </div>
          <p class="agent-ws-msg-text">{{ messageDecorations[idx]?.main || '…' }}</p>
          <details v-if="messageDecorations[idx]?.folded" class="agent-ws-fold agent-ws-fold-prompt">
            <summary>{{ t('agent.prompt_full') }}</summary>
            <pre class="agent-ws-fold-body">{{ messageDecorations[idx]?.folded }}</pre>
          </details>
          <!-- 非成功终态内联备注 + 重试入口：这轮为什么挂了、怎么再来一次，
               都在对话流里可追溯（横幅只显示最近一次 run） -->
          <div
            v-if="canRetry(messageDecorations[idx]?.run)"
            class="agent-ws-run-failed"
            :class="{
              'run-cancelled': messageDecorations[idx]?.run?.status === 'cancelled',
              'run-unknown': messageDecorations[idx]?.run?.status === 'unknown',
            }"
          >
            <span class="agent-ws-run-failed-status">{{ runStatusLabel(messageDecorations[idx]!.run!.status) }}</span>
            <span class="agent-ws-run-failed-text" :title="runFailedNote(messageDecorations[idx]?.run) ?? ''">
              {{ runFailedNote(messageDecorations[idx]?.run) || runStatusLabel(messageDecorations[idx]!.run!.status) }}
            </span>
            <!-- 结果未知（终态事件丢失）：与真失败区分——任务可能已经跑完，
                 直接重跑会重复烧词元、重复副作用（评审 3.1） -->
            <span v-if="messageDecorations[idx]!.run!.status === 'unknown'" class="agent-ws-run-advice">{{ t('agent.unknown_advice') }}</span>
            <span class="agent-ws-run-failed-actions">
              <button class="btn-sm" :title="t('agent.retry')" @click="retry(messageDecorations[idx]!.run!)">
                {{ t('agent.retry') }}
              </button>
              <button class="btn-sm" :title="t('agent.retry_edit')" @click="retryEdit(messageDecorations[idx]!.run!)">
                {{ t('agent.retry_edit') }}
              </button>
            </span>
            <!-- 超时引导（评审 3.6）：行动建议 + 就地调时长（timeout 每次调度重读，无需重启进程） -->
            <template v-if="isTimeoutRun(messageDecorations[idx]?.run)">
              <span class="agent-ws-run-advice">{{ t('agent.timeout_advice') }}</span>
              <span v-if="!adjustingTimeout" class="agent-ws-run-advice-actions">
                <button class="btn-sm" :title="t('agent.timeout_adjust')" @click="emit('startAdjustTimeout')">
                  {{ t('agent.timeout_adjust') }}
                </button>
              </span>
              <span v-else class="agent-ws-run-advice-adjust">
                <input
                  :value="timeoutInput"
                  type="number"
                  min="10"
                  max="3600"
                  class="agent-ws-timeout-input"
                  :placeholder="t('agent.timeout_placeholder')"
                  @input="emit('update:timeoutInput', ($event.target as HTMLInputElement).value)"
                  @keydown.enter.prevent="emit('saveTimeout')"
                />
                <button class="btn-sm" @click="emit('saveTimeout')">{{ t('agent.timeout_save') }}</button>
                <button class="btn-sm" @click="emit('cancelAdjustTimeout')">{{ t('agent.timeout_cancel') }}</button>
              </span>
            </template>
          </div>
        </div>

        <!-- assistant 消息：左对齐，Markdown + 思考/工具折叠 -->
        <div v-else-if="msg.role === 'assistant'" class="agent-ws-bubble agent-ws-bubble-assistant">
          <div v-if="msg.model" class="agent-ws-bubble-model">{{ msg.model }}</div>
          <template v-for="(block, bi) in msg.blocks" :key="bi">
            <MarkdownContent v-if="block.kind === 'text'" :content="block.kind === 'text' ? block.text : ''" :no-cache="isLiveMessage(msg)" />
            <details v-else-if="block.kind === 'thinking'" class="agent-ws-fold agent-ws-fold-thinking">
              <summary>{{ t('agent.thinking') }}</summary>
              <pre class="agent-ws-fold-body">{{ block.kind === 'thinking' ? block.text : '' }}</pre>
            </details>
            <div v-else-if="block.kind === 'toolCall'" class="agent-ws-tool-card">
              <div class="agent-ws-tool-head">
                <svg class="agent-ws-tool-icon"><use href="/icons.svg#terminal-icon" /></svg>
                <span class="agent-ws-tool-name">{{ block.kind === 'toolCall' ? block.name : '' }}</span>
                <span class="agent-ws-tool-tag">{{ t('agent.tool_call') }}</span>
              </div>
              <details v-if="block.kind === 'toolCall' && block.args">
                <summary>{{ t('agent.tool_args') }}</summary>
                <pre class="agent-ws-fold-body">{{ toolArgsSummary(block.kind === 'toolCall' ? block.args : '') }}</pre>
              </details>
            </div>
          </template>
        </div>

        <!-- toolResult：折叠卡片 -->
        <div v-else-if="msg.role === 'toolResult' || msg.role === 'bash'" class="agent-ws-tool-card" :class="{ 'tool-error': isToolError(msg) }">
          <div class="agent-ws-tool-head">
            <svg class="agent-ws-tool-icon"><use href="/icons.svg#terminal-icon" /></svg>
            <span class="agent-ws-tool-name">{{ toolCardName(msg) }}</span>
            <span v-if="msg.role === 'bash'" class="agent-ws-tool-tag">{{ bashExitLabel(msg) }}</span>
            <span v-else class="agent-ws-tool-tag">{{ isToolError(msg) ? t('agent.tool_error') : t('agent.tool_result') }}</span>
          </div>
          <details>
            <summary>{{ t('agent.tool_detail') }}</summary>
            <pre class="agent-ws-fold-body">{{ toolCardBody(msg) }}</pre>
          </details>
        </div>

        <!-- 其他（custom 等）：左对齐文本 -->
        <div v-else class="agent-ws-bubble agent-ws-bubble-assistant">
          <p class="agent-ws-msg-text">{{ blockText(msg.blocks) || '…' }}</p>
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
/* 消息区 */
.agent-ws-messages {
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  padding: 14px;
  display: flex;
  flex-direction: column;
  gap: 10px;
}
.agent-ws-hint {
  text-align: center;
  opacity: 0.6;
  font-size: 13px;
  padding: 24px 0;
}
.agent-ws-hint-empty {
  white-space: pre-line;
  line-height: 1.8;
}
.agent-ws-msg-row {
  display: flex;
  flex-direction: column;
}
.agent-ws-msg-row.role-user {
  align-items: flex-end;
}
.agent-ws-msg-row.role-assistant {
  align-items: flex-start;
}

/* 气泡 */
.agent-ws-bubble {
  max-width: 86%;
  border-radius: 10px;
  padding: 9px 12px;
  font-size: 13px;
  line-height: 1.55;
}
.agent-ws-bubble-user {
  background: rgba(46, 111, 208, 0.13);
  border: 1px solid rgba(46, 111, 208, 0.28);
  border-top-right-radius: 3px;
  color: var(--text);
}
.agent-ws-bubble-assistant {
  /* 覆盖 .agent-ws-bubble 的 max-width:86%——右侧留白由 margin-right 保证与左侧 padding(14px) 一致 */
  max-width: none;
  margin-right: 14px;
  background: var(--surface);
  border: 1px solid var(--border);
  border-top-left-radius: 3px;
}
.agent-ws-bubble-model {
  font-size: 10px;
  opacity: 0.5;
  margin-bottom: 4px;
  font-family: var(--mono-font, monospace);
}
.agent-ws-bubble-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  margin-bottom: 5px;
}
.agent-ws-msg-text {
  margin: 0;
  white-space: pre-wrap;
  word-break: break-word;
}
/* 非成功终态内联备注 + 重试入口（挂在对应 user 气泡下） */
.agent-ws-run-failed {
  display: flex;
  align-items: baseline;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 7px;
  padding: 5px 8px;
  border-radius: 6px;
  border: 1px solid rgba(214, 69, 69, 0.35);
  background: rgba(214, 69, 69, 0.08);
  font-size: 11px;
  line-height: 1.45;
  max-width: 100%;
}
/* 被取消不是错误（用户主动停 / 应用重启清理），用中性色，不伪装成报错 */
.agent-ws-run-failed.run-cancelled {
  border-color: var(--border);
  background: var(--bg-subtle);
}
/* 结果未知：既不是失败（可能跑成了），也不是取消（不是用户主动停的）——
   用中性的琥珀色，与红（失败）/灰（取消）三方区分 */
.agent-ws-run-failed.run-unknown {
  border-color: rgba(214, 158, 46, 0.45);
  background: rgba(214, 158, 46, 0.09);
}
.agent-ws-run-failed.run-unknown .agent-ws-run-failed-status {
  color: #b0882e;
}
.agent-ws-run-failed-status {
  font-weight: 600;
  color: #d64545;
  flex-shrink: 0;
}
.agent-ws-run-failed.run-cancelled .agent-ws-run-failed-status {
  color: var(--text-muted);
}
.agent-ws-run-failed-text {
  color: var(--text-muted);
  word-break: break-word;
  flex: 1;
  min-width: 60px;
}
/* 重试操作：与提示同行，空间不足时换行 */
.agent-ws-run-failed-actions {
  display: flex;
  gap: 4px;
  flex-shrink: 0;
}
.agent-ws-run-failed-actions .btn-sm {
  padding: 1px 7px;
  font-size: 11px;
}
/* 超时引导（评审 3.6）：行动建议独占一行 + 就地调时长 */
.agent-ws-run-advice {
  flex-basis: 100%;
  color: var(--text-muted);
  word-break: break-word;
}
.agent-ws-run-advice-actions {
  flex-shrink: 0;
}
.agent-ws-run-advice-actions .btn-sm {
  padding: 1px 7px;
  font-size: 11px;
}
.agent-ws-run-advice-adjust {
  display: flex;
  gap: 4px;
  flex-shrink: 0;
  align-items: center;
}
.agent-ws-run-advice-adjust .btn-sm {
  padding: 1px 7px;
  font-size: 11px;
}
.agent-ws-timeout-input {
  width: 72px;
  padding: 2px 6px;
  font-size: 11px;
  border: 1px solid var(--border);
  border-radius: 6px;
  background: var(--bg);
  color: var(--text);
}
.agent-ws-timeout-input:focus {
  outline: none;
  border-color: var(--accent, #2e6fd0);
}

/* 折叠块（思考 / 工具详情） */
.agent-ws-fold {
  border: 1px solid var(--border);
  border-radius: 7px;
  background: var(--bg-subtle);
  margin: 6px 0;
  font-size: 12px;
}
.agent-ws-fold summary {
  padding: 5px 9px;
  cursor: pointer;
  opacity: 0.75;
  font-size: 11px;
  user-select: none;
}
.agent-ws-fold-thinking summary {
  color: #8a6d3b;
}
.agent-ws-fold-prompt summary {
  color: var(--text-muted);
}
.agent-ws-fold-body {
  margin: 0;
  padding: 8px 10px;
  border-top: 1px solid var(--border);
  font-family: var(--mono-font, monospace);
  font-size: 11px;
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 260px;
  overflow-y: auto;
  background: var(--bg);
  border-radius: 0 0 7px 7px;
}

/* 工具卡片（toolCall / toolResult / bash） */
.agent-ws-tool-card {
  /* 右侧留白与消息区左侧 padding(14px) 一致，与 assistant 气泡同步 */
  margin-right: 14px;
  align-self: flex-start;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--bg-subtle);
  font-size: 12px;
  overflow: hidden;
}
.agent-ws-tool-card.tool-error {
  border-color: rgba(214, 69, 69, 0.5);
}
.agent-ws-tool-head {
  display: flex;
  align-items: center;
  gap: 7px;
  padding: 6px 10px;
}
.agent-ws-tool-icon {
  width: 13px;
  height: 13px;
  color: var(--accent, #2e6fd0);
  flex-shrink: 0;
}
.agent-ws-tool-name {
  font-family: var(--mono-font, monospace);
  font-weight: 600;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.agent-ws-tool-tag {
  margin-left: auto;
  font-size: 10px;
  opacity: 0.55;
  flex-shrink: 0;
}
.agent-ws-tool-card details summary {
  padding: 5px 10px;
  border-top: 1px solid var(--border);
  cursor: pointer;
  font-size: 11px;
  opacity: 0.7;
  user-select: none;
}
</style>
