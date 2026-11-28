<script setup lang="ts">
import { storeToRefs } from 'pinia'
import { t } from '../../i18n'
import MarkdownContent from '../common/MarkdownContent.vue'
import type { AgentRunSummary } from '../../api/agent'
import { useAgentStore } from '../../stores/agent'
import {
  canRetry,
  isToolError,
  toolCardBody,
  toolCardName,
  bashExitLabel,
  runStatusLabel,
} from './agentChatUtils'
import { skillShortName } from '../../utils'

const agent = useAgentStore()
const {
  messagesLoading,
  liveMessages,
  displayedMessages,
  messageDecorations,
  adjustingTimeout,
  timeoutInput,
} = storeToRefs(agent)

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

/** 流式 bash 块的正文（命令可能比输出晚到，缺谁省谁）。 */
function bashLiveText(block: { command: string; output: string }): string {
  return [block.command ? `$ ${block.command}` : '', block.output]
    .filter((s) => s.trim())
    .join('\n')
}
</script>

<template>
  <div :ref="agent.bindChatScrollEl" class="agent-ws-messages">
    <div v-if="messagesLoading && displayedMessages.length === 0 && liveMessages.length === 0" class="agent-ws-hint">{{ t('agent.loading') }}</div>
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
        <div v-if="msg.role === 'user'" class="agent-ws-bubble agent-ws-bubble-user">
          <div v-if="messageDecorations[idx]?.run" class="agent-ws-bubble-meta">
            <span v-for="e in messageDecorations[idx]?.entities ?? []" :key="`${e.kind}:${e.id}`" class="agent-ws-chip">
              <span
                class="agent-ws-chip-text"
                @mouseenter="agent.handleChipEnter($event, `${agent.entityKindLabel(e.kind)} · ${agent.entityLabel(e)}`)"
                @mousemove="agent.handleChipMove"
                @mouseleave="agent.hideChipTooltip"
              >{{ agent.entityKindLabel(e.kind) }} · {{ agent.entityLabel(e) }}</span>
            </span>
            <span v-if="messageDecorations[idx]?.run?.skill_path" class="agent-ws-skill-badge">@{{ skillShortName(messageDecorations[idx]?.run?.skill_path ?? '') }}</span>
          </div>
          <p class="agent-ws-msg-text">{{ messageDecorations[idx]?.main || '…' }}</p>
          <details v-if="messageDecorations[idx]?.folded" class="agent-ws-fold agent-ws-fold-prompt">
            <summary>{{ t('agent.prompt_full') }}</summary>
            <pre class="agent-ws-fold-body">{{ messageDecorations[idx]?.folded }}</pre>
          </details>
          <div
            v-if="canRetry(messageDecorations[idx]?.run)"
            class="agent-ws-run-failed"
            :class="{
              'run-cancelled': messageDecorations[idx]?.run?.status === 'cancelled',
              'run-unknown': messageDecorations[idx]?.run?.status === 'unknown',
            }"
          >
            <span class="agent-ws-run-failed-status">{{ runStatusLabel(messageDecorations[idx]!.run!.status, t) }}</span>
            <span class="agent-ws-run-failed-text" :title="agent.runFailedNote(messageDecorations[idx]?.run) ?? ''">
              {{ agent.runFailedNote(messageDecorations[idx]?.run) || runStatusLabel(messageDecorations[idx]!.run!.status, t) }}
            </span>
            <span v-if="messageDecorations[idx]!.run!.status === 'unknown'" class="agent-ws-run-advice">{{ t('agent.unknown_advice') }}</span>
            <span class="agent-ws-run-failed-actions">
              <button class="btn-sm" :title="t('agent.retry')" @click="agent.handleRetry(messageDecorations[idx]!.run!)">
                {{ t('agent.retry') }}
              </button>
              <button class="btn-sm" :title="t('agent.retry_edit')" @click="agent.handleRetryEdit(messageDecorations[idx]!.run!)">
                {{ t('agent.retry_edit') }}
              </button>
            </span>
            <template v-if="isTimeoutRun(messageDecorations[idx]?.run)">
              <span class="agent-ws-run-advice">{{ t('agent.timeout_advice') }}</span>
              <span v-if="!adjustingTimeout" class="agent-ws-run-advice-actions">
                <button class="btn-sm" :title="t('agent.timeout_adjust')" @click="agent.startAdjustTimeout()">
                  {{ t('agent.timeout_adjust') }}
                </button>
              </span>
              <span v-else class="agent-ws-run-advice-adjust">
                <input
                  v-model="timeoutInput"
                  type="number"
                  min="10"
                  max="3600"
                  class="agent-ws-timeout-input"
                  :placeholder="t('agent.timeout_placeholder')"
                  @keydown.enter.prevent="agent.saveTimeout()"
                />
                <button class="btn-sm" @click="agent.saveTimeout()">{{ t('agent.timeout_save') }}</button>
                <button class="btn-sm" @click="agent.cancelAdjustTimeout()">{{ t('agent.timeout_cancel') }}</button>
              </span>
            </template>
          </div>
        </div>

        <div v-else-if="msg.role === 'assistant'" class="agent-ws-bubble agent-ws-bubble-assistant">
          <div v-if="msg.model" class="agent-ws-bubble-model">{{ msg.model }}</div>
          <template v-for="(block, bi) in msg.blocks" :key="bi">
            <MarkdownContent v-if="block.kind === 'text'" :content="block.kind === 'text' ? block.text : ''" :no-cache="agent.isLiveMessage(msg)" />
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
            <!-- 流式 bash：pi 把命令输入推成 assistant 消息里的 bash 块，这里必须直接渲染。
                 漏掉这一支时命令输出只在整轮结束后重读 JSONL（role='bash'）才出现。 -->
            <div v-else-if="block.kind === 'bash'" class="agent-ws-tool-card">
              <div class="agent-ws-tool-head">
                <svg class="agent-ws-tool-icon"><use href="/icons.svg#terminal-icon" /></svg>
                <span class="agent-ws-tool-name">bash</span>
                <span class="agent-ws-tool-tag">exit {{ block.kind === 'bash' ? (block.exit_code ?? '…') : '' }}</span>
              </div>
              <pre class="agent-ws-fold-body agent-ws-bash-live">{{ bashLiveText(block) }}</pre>
            </div>
          </template>
        </div>

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
  color: var(--text-muted);
  font-size: var(--fs-body);
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
  border-radius: var(--radius-lg);
  padding: 9px 12px;
  font-size: var(--fs-body);
  line-height: 1.55;
}
.agent-ws-bubble-user {
  background: var(--primary-soft-bg);
  border: 1px solid var(--primary-soft-border);
  border-top-right-radius: var(--radius-xs);
  color: var(--text);
}
.agent-ws-bubble-assistant {
  /* 覆盖 .agent-ws-bubble 的 max-width:86%——右侧留白由 margin-right 保证与左侧 padding(14px) 一致 */
  max-width: none;
  margin-right: 14px;
  background: var(--surface);
  border: 1px solid var(--border);
  border-top-left-radius: var(--radius-xs);
}
.agent-ws-bubble-model {
  font-size: var(--fs-caption);
  color: var(--text-muted);
  margin-bottom: 4px;
  font-family: var(--mono-font);
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
/* 非成功终态内联备注 + 重试入口（挂在对应 user 气泡下）。
   这一块是任务相关内容（为什么挂了 / 怎么重试），至少取 --fs-meta */
.agent-ws-run-failed {
  display: flex;
  align-items: baseline;
  flex-wrap: wrap;
  gap: 6px;
  margin-top: 7px;
  padding: 5px 8px;
  border-radius: var(--radius);
  border: 1px solid color-mix(in srgb, var(--danger) 35%, transparent);
  background: var(--danger-soft-bg);
  font-size: var(--fs-meta);
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
  border-color: color-mix(in srgb, var(--warning) 45%, transparent);
  background: var(--warning-soft-bg);
}
.agent-ws-run-failed.run-unknown .agent-ws-run-failed-status {
  color: var(--warning);
}
.agent-ws-run-failed-status {
  font-weight: 600;
  color: var(--danger);
  flex-shrink: 0;
}
.agent-ws-run-failed.run-cancelled .agent-ws-run-failed-status {
  color: var(--text-muted);
}
/* 失败原因不是次要注释：压在三种底色（红/灰/琥珀）上都用正文色 */
.agent-ws-run-failed-text {
  color: var(--text);
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
}
/* 超时引导：行动建议独占一行 + 就地调时长 */
.agent-ws-run-advice {
  flex-basis: 100%;
  color: var(--text);
  word-break: break-word;
}
.agent-ws-run-advice-actions {
  flex-shrink: 0;
}
.agent-ws-run-advice-actions .btn-sm {
  padding: 1px 7px;
}
.agent-ws-run-advice-adjust {
  display: flex;
  gap: 4px;
  flex-shrink: 0;
  align-items: center;
}
.agent-ws-run-advice-adjust .btn-sm {
  padding: 1px 7px;
}
.agent-ws-timeout-input {
  width: 72px;
  padding: 2px 6px;
  font-size: var(--fs-meta);
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg);
  color: var(--text);
}
.agent-ws-timeout-input:focus {
  outline: none;
  border-color: var(--primary);
  box-shadow: var(--focus-ring);
}

/* 折叠块（思考 / 工具详情） */
.agent-ws-fold {
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-subtle);
  margin: 6px 0;
  font-size: var(--fs-meta);
}
/* 折叠标题是控件标签而不是说明文字：取 --fs-caption 但用实色，
   不再叠 opacity（叠完只有 3.4:1）*/ 
.agent-ws-fold summary {
  padding: 5px 9px;
  cursor: pointer;
  color: var(--text-muted);
  font-size: var(--fs-caption);
  user-select: none;
}
.agent-ws-fold-thinking summary {
  color: var(--warning);
}
.agent-ws-fold-prompt summary {
  color: var(--text-muted);
}
.agent-ws-fold-body {
  margin: 0;
  padding: 8px 10px;
  border-top: 1px solid var(--border);
  font-family: var(--mono-font);
  font-size: var(--fs-meta);
  line-height: 1.5;
  white-space: pre-wrap;
  word-break: break-word;
  max-height: 260px;
  overflow-y: auto;
  background: var(--bg);
  border-radius: 0 0 var(--radius) var(--radius);
}

/* 流式 bash 输出直接铺开（不折叠）：折叠态下「命令正在跑」时气泡里空无一物，
   用户看不到任何进展。限高滚动，避免长输出把整轮对话顶出视口。 */
.agent-ws-bash-live {
  max-height: 200px;
}

/* 工具卡片（toolCall / toolResult / bash） */
.agent-ws-tool-card {
  /* 右侧留白与消息区左侧 padding(14px) 一致，与 assistant 气泡同步 */
  margin-right: 14px;
  align-self: flex-start;
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg-subtle);
  font-size: var(--fs-meta);
  overflow: hidden;
}
.agent-ws-tool-card.tool-error {
  border-color: color-mix(in srgb, var(--danger) 50%, transparent);
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
  color: var(--primary);
  flex-shrink: 0;
}
.agent-ws-tool-name {
  font-family: var(--mono-font);
  font-weight: 600;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.agent-ws-tool-tag {
  margin-left: auto;
  font-size: var(--fs-caption);
  color: var(--text-muted);
  flex-shrink: 0;
}
.agent-ws-tool-card details summary {
  padding: 5px 10px;
  border-top: 1px solid var(--border);
  cursor: pointer;
  font-size: var(--fs-caption);
  color: var(--text-muted);
  user-select: none;
}
</style>
