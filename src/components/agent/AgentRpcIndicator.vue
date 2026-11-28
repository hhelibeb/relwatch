<script setup lang="ts">
import { storeToRefs } from 'pinia'
import { t } from '../../i18n'
import { useAgentStore } from '../../stores/agent'

const agent = useAgentStore()
const { rpcStatus, rpcRestarting, rpcMenuOpen, rpcMenuStyle } = storeToRefs(agent)
</script>

<template>
  <div class="agent-ws-rpc-wrap">
    <button
      :ref="agent.bindRpcDotEl"
      class="agent-ws-rpc-dot"
      :class="{ running: rpcStatus?.running, restarting: rpcRestarting }"
      :title="rpcStatus?.running ? t('agent.rpc_running') : t('agent.rpc_stopped')"
      :aria-label="t('agent.rpc_status')"
      @click="agent.toggleRpcMenu()"
    ></button>
    <Teleport to="body">
      <div v-if="rpcMenuOpen" class="agent-ws-menu agent-ws-menu-rpc" :style="rpcMenuStyle">
        <div class="agent-ws-rpc-status">
          <span class="agent-ws-rpc-status-dot" :class="{ on: rpcStatus?.running }" aria-hidden="true"></span>
          <div class="agent-ws-rpc-status-text">
            <span class="agent-ws-rpc-status-main">{{ rpcStatus?.running ? t('agent.rpc_running') : t('agent.rpc_stopped') }}</span>
            <span v-if="!rpcStatus?.running" class="agent-ws-rpc-status-sub">{{ t('agent.rpc_not_started_hint') }}</span>
            <span v-else-if="rpcStatus.process_count > 1" class="agent-ws-rpc-status-sub">{{ t('agent.rpc_processes', String(rpcStatus.process_count)) }}</span>
            <span v-else-if="rpcStatus.pid" class="agent-ws-rpc-status-sub">pid {{ rpcStatus.pid }}</span>
          </div>
        </div>
        <button
          v-if="rpcStatus?.running"
          class="agent-ws-menu-item"
          :disabled="rpcRestarting"
          @click="agent.handleRestartRpc()"
        >
          <span class="agent-ws-menu-main">{{ t('agent.rpc_restart') }}</span>
        </button>
      </div>
    </Teleport>
  </div>
</template>

<style scoped>
/* pi 进程健康指示：点击热区大于视觉圆点（8px 点太小，Fitts），圆点用 ::before 画 */
.agent-ws-rpc-wrap {
  position: relative;
  flex-shrink: 0;
}
.agent-ws-rpc-dot {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  padding: 0;
  border: none;
  background: transparent;
  cursor: pointer;
}
.agent-ws-rpc-dot::before {
  content: '';
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--text-muted);
  opacity: 0.45;
  transition: opacity var(--dur-state) var(--ease-out);
}
.agent-ws-rpc-dot:hover::before {
  opacity: 1;
}
.agent-ws-rpc-dot.running::before {
  /* 运行中：绿色实心（与「未运行」的灰点一眼可辨） */
  background: var(--success);
  opacity: 1;
}
.agent-ws-rpc-dot.restarting::before {
  animation: agent-ws-pulse 0.9s ease-in-out infinite;
}
@keyframes agent-ws-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.3; }
}

/* 状态菜单：Teleport 到 body 后 fixed 定位在灯正下方（坐标由 toggleRpcMenu 计算）。
 * header 贴窗口顶缘：absolute 上弹会被 Windows 窗口裁剪，下弹会被聊天区后续内容遮挡，
 * 故脱离文档流盖在最上层，与其它下拉取同一档（--z-menu）——它仍是「菜单」而非
 * tooltip，不该跟着 chip-tooltip 抬到 --z-popover。
 * 双类选择器抬高特异性覆盖基类（.agent-ws-menu 基类在 agent-shared.css，非 scoped；
 * 单类时其 bottom: calc(100% - 8px)/max-height/overflow 反杀：inline top 与基类
 * bottom 双锚把高度拉伸成 0，菜单被压成窄条滚动区）——与 agent-ws-menu-model 同一策略。 */
.agent-ws-menu.agent-ws-menu-rpc {
  position: fixed;
  bottom: auto;
  right: auto;
  min-width: 208px;
  max-height: none;
  overflow-y: visible;
  z-index: var(--z-menu);
}
/* 状态详情行（非交互，与操作项分区） */
.agent-ws-rpc-status {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 6px 8px 4px;
}
.agent-ws-rpc-status-dot {
  flex-shrink: 0;
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--text-muted);
  opacity: 0.45;
}
.agent-ws-rpc-status-dot.on {
  background: var(--success);
  opacity: 1;
}
.agent-ws-rpc-status-text {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
}
.agent-ws-rpc-status-main {
  font-size: var(--fs-meta);
  font-weight: 600;
}
.agent-ws-rpc-status-sub {
  font-size: var(--fs-caption);
  color: var(--text-muted);
}
.agent-ws-menu-rpc .agent-ws-menu-item {
  border-top: 1px solid var(--border);
  border-radius: 0 0 var(--radius) var(--radius);
  margin-top: 2px;
}
</style>
