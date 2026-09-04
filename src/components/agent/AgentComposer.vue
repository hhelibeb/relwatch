<!-- 输入区（F/D/H 域展示层）：chips 行 + textarea + 模型/附件/发送 + 三个菜单
     （模型 / @ Skill / [[ 实体）+ 会话上下文水位。
     状态由编排层的 useAgentComposer / useAgentModels / useAgentUsage 持有，
     本组件纯 props/emit 展示：名称映射/悬浮提示等函数经 props 注入，
     textarea 元素经 setTextareaEl 回填（composable 的 focus()/replaceTrigger
     需要），菜单索引回写经 emit 上抛（键盘导航 K 在编排层读写同一 ref）。 -->
<script setup lang="ts">
import type { ComponentPublicInstance } from 'vue'
import { t } from '../../i18n'
import { formatDate, skillShortName } from '../../utils'
import type { AgentModelRef, AgentSessionUsage, RpcAvailableModel } from '../../api/agent'
import type { Source } from '../../api/sources'
import type { ReleaseInfo } from '../../api/releases'
import type { AgentEntityRefSeed } from '../../injection-keys'

defineProps<{
  /** textarea 元素回填（useAgentComposer.textareaRef 在编排层） */
  setTextareaEl: (el: Element | ComponentPublicInstance | null) => void
  // ── F 域状态（引用与输入）──
  entities: AgentEntityRefSeed[]
  files: string[]
  instruction: string
  skillPath: string | null
  flashKey: string | null
  attachAnnouncement: string
  skills: string[]
  showSkillMenu: boolean
  showEntityMenu: boolean
  skillMenuIndex: number
  entityMenuIndex: number
  filteredSkills: string[]
  filteredSources: Source[]
  filteredReleases: ReleaseInfo[]
  filteredSourcesCount: number
  filteredReleasesCount: number
  entityMenuHasMatch: boolean
  // ── D 域状态（模型选择）──
  showModelMenu: boolean
  availableModels: RpcAvailableModel[]
  effectiveModel: AgentModelRef | null
  modelOnce: boolean
  activeModelLabel: string
  modelDefaultSub: string
  // ── C 域状态（提交/停止）──
  submitting: boolean
  canStop: boolean
  cancelling: boolean
  // ── 引用 chip 全文悬浮提示（跟随鼠标，仅文本截断时显示；composer 域状态）──
  chipTooltip: { x: number; y: number; text: string } | null
  // ── H 域状态（会话上下文水位）──
  usageText: string | null
  usageWarn: boolean
  usage: AgentSessionUsage | null
  // ── 函数注入（composable 动作经编排层透传；ref 状态回写经 emit）──
  /** 模型菜单项文案与选中态（models.modelLabel / modelKey 语义） */
  modelLabel: (m: RpcAvailableModel) => string
  isModelSelected: (m: RpcAvailableModel) => boolean
  /** 实体/文件/源/版本的名称映射（composer 域函数） */
  entityLabel: (e: AgentEntityRefSeed) => string
  entityKindLabel: (kind: string) => string
  fileDisplayName: (path: string) => string
  sourceDisplayName: (s: Source) => string
  releaseDisplayName: (r: ReleaseInfo) => string
  /** chip 悬浮提示（composer 域函数） */
  handleChipEnter: (e: MouseEvent, text: string) => void
  handleChipMove: (e: MouseEvent) => void
  hideChipTooltip: () => void
}>()

const emit = defineEmits<{
  'update:instruction': [value: string]
  /** 菜单索引回写（K 键盘导航与 hover 共用编排层的同一 ref） */
  'skill-hover': [index: number]
  'entity-hover': [index: number]
  'model-hover': [index: number]
  /** 动作（原 @click 语义保持，实现在编排层/composable） */
  submit: []
  cancel: []
  input: []
  keydown: [e: KeyboardEvent]
  attachFiles: []
  removeEntity: [index: number]
  removeFile: [index: number]
  clearSkill: []
  pickSkill: [skill: string]
  pickEntity: [kind: 'source' | 'release', id: number]
  toggleModelMenu: []
  pickModel: [model: RpcAvailableModel | null]
  toggleModelOnce: []
  newSession: []
}>()
</script>

<template>
  <footer class="agent-ws-input">
    <div class="agent-ws-input-meta">
      <!-- 引用变更不再走 Toast（会遮挡发送按钮），改由 chip 高亮就地反馈；
           屏幕阅读器由下方 live region 播报，Toast 的告知作用不丢失 -->
      <span
        v-for="(e, i) in entities"
        :key="`${e.kind}:${e.id}`"
        class="agent-ws-chip agent-ws-chip-attached"
        :class="{ 'is-new': flashKey === `${e.kind}:${e.id}` }"
      >
        <span
          class="agent-ws-chip-text"
          @mouseenter="handleChipEnter($event, `${entityKindLabel(e.kind)} · ${entityLabel(e)}`)"
          @mousemove="handleChipMove"
          @mouseleave="hideChipTooltip"
        >{{ entityKindLabel(e.kind) }} · {{ entityLabel(e) }}</span>
        <button class="agent-ws-chip-remove" :title="t('agent.remove_entity')" @click="emit('removeEntity', i)">
          <svg viewBox="0 0 16 16"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" fill="none" /></svg>
        </button>
      </span>
      <span v-if="skillPath" class="agent-ws-skill-badge" :title="skillPath">
        @{{ skillShortName(skillPath) }}
        <button class="agent-ws-chip-remove" :title="t('agent.clear_skill')" @click="emit('clearSkill')">
          <svg viewBox="0 0 16 16"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" fill="none" /></svg>
        </button>
      </span>
      <!-- 本地文件附件：chip 只显示文件名，完整路径放 title -->
      <span
        v-for="(f, i) in files"
        :key="f"
        class="agent-ws-chip agent-ws-chip-file"
        :title="f"
      >
        <svg class="agent-ws-chip-file-icon" viewBox="0 0 16 16"><path d="M4 1.5h5L12.5 5v9.5H4z" stroke="currentColor" stroke-width="1.2" fill="none" stroke-linejoin="round"/><path d="M9 1.5V5h3.5" stroke="currentColor" stroke-width="1.2" fill="none" stroke-linejoin="round"/></svg>
        <span class="agent-ws-chip-text">{{ fileDisplayName(f) }}</span>
        <button class="agent-ws-chip-remove" :title="t('agent.remove_file')" @click="emit('removeFile', i)">
          <svg viewBox="0 0 16 16"><path d="M4 4l8 8M12 4l-8 8" stroke="currentColor" stroke-width="1.6" stroke-linecap="round" fill="none" /></svg>
        </button>
      </span>
      <span class="agent-ws-sr-only" aria-live="polite">{{ attachAnnouncement }}</span>
    </div>
    <div class="agent-ws-input-row">
      <textarea
        :ref="setTextareaEl"
        :value="instruction"
        class="agent-ws-textarea"
        :placeholder="t('agent.placeholder')"
        rows="3"
        @input="emit('update:instruction', ($event.target as HTMLTextAreaElement).value); emit('input')"
        @keydown="emit('keydown', $event)"
      ></textarea>
    </div>
    <!-- 底部操作行：模型选择（最左）+ 附件/发送（右侧成组），共占一行 -->
    <div class="agent-ws-input-actions">
      <button class="agent-ws-model-btn" :class="{ open: showModelMenu }" :title="t('agent.model_pick')" @click="emit('toggleModelMenu')">
        <svg class="agent-ws-model-icon" viewBox="0 0 16 16"><path d="M2.5 4.5h11M2.5 8h11M2.5 11.5h11" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" fill="none"/></svg>
        <span class="agent-ws-model-label">{{ activeModelLabel }}</span>
        <svg class="agent-ws-model-caret" viewBox="0 0 16 16"><path d="M4 6l4 4 4-4" stroke="currentColor" stroke-width="1.5" stroke-linecap="round" stroke-linejoin="round" fill="none"/></svg>
      </button>
      <div class="agent-ws-input-actions-right">
        <button class="agent-ws-attach-btn" :title="t('agent.attach_file')" @click="emit('attachFiles')">
          <svg viewBox="0 0 16 16"><path d="M13 7.5L8 12.5a3 3 0 01-4.2-4.2l5-5a2 2 0 012.8 2.8l-5 5a1 1 0 01-1.4-1.4l4.3-4.3" stroke="currentColor" stroke-width="1.3" stroke-linecap="round" stroke-linejoin="round" fill="none"/></svg>
        </button>
        <button
          class="btn-primary agent-ws-submit"
          :class="{ 'agent-ws-stop': canStop }"
          :disabled="submitting && !canStop"
          :title="canStop ? t('agent.stop_hint') : ''"
          @click="canStop ? emit('cancel') : emit('submit')"
        >
          {{ canStop ? (cancelling ? t('agent.stopping') : t('agent.stop')) : submitting ? t('agent.running') : t('agent.submit') }}
        </button>
      </div>

      <!-- 模型选择菜单：定位在操作行上方（bottom:100%），紧贴左侧模型按钮弹出 -->
      <div v-if="showModelMenu" class="agent-ws-menu agent-ws-menu-model">
        <div class="agent-ws-menu-title">{{ t('agent.model_pick') }}</div>
        <!-- 单次覆盖开关：开启后选择只作用于下一次提交，不改会话长期模型 -->
        <button
          class="agent-ws-menu-item agent-ws-menu-once"
          :class="{ selected: modelOnce }"
          :title="t('agent.model_once_hint')"
          @click="emit('toggleModelOnce')"
        >
          <span class="agent-ws-menu-check">{{ modelOnce ? '✓' : '' }}</span>
          <span class="agent-ws-menu-main">{{ t('agent.model_once') }}</span>
        </button>
        <button
          class="agent-ws-menu-item"
          :class="{ selected: !effectiveModel }"
          @mouseenter="emit('model-hover', 0)"
          @click="emit('pickModel', null)"
        >
          <span class="agent-ws-menu-main">{{ t('agent.model_default') }}</span>
          <span v-if="modelDefaultSub" class="agent-ws-menu-sub">{{ modelDefaultSub }}</span>
        </button>
        <button
          v-for="(m, i) in availableModels"
          :key="`${m.provider}:${m.id}`"
          class="agent-ws-menu-item"
          :class="{ selected: isModelSelected(m) }"
          @mouseenter="emit('model-hover', i + 1)"
          @click="emit('pickModel', m)"
        >
          <span class="agent-ws-menu-main">{{ modelLabel(m) }}</span>
          <span class="agent-ws-menu-sub">{{ m.provider }} · {{ m.id }}</span>
        </button>
        <div v-if="availableModels.length === 0" class="agent-ws-menu-empty">{{ t('agent.model_none') }}</div>
      </div>
    </div>

    <!-- 会话上下文水位（消息数 / 词元 / 成本）：对齐主流 chat 应用惯例放在输入框下方，
         顶部不再堆叠「状态横幅 + 水位条」两条；接近上限时警告色 + 新建会话快捷入口 -->
    <div v-if="usageText" class="agent-ws-usage" :class="{ warn: usageWarn }">
      <span class="agent-ws-usage-text" :title="usageWarn ? usageText : undefined">{{ usageWarn ? t('agent.context_near_limit') : usageText }}</span>
      <span v-if="!usageWarn && usage && !usage.has_usage" class="agent-ws-usage-est" :title="t('agent.usage_estimate_hint')">≈</span>
      <button v-if="usageWarn" class="btn-sm agent-ws-usage-new" :title="usageText ?? ''" @click="emit('newSession')">{{ t('agent.session_new') }}</button>
    </div>

    <!-- @ Skill 菜单 -->
    <div v-if="showSkillMenu" class="agent-ws-menu">
      <div class="agent-ws-menu-title">{{ t('agent.skill_pick') }}</div>
      <button
        v-for="(s, i) in filteredSkills"
        :key="s"
        class="agent-ws-menu-item"
        :class="{ selected: i === skillMenuIndex }"
        @mouseenter="emit('skill-hover', i)"
        @click="emit('pickSkill', s)"
      >
        <span class="agent-ws-menu-main">@{{ skillShortName(s) }}</span>
        <span class="agent-ws-menu-sub">{{ s }}</span>
      </button>
      <div v-if="filteredSkills.length === 0" class="agent-ws-menu-empty">
        {{ skills.length === 0 ? t('agent.no_skills') : t('agent.no_match') }}
      </div>
    </div>

    <!-- [[ 实体菜单 -->
    <div v-if="showEntityMenu" class="agent-ws-menu agent-ws-menu-entities">
      <div class="agent-ws-menu-title">{{ t('agent.entity_pick') }}</div>
      <template v-if="filteredSources.length">
        <div class="agent-ws-menu-group">
          <svg class="agent-ws-menu-group-icon"><use href="/icons.svg#source-icon" /></svg>
          {{ t('agent.entity_source') }} ({{ filteredSourcesCount }})
        </div>
        <button
          v-for="(s, i) in filteredSources"
          :key="`s${s.id}`"
          class="agent-ws-menu-item"
          :class="{ selected: i === entityMenuIndex }"
          @mouseenter="emit('entity-hover', i)"
          @click="emit('pickEntity', 'source', s.id)"
        >
          <span class="agent-ws-menu-main">{{ sourceDisplayName(s) }}</span>
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
          @mouseenter="emit('entity-hover', filteredSources.length + i)"
          @click="emit('pickEntity', 'release', r.id)"
        >
          <span class="agent-ws-menu-main">{{ releaseDisplayName(r) }}</span>
          <span class="agent-ws-menu-sub">{{ formatDate(r.published_at) }}</span>
        </button>
      </template>
      <div v-if="!entityMenuHasMatch" class="agent-ws-menu-empty">{{ t('agent.no_match') }}</div>
    </div>

    <!-- 引用 chip 全文悬浮提示（跟随鼠标，仅文本截断时显示） -->
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
/* 引用加入的就地高亮——替代原先压在发送按钮上的 Toast。
   只做描边/底色 + 光晕，不改尺寸位移，避免 chip 换行引起输入区抖动。
   动画时长须与脚本里的 FLASH_DURATION（1200ms）保持一致 */
.agent-ws-chip-attached.is-new {
  border-color: var(--accent, #2e6fd0);
  background: rgba(46, 111, 208, 0.14);
  animation: agent-ws-chip-flash 1.2s ease-out;
}
@keyframes agent-ws-chip-flash {
  0% {
    background: rgba(46, 111, 208, 0.28);
    box-shadow: 0 0 0 0 rgba(46, 111, 208, 0.45);
  }
  35% {
    box-shadow: 0 0 0 3px rgba(46, 111, 208, 0.18);
  }
  100% {
    background: rgba(46, 111, 208, 0.14);
    box-shadow: 0 0 0 0 rgba(46, 111, 208, 0);
  }
}
@media (prefers-reduced-motion: reduce) {
  .agent-ws-chip-attached.is-new {
    animation: none;
  }
}
/* 屏幕阅读器专用（视觉不可见）：承接 Toast 原先的告知作用。
   absolute 定位使其脱离 flex 流，不会给 chip 行挤进额外间距 */
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
  font-size: 13px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--bg);
  color: var(--text);
  line-height: 1.5;
  font-family: inherit;
}
.agent-ws-textarea:focus {
  outline: none;
  border-color: var(--accent, #2e6fd0);
}
.agent-ws-submit {
  flex-shrink: 0;
  /* 对齐 composer 30px/8px 基线，覆盖全局 .btn-primary 的 padding/字号/圆角 */
  height: 30px;
  padding: 0 16px;
  font-size: 12px;
  border-radius: 8px;
}
.agent-ws-submit.agent-ws-stop {
  background: #d64545;
  border-color: #d64545;
}
.agent-ws-submit.agent-ws-stop:hover {
  background: #c0392b;
  border-color: #c0392b;
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
  border-radius: 8px;
  cursor: pointer;
  transition: border-color 0.12s ease, background 0.12s ease, color 0.12s ease;
}
.agent-ws-attach-btn:hover {
  color: var(--text);
  border-color: var(--accent, #2e6fd0);
  background: rgba(46, 111, 208, 0.08);
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
  font-size: 12px;
  color: var(--text);
  background: var(--bg-subtle);
  border: 1px solid var(--border);
  border-radius: 8px;
  cursor: pointer;
  transition: border-color 0.12s ease, background 0.12s ease;
}
.agent-ws-model-btn:hover,
.agent-ws-model-btn.open {
  border-color: var(--accent, #2e6fd0);
  background: rgba(46, 111, 208, 0.08);
}
.agent-ws-model-icon {
  width: 12px;
  height: 12px;
  color: var(--accent, #2e6fd0);
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
  transition: transform 0.12s ease;
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
  font-size: 11px;
  color: #2e6fd0;
}

/* 会话上下文水位：移至输入框下方（对齐主流 chat 应用惯例），顶部不再堆叠
 * 「状态横幅 + 水位条」两条；接近上限时警告色 + 新建会话快捷入口 */
.agent-ws-usage {
  display: flex;
  align-items: center;
  gap: 6px;
  margin-top: 6px;
  font-size: 11px;
  color: var(--text-muted);
}
.agent-ws-usage.warn {
  color: #b0882e;
}
.agent-ws-usage-text {
  flex: 1;
  min-width: 0;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}
.agent-ws-usage-new {
  flex-shrink: 0;
  height: 20px;
  padding: 0 8px;
  font-size: 11px;
}
/* 引用 chip 全文悬浮提示（fixed 避免被消息区滚动容器裁剪） */
.agent-ws-chip-tooltip {
  position: fixed;
  z-index: 10002;
  max-width: min(480px, calc(100vw - 32px));
  padding: 9px 12px;
  background: var(--surface);
  border: 1px solid var(--border);
  border-radius: var(--radius-sm, 8px);
  box-shadow: var(--shadow-lg, 0 4px 16px rgba(0, 0, 0, 0.25));
  color: var(--text);
  font-size: 12px;
  line-height: 1.55;
  overflow-wrap: anywhere;
  pointer-events: none;
}

/* 估算标记：pi 未上报用量时，词元数是按字符数估的，标一个 ≈ 免得被当成精确计费值 */
.agent-ws-usage-est {
  flex-shrink: 0;
  font-style: normal;
  opacity: 0.65;
  cursor: help;
}
</style>
