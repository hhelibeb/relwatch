<!-- pi 常驻进程健康指示（E 域展示层）：点灯弹状态菜单（状态详情 + 重启入口）。
     重启是低频排障操作，收进菜单而非一级按钮；未运行时菜单不提供
     重启项（无物可重启，首次提交时进程会自动拉起）。
     状态与菜单逻辑由编排层的 useAgentRpc 持有，本组件纯 props/emit 展示。 -->
<script setup lang="ts">
import type { ComponentPublicInstance } from 'vue'
import type { AgentRpcStatus } from '../../api/agent'
import { t } from '../../i18n'

const props = defineProps<{
  rpcStatus: AgentRpcStatus | null
  rpcRestarting: boolean
  rpcMenuOpen: boolean
  rpcMenuStyle: { left: string; top: string }
  /** 灯按钮元素回填：编排层持有 useAgentRpc 的 rpcDotEl（菜单以灯为锚定位），
   *  经此函数 ref 从子组件模板回填（VNodeRef 签名的 el 可能是 SVGSVGElement 等）。 */
  setDotEl: (el: Element | ComponentPublicInstance | null) => void
}>()

const emit = defineEmits<{ toggleMenu: []; restart: [] }>()
</script>

<template>
  <div class="agent-ws-rpc-wrap">
    <button
      :ref="props.setDotEl"
      class="agent-ws-rpc-dot"
      :class="{ running: rpcStatus?.running, restarting: rpcRestarting }"
      :title="rpcStatus?.running ? t('agent.rpc_running') : t('agent.rpc_stopped')"
      :aria-label="t('agent.rpc_status')"
      @click="emit('toggleMenu')"
    ></button>
    <!-- Teleport 到 body：header 贴窗口顶缘，absolute 定位无论上弹/下弹
         都会被窗口边缘或后续层叠内容吃掉；fixed + 高 z-index 才能盖在最上层 -->
    <Teleport to="body">
      <div v-if="rpcMenuOpen" class="agent-ws-menu agent-ws-menu-rpc" :style="rpcMenuStyle">
        <!-- 状态详情（非交互）：只陈述事实，操作在下方独立分区 -->
        <div class="agent-ws-rpc-status">
          <span class="agent-ws-rpc-status-dot" :class="{ on: rpcStatus?.running }" aria-hidden="true"></span>
          <div class="agent-ws-rpc-status-text">
            <span class="agent-ws-rpc-status-main">{{ rpcStatus?.running ? t('agent.rpc_running') : t('agent.rpc_stopped') }}</span>
            <span v-if="rpcStatus?.running && rpcStatus.pid" class="agent-ws-rpc-status-sub">pid {{ rpcStatus.pid }}</span>
            <span v-else class="agent-ws-rpc-status-sub">{{ t('agent.rpc_not_started_hint') }}</span>
          </div>
        </div>
        <!-- 重启入口：仅运行中提供（未运行时点击无物可重启，且首次提交会自动拉起） -->
        <button
          v-if="rpcStatus?.running"
          class="agent-ws-menu-item"
          :disabled="rpcRestarting"
          @click="emit('restart')"
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
  transition: opacity 0.15s ease;
}
.agent-ws-rpc-dot:hover::before {
  opacity: 1;
}
.agent-ws-rpc-dot.running::before {
  /* 运行中：绿色实心（与「未运行」的灰点一眼可辨） */
  background: #35a06b;
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
 * 故脱离文档流盖在最上层（与 chip-tooltip 同一策略，z-index 对齐 10002）。
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
  z-index: 10002;
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
  background: #35a06b;
  opacity: 1;
}
.agent-ws-rpc-status-text {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
}
.agent-ws-rpc-status-main {
  font-size: 12px;
  font-weight: 600;
}
.agent-ws-rpc-status-sub {
  font-size: 11px;
  color: var(--text-muted);
}
.agent-ws-menu-rpc .agent-ws-menu-item {
  border-top: 1px solid var(--border);
  border-radius: 0 0 6px 6px;
  margin-top: 2px;
}
</style>
