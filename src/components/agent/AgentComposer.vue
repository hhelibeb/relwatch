<script setup lang="ts">
import { storeToRefs } from 'pinia'
import { t } from '../../i18n'
import { formatDate, skillShortName } from '../../utils'
import { useAgentStore } from '../../stores/agent'
import AppIcon from '../ui/AppIcon.vue'

const agent = useAgentStore()
const {
  entities,
  files,
  instruction,
  skillPath,
  flashKey,
  attachAnnouncement,
  skills,
  showSkillMenu,
  showEntityMenu,
  skillMenuIndex,
  entityMenuIndex,
  filteredSkills,
  filteredSources,
  filteredReleases,
  filteredSourcesCount,
  filteredReleasesCount,
  entityMenuHasMatch,
  showModelMenu,
  modelMenuIndex,
  availableModels,
  effectiveModel,
  modelOnce,
  activeModelLabel,
  modelDefaultSub,
  submitting,
  canStop,
  cancelling,
  chipTooltip,
  usageText,
  usageEstimated,
  usageHint,
} = storeToRefs(agent)
</script>

<template>
  <footer class="agent-ws-input">
    <div class="agent-ws-input-meta">
      <span
        v-for="(e, i) in entities"
        :key="`${e.kind}:${e.id}`"
        class="agent-ws-chip agent-ws-chip-attached"
        :class="{ 'is-new': flashKey === `${e.kind}:${e.id}` }"
      >
        <span
          class="agent-ws-chip-text"
          @mouseenter="agent.handleChipEnter($event, `${agent.entityKindLabel(e.kind)} · ${agent.entityLabel(e)}`)"
          @mousemove="agent.handleChipMove"
          @mouseleave="agent.hideChipTooltip"
        >{{ agent.entityKindLabel(e.kind) }} · {{ agent.entityLabel(e) }}</span>
        <button class="agent-ws-chip-remove" :title="t('agent.remove_entity')" @click="agent.removeEntity(i)">
          <AppIcon name="close-icon" :size="10" />
        </button>
      </span>
      <span v-if="skillPath" class="agent-ws-skill-badge" :title="skillPath">
        @{{ skillShortName(skillPath) }}
        <button class="agent-ws-chip-remove" :title="t('agent.clear_skill')" @click="agent.clearSkill()">
          <AppIcon name="close-icon" :size="10" />
        </button>
      </span>
      <span
        v-for="(f, i) in files"
        :key="f"
        class="agent-ws-chip agent-ws-chip-file"
        :title="f"
      >
        <svg class="agent-ws-chip-file-icon" viewBox="0 0 16 16"><path d="M4 1.5h5L12.5 5v9.5H4z" stroke="currentColor" stroke-width="1.2" fill="none" stroke-linejoin="round"/><path d="M9 1.5V5h3.5" stroke="currentColor" stroke-width="1.2" fill="none" stroke-linejoin="round"/></svg>
        <span class="agent-ws-chip-text">{{ agent.fileDisplayName(f) }}</span>
        <button class="agent-ws-chip-remove" :title="t('agent.remove_file')" @click="agent.removeFile(i)">
          <AppIcon name="close-icon" :size="10" />
        </button>
      </span>
      <span class="agent-ws-sr-only" aria-live="polite">{{ attachAnnouncement }}</span>
    </div>
    <div class="agent-ws-input-row">
      <textarea
        :ref="agent.bindTextareaEl"
        v-model="instruction"
        class="agent-ws-textarea"
        :placeholder="t('agent.placeholder')"
        rows="3"
        @input="agent.handleComposerInput()"
        @keydown="agent.handleComposerKeydown($event)"
      ></textarea>
    </div>
    <div class="agent-ws-input-actions">
      <button class="agent-ws-model-btn" :class="{ open: showModelMenu }" :title="t('agent.model_pick')" @click="agent.toggleModelMenu()">
        <svg class="agent-ws-model-icon" viewBox="0 0 16 16"><path d="M2.5 4.5h11M2.5 8h11M2.5 11.5h11" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" fill="none"/></svg>
        <span class="agent-ws-model-label">{{ activeModelLabel }}</span>
        <svg class="agent-ws-model-caret" viewBox="0 0 16 16"><path d="M4 6l4 4 4-4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" fill="none"/></svg>
      </button>
      <div class="agent-ws-input-actions-right">
        <button class="agent-ws-attach-btn" :title="t('agent.attach_file')" @click="agent.handleAttachFiles()">
          <svg viewBox="0 0 16 16"><path d="M13 7.5L8 12.5a3 3 0 01-4.2-4.2l5-5a2 2 0 012.8 2.8l-5 5a1 1 0 01-1.4-1.4l4.3-4.3" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" fill="none"/></svg>
        </button>
        <button
          class="btn-primary agent-ws-submit"
          :class="{ 'agent-ws-stop': canStop }"
          :disabled="submitting && !canStop"
          :title="canStop ? t('agent.stop_hint') : ''"
          @click="canStop ? agent.handleCancel() : agent.handleSubmit()"
        >
          {{ canStop ? (cancelling ? t('agent.stopping') : t('agent.stop')) : submitting ? t('agent.running') : t('agent.submit') }}
        </button>
      </div>

      <div v-if="showModelMenu" class="agent-ws-menu agent-ws-menu-model">
        <div class="agent-ws-menu-title">{{ t('agent.model_pick') }}</div>
        <button
          class="agent-ws-menu-item agent-ws-menu-once"
          :class="{ selected: modelOnce }"
          :title="t('agent.model_once_hint')"
          @click="agent.toggleModelOnce()"
        >
          <span class="agent-ws-menu-check">{{ modelOnce ? '✓' : '' }}</span>
          <span class="agent-ws-menu-main">{{ t('agent.model_once') }}</span>
        </button>
        <button
          class="agent-ws-menu-item"
          :class="{ selected: !effectiveModel }"
          @mouseenter="modelMenuIndex = 0"
          @click="agent.pickModel(null)"
        >
          <span class="agent-ws-menu-main">{{ t('agent.model_default') }}</span>
          <span v-if="modelDefaultSub" class="agent-ws-menu-sub">{{ modelDefaultSub }}</span>
        </button>
        <button
          v-for="(m, i) in availableModels"
          :key="`${m.provider}:${m.id}`"
          class="agent-ws-menu-item"
          :class="{ selected: agent.isModelSelected(m) }"
          @mouseenter="modelMenuIndex = i + 1"
          @click="agent.pickModel(m)"
        >
          <span class="agent-ws-menu-main">{{ agent.modelLabel(m) }}</span>
          <span class="agent-ws-menu-sub">{{ m.provider }} · {{ m.id }}</span>
        </button>
        <div v-if="availableModels.length === 0" class="agent-ws-menu-empty">{{ t('agent.model_none') }}</div>
      </div>
    </div>

    <div v-if="usageText" class="agent-ws-usage">
      <span class="agent-ws-usage-text" :title="usageHint">{{ usageText }}</span>
      <span v-if="usageEstimated" class="agent-ws-usage-est" :title="t('agent.usage_estimate_hint')">≈</span>
    </div>

    <div v-if="showSkillMenu" class="agent-ws-menu">
      <div class="agent-ws-menu-title">{{ t('agent.skill_pick') }}</div>
      <button
        v-for="(s, i) in filteredSkills"
        :key="s"
        class="agent-ws-menu-item"
        :class="{ selected: i === skillMenuIndex }"
        @mouseenter="skillMenuIndex = i"
        @click="agent.pickSkill(s)"
      >
        <span class="agent-ws-menu-main">@{{ skillShortName(s) }}</span>
        <span class="agent-ws-menu-sub">{{ s }}</span>
      </button>
      <div v-if="filteredSkills.length === 0" class="agent-ws-menu-empty">
        {{ skills.length === 0 ? t('agent.no_skills') : t('agent.no_match') }}
      </div>
    </div>

    <div v-if="showEntityMenu" class="agent-ws-menu agent-ws-menu-entities">
      <div class="agent-ws-menu-title">{{ t('agent.entity_pick') }}</div>
      <template v-if="filteredSources.length">
        <div class="agent-ws-menu-group">
          <svg class="agent-ws-menu-group-icon"><use href="/icons.svg#sources-icon" /></svg>
          {{ t('agent.entity_source') }} ({{ filteredSourcesCount }})
        </div>
        <button
          v-for="(s, i) in filteredSources"
          :key="`s${s.id}`"
          class="agent-ws-menu-item"
          :class="{ selected: i === entityMenuIndex }"
          @mouseenter="entityMenuIndex = i"
          @click="agent.pickEntity('source', s.id)"
        >
          <span class="agent-ws-menu-main">{{ agent.sourceDisplayName(s) }}</span>
          <span class="agent-ws-menu-sub">{{ s.source_type }} · 监控源 #{{ s.id }}</span>
        </button>
      </template>
      <template v-if="filteredReleases.length">
        <div class="agent-ws-menu-group">
          <svg class="agent-ws-menu-group-icon"><use href="/icons.svg#release-icon" /></svg>
          {{ t('agent.entity_release') }} ({{ filteredReleasesCount }})
        </div>
        <button
          v-for="(r, i) in filteredReleases"
          :key="`r${r.id}`"
          class="agent-ws-menu-item"
          :class="{ selected: filteredSources.length + i === entityMenuIndex }"
          @mouseenter="entityMenuIndex = filteredSources.length + i"
          @click="agent.pickEntity('release', r.id)"
        >
          <span class="agent-ws-menu-main">{{ agent.releaseDisplayName(r) }}</span>
          <span class="agent-ws-menu-sub">{{ formatDate(r.published_at) }}</span>
        </button>
      </template>
      <div v-if="!entityMenuHasMatch" class="agent-ws-menu-empty">{{ t('agent.no_match') }}</div>
    </div>

    <div v-if="chipTooltip" class="agent-ws-chip-tooltip" :style="{ left: chipTooltip.x + 'px', top: chipTooltip.y + 'px' }">
      {{ chipTooltip.text }}
    </div>
  </footer>
</template>

<style scoped>
/* 输入区 */
.agent-ws-input {
  position: relative;
  border-top: 1px solid var(--border);
  padding: 10px 14px 12px;
  background: var(--bg);
}
.agent-ws-input-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 4px;
  margin-bottom: 6px;
  min-height: 18px;
}
/* 引用加入的就地高亮。只做描边/底色 + 光晕，不改尺寸位移，
   避免 chip 换行引起输入区抖动。动画时长须与脚本里的 FLASH_DURATION（1200ms）保持一致 */
.agent-ws-chip-attached.is-new {
  border-color: var(--primary);
  background: var(--primary-soft-bg);
  animation: agent-ws-chip-flash 1.2s ease-out;
}
@keyframes agent-ws-chip-flash {
  0% {
    background: color-mix(in srgb, var(--primary) 28%, transparent);
    box-shadow: 0 0 0 0 color-mix(in srgb, var(--primary) 45%, transparent);
  }
  35% {
    box-shadow: 0 0 0 3px color-mix(in srgb, var(--primary) 18%, transparent);
  }
  100% {
    background: var(--primary-soft-bg);
    box-shadow: 0 0 0 0 transparent;
  }
}
@media (prefers-reduced-motion: reduce) {
  .agent-ws-chip-attached.is-new {
    animation: none;
  }
}
/* 屏幕阅读器专用（视觉不可见）：absolute 定位使其脱离 flex 流，
   不会给 chip 行挤进额外间距 */
.agent-ws-sr-only {
  position: absolute;
  width: 1px;
  height: 1px;
  margin: -1px;
  padding: 0;
  overflow: hidden;
  clip: rect(0 0 0 0);
  clip-path: inset(50%);
  white-space: nowrap;
  border: 0;
}
.agent-ws-input-row {
  display: flex;
  gap: 8px;
  align-items: flex-end;
}
.agent-ws-textarea {
  flex: 1;
  resize: none;
  padding: 8px 10px;
  font-size: var(--fs-body);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  background: var(--bg);
  color: var(--text);
  line-height: 1.5;
  font-family: inherit;
}
.agent-ws-textarea:focus {
  outline: none;
  border-color: var(--primary);
  box-shadow: var(--focus-ring);
}
.agent-ws-submit {
  flex-shrink: 0;
  /* 对齐 composer 30px/8px 基线，覆盖全局 .btn-primary 的 padding/字号/圆角 */
  height: 30px;
  padding: 0 16px;
  font-size: var(--fs-meta);
  border-radius: var(--radius-lg);
}
.agent-ws-submit.agent-ws-stop {
  background: var(--danger);
  border-color: var(--danger);
}
.agent-ws-submit.agent-ws-stop:hover {
  background: var(--danger-hover);
  border-color: var(--danger-hover);
}

/* 底部操作行：模型选择（最左）+ 附件/发送（右侧成组），共占一行。
 * 三按钮统一几何基线：30px 高 + 8px 圆角（与输入框一致），
 * 次级（模型/附件）同款灰底细边框，主操作（发送）墨色实心——
 * 样式语言只有一种次级 + 一种主级，高度/圆角不再各吹各调 */
.agent-ws-input-actions {
  position: relative;
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 8px;
  margin-top: 6px;
}
/* 右侧操作组：附件 + 发送/停止 */
.agent-ws-input-actions-right {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}
/* 附加本地文件按钮：与模型按钮同高的纯图标次级按钮（30px 基线） */
.agent-ws-attach-btn {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
  width: 30px;
  height: 30px;
  padding: 0;
  color: var(--text-muted);
  background: var(--bg-subtle);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  cursor: pointer;
  transition: border-color var(--dur-hover) var(--ease-out), background var(--dur-hover) var(--ease-out), color var(--dur-hover) var(--ease-out);
}
.agent-ws-attach-btn:hover {
  color: var(--text);
  border-color: var(--primary);
  background: var(--primary-soft-bg);
}
.agent-ws-attach-btn svg {
  width: 14px;
  height: 14px;
}

/* 文件附件 chip（与实体 chip 同形，加一个文件图标以区分来源） */
.agent-ws-chip-file {
  display: inline-flex;
  align-items: center;
  gap: 4px;
}
.agent-ws-chip-file-icon {
  flex-shrink: 0;
  width: 11px;
  height: 11px;
  opacity: 0.7;
}

.agent-ws-model-btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  max-width: 100%;
  height: 30px;
  padding: 0 9px;
  font-size: var(--fs-meta);
  color: var(--text);
  background: var(--bg-subtle);
  border: 1px solid var(--border);
  border-radius: var(--radius-lg);
  cursor: pointer;
  transition: border-color var(--dur-hover) var(--ease-out), background var(--dur-hover) var(--ease-out);
}
.agent-ws-model-btn:hover,
.agent-ws-model-btn.open {
  border-color: var(--primary);
  background: var(--primary-soft-bg);
}
.agent-ws-model-icon {
  width: 12px;
  height: 12px;
  color: var(--primary);
  flex-shrink: 0;
}
.agent-ws-model-label {
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.agent-ws-model-caret {
  width: 11px;
  height: 11px;
  color: var(--text-muted);
  flex-shrink: 0;
  transition: transform var(--dur-hover) var(--ease-out);
}
.agent-ws-model-btn.open .agent-ws-model-caret {
  transform: rotate(180deg);
}
/* 模型选择菜单：作为 .agent-ws-input-actions 的子元素定位（position:relative 的 parent），
   bottom:100% 使面板紧贴在操作行（左侧模型按钮）上方弹出，而非输入区顶部；
   高优先级选择器覆盖 .agent-ws-menu 基类的 footer 定位，不依赖顺序 */
.agent-ws-menu.agent-ws-menu-model {
  position: absolute;
  bottom: calc(100% + 6px);
  left: 0;
  right: auto;
  top: auto;
  width: min(340px, 100%);
}
/* 「仅本次」开关行：勾选态标记左置，与普通菜单项区分 */
.agent-ws-menu-once {
  border-bottom: 1px solid var(--border);
}
.agent-ws-menu-check {
  flex-shrink: 0;
  width: 12px;
  font-size: var(--fs-caption);
  color: var(--primary);
}

/* 会话上下文水位（数字由后端按 pi footer 同口径算好） */
.agent-ws-usage {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-top: 6px;
  font-size: var(--fs-caption);
  color: var(--text-muted);
}
.agent-ws-usage-text {
  flex: 1;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
/* 引用 chip 全文悬浮提示（fixed 避免被消息区滚动容器裁剪） */
.agent-ws-chip-tooltip {
  position: fixed;
  z-index: var(--z-popover);
  max-width: min(480px, calc(100vw - 32px));
  padding: 9px 12px;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm);
  box-shadow: var(--shadow-lg, 0 4px 16px rgba(0, 0, 0, 0.25));
  color: var(--text);
  font-size: var(--fs-meta);
  line-height: 1.55;
  overflow-wrap: anywhere;
  pointer-events: none;
}

/* 估算标记：pi 未上报用量时，词元数是按字符数估的，标一个 ≈ 免得被当成精确计费值。
   不再单给一档透明度：叠完只有 2.5:1，与左侧数字同色同重即可，语义由 ≈ 本身承担 */
.agent-ws-usage-est {
  flex-shrink: 0;
  font-style: normal;
  cursor: help;
}
</style>
