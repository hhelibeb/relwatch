<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { storeToRefs } from 'pinia'
import type { UnlistenFn } from '@tauri-apps/api/event'
import { events } from '../bindings'
import { isEventBridgeMounted } from '../api/event-bridge'
import type { AgentWorkspaceSeed } from '../types/agent'
import { t } from '../i18n'
import { useUiStore } from '../stores/ui'
import { useCatalogStore } from '../stores/catalog'
import { useAgentStore } from '../stores/agent'
import { useAgentFind } from './agent/useAgentFind'
import { useAgentDrop } from './agent/useAgentDrop'
import AppIcon from './ui/AppIcon.vue'
import AgentRpcIndicator from './agent/AgentRpcIndicator.vue'
import AgentSessionSidebar from './agent/AgentSessionSidebar.vue'
import AgentRunBanner from './agent/AgentRunBanner.vue'
import AgentMessageList from './agent/AgentMessageList.vue'
import AgentComposer from './agent/AgentComposer.vue'
import FindBar from './common/FindBar.vue'

const props = defineProps<{ seed?: AgentWorkspaceSeed | null; width?: number }>()
const emit = defineEmits<{ close: [] }>()

const ui = useUiStore()
const catalog = useCatalogStore()
const agent = useAgentStore()

const {
  activeKey,
  sidebarOpen,
  scrollRef,
  displayedMessages,
  messageDecorations,
  sources,
  releases,
  liveMessages,
  submitting,
} = storeToRefs(agent)

const panelWidth = computed(() => props.width ?? 440)
const rootEl = ref<HTMLElement | null>(null)

const {
  findScopeInside,
  findOpen,
  findQuery,
  findMatchCount,
  findCurrentIndex,
  setFindInputEl,
  toggleFind,
  closeFind,
  nextFindMatch,
  prevFindMatch,
  handleFindInputKeydown,
} = useAgentFind({
  rootEl,
  scrollRef,
  activeKey,
  displayedMessages,
  messageDecorations,
  sources,
  releases,
  isStreaming: () => liveMessages.value.length > 0 || submitting.value,
})

const {
  dragOver,
  headerDropOver,
  onDragOver,
  onDragLeave,
  handleDrop,
  onHeaderDragOver,
  onHeaderDragLeave,
  handleDropNewSession,
} = useAgentDrop({
  findScopeInside,
  onAttachEntity: (entity) => {
    agent.ensureEntityKnown(entity)
    agent.afterAttach(entity, agent.addEntity(entity))
  },
  onStartNewSession: agent.startNewSession,
})

const fallbackUnlisteners: UnlistenFn[] = []
let wsDisposed = false

onMounted(async () => {
  agent.mountWorkspaceRuntime()
  agent.applySeed(props.seed)
  await Promise.all([agent.loadCatalog(), agent.loadChat(), agent.loadRpcStatus()])
  if (wsDisposed) return
  const recovered = await agent.discoverSessions()
  if (wsDisposed) return
  if (recovered > 0) ui.showToast(t('agent.sessions_recovered', String(recovered)))

  if (!isEventBridgeMounted()) {
    const offs = await Promise.all([
      events.agentRunFinished.listen((e) => {
        void agent.onRunFinished(e.payload.session_key, e.payload.run_id)
      }),
      events.agentRpcStream.listen((e) => {
        agent.handleRpcStream(e.payload)
      }),
      events.releaseStateChanged.listen(() => {
        catalog.scheduleRefreshCatalog()
      }),
    ])
    if (wsDisposed) {
      for (const off of offs) off()
      return
    }
    fallbackUnlisteners.push(...offs)
  }

  await nextTick()
  if (wsDisposed) return
  agent.focusComposer()
  document.addEventListener('pointerdown', agent.handleDocumentPointerDown, true)
})

watch(
  () => props.seed,
  (seed) => agent.applySeed(seed),
)

onUnmounted(() => {
  wsDisposed = true
  for (const off of fallbackUnlisteners) off()
  fallbackUnlisteners.length = 0
  agent.cleanupWorkspaceRuntime()
  document.removeEventListener('pointerdown', agent.handleDocumentPointerDown, true)
})
</script>

<template>
  <div ref="rootEl" class="agent-ws" :style="{ flexBasis: panelWidth + 'px' }">
    <header
      class="agent-ws-header"
      :class="{ 'drop-over': headerDropOver }"
      @dragenter="onHeaderDragOver"
      @dragover="onHeaderDragOver"
      @dragleave="onHeaderDragLeave"
      @drop="handleDropNewSession"
    >
      <div class="agent-ws-title">
        <svg class="agent-ws-title-icon"><use href="/icons.svg#agent-icon" /></svg>
        <span>{{ t('agent.workspace_title') }}</span>
      </div>
      <div class="agent-ws-header-actions">
        <AgentRpcIndicator />
        <button class="btn-sm" :title="t('agent.session_new')" @click="agent.startNewSession()">
          <svg class="agent-ws-btn-icon"><use href="/icons.svg#plus-icon" /></svg>
          <span class="agent-ws-btn-label">{{ t('agent.session_new') }}</span>
        </button>
        <button
          class="btn-sm agent-ws-find-toggle"
          :class="{ active: findOpen }"
          :title="`${t('agent.find_in_chat')} (Ctrl+F)`"
          :aria-label="t('agent.find_in_chat')"
          @click="toggleFind"
        >
          <svg class="agent-ws-btn-icon"><use href="/icons.svg#search-icon" /></svg>
        </button>
        <button class="btn-sm agent-ws-sessions-btn" :class="{ active: sidebarOpen }" :title="t('agent.session_list')" @click="agent.toggleSidebar()">
          <svg class="agent-ws-btn-icon"><use href="/icons.svg#list-icon" /></svg>
          <span class="agent-ws-btn-label">{{ t('agent.session_list') }}</span>
        </button>
        <button class="agent-ws-close" :title="t('release.detail_close')" @click="emit('close')">
          <AppIcon name="close-icon" :size="13" />
        </button>
      </div>
      <div v-if="headerDropOver" class="agent-ws-drop-hint agent-ws-drop-hint-header">{{ t('agent.drop_new_session') }}</div>
    </header>

    <div
      class="agent-ws-main"
      :class="{ 'drag-over': dragOver }"
      @dragover="onDragOver"
      @dragleave="onDragLeave"
      @drop="handleDrop"
    >
      <div v-if="dragOver" class="agent-ws-drop-hint agent-ws-drop-hint-main">{{ t('agent.drop_current_session') }}</div>
      <section class="agent-ws-chat">
        <FindBar
          v-if="findOpen"
          v-model="findQuery"
          class-prefix="agent-ws-find"
          compact
          :match-count="findMatchCount"
          :current-index="findCurrentIndex"
          :placeholder="t('agent.find_in_chat')"
          :prev-label="t('agent.find_prev')"
          :next-label="t('agent.find_next')"
          :close-label="t('agent.find_close')"
          :set-input-el="setFindInputEl"
          @prev="prevFindMatch"
          @next="nextFindMatch"
          @close="closeFind"
          @input-keydown="handleFindInputKeydown"
        />
        <AgentRunBanner />
        <AgentMessageList />
        <AgentComposer />
      </section>

      <AgentSessionSidebar />
    </div>
  </div>
</template>

<style src="./agent/agent-shared.css"></style>
<style scoped>
.agent-ws {
  flex: 1 1 280px;
  min-width: 280px;
  height: 100%;
  display: flex;
  flex-direction: column;
  background: var(--bg);
  color: var(--text);
  overflow: hidden;
}
.agent-ws-main.drag-over {
  outline: 2px dashed var(--primary);
  outline-offset: -4px;
}

.agent-ws-drop-hint {
  position: absolute;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: var(--fs-meta);
  font-weight: 600;
  letter-spacing: 0.5px;
  color: var(--primary);
  background: color-mix(in srgb, var(--primary) 10%, var(--bg));
  pointer-events: none;
  z-index: var(--z-sticky);
}
.agent-ws-drop-hint-header {
  inset: 0;
}
.agent-ws-drop-hint-main {
  top: 12px;
  left: 50%;
  transform: translateX(-50%);
  padding: 6px 16px;
  border-radius: var(--radius-pill);
  white-space: nowrap;
  box-shadow: 0 2px 10px rgba(0, 0, 0, 0.18);
}

.agent-ws-header {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 14px;
  border-bottom: 1px solid var(--border);
  background: var(--bg);
}
.agent-ws-header.drop-over {
  outline: 2px dashed var(--primary);
  outline-offset: -4px;
}
.agent-ws-title {
  display: flex;
  align-items: center;
  gap: 8px;
  font-size: var(--fs-title);
  font-weight: 600;
}
.agent-ws-title-icon {
  width: 16px;
  height: 16px;
  color: var(--primary);
}
.agent-ws-header-actions {
  display: flex;
  align-items: center;
  gap: 6px;
}
.agent-ws-header-actions .btn-sm {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}
.agent-ws-btn-icon {
  width: 12px;
  height: 12px;
  flex-shrink: 0;
}
.agent-ws-btn-label {
  font-size: var(--fs-meta);
}
.agent-ws-sessions-btn.active {
  background: var(--primary-soft-bg);
  border-color: var(--primary-soft-border);
  color: var(--primary-soft-text);
}
.agent-ws-close {
  border: none;
  background: none;
  cursor: pointer;
  color: var(--text);
  padding: 4px;
  display: flex;
  align-items: center;
  border-radius: var(--radius);
}
.agent-ws-close:hover {
  background: var(--bg-hover);
}
.agent-ws-close svg {
  width: 13px;
  height: 13px;
}

.agent-ws-main {
  position: relative;
  flex: 1;
  min-height: 0;
  display: flex;
}

.agent-ws-find-toggle.active {
  color: var(--primary);
  background: var(--primary-soft-bg);
  border-color: var(--primary-soft-border);
}

.agent-ws-chat {
  position: relative;
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  background: var(--bg);
}
</style>
