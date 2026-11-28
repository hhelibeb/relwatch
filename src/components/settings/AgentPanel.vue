<script setup lang="ts">
import { ref } from 'vue'
import {
  AGENT_MAX_CONCURRENCY_LIMIT,
  AGENT_TIMEOUT_MAX,
  AGENT_TIMEOUT_MIN,
} from '../../api/agent'
import { t } from '../../i18n'
import { skillShortName } from '../../utils'
import { useUiStore } from '../../stores/ui'
import AppIcon from '../ui/AppIcon.vue'

export interface AgentDraftForm {
  enabled: boolean
  agentType: string
  binary: string
  model: string
  workingDir: string
  promptSuffix: string
  timeout: number
  maxConcurrency: number
  skills: string[]
}

const draft = defineModel<AgentDraftForm>('draft', { required: true })

const ui = useUiStore()
const newAgentSkill = ref('')

function addAgentSkill() {
  const p = newAgentSkill.value.trim()
  if (!p) return
  if (draft.value.skills.includes(p)) {
    ui.showToast(t('agent.skill_duplicated'))
    return
  }
  draft.value.skills.push(p)
  newAgentSkill.value = ''
}

function removeAgentSkill(index: number) {
  draft.value.skills.splice(index, 1)
}
</script>

<template>
  <div class="settings-form">
    <div class="setting-section-title">{{ t('agent.section_title') }}</div>
    <p class="setting-section-desc">{{ t('agent.section_desc') }}</p>
    <label class="setting-row setting-row-checkbox">
      <input type="checkbox" v-model="draft.enabled" />
      <span class="setting-label">{{ t('agent.enabled_global') }}</span>
    </label>
    <template v-if="draft.enabled">
      <label class="setting-row">
        <span class="setting-label">{{ t('agent.type') }}</span>
        <select v-model="draft.agentType" class="select-input setting-select">
          <option value="pi">{{ t('agent.type_pi') }}</option>
        </select>
      </label>
      <label class="setting-row">
        <span class="setting-label">{{ t('agent.binary_path') }}</span>
        <input
          type="text"
          v-model="draft.binary"
          :placeholder="t('agent.binary_path_placeholder')"
          class="setting-input"
        />
      </label>
      <label class="setting-row">
        <span class="setting-label">{{ t('agent.model') }}</span>
        <input
          type="text"
          v-model="draft.model"
          :placeholder="t('agent.model_placeholder')"
          class="setting-input"
        />
      </label>
      <label class="setting-row">
        <span class="setting-label">{{ t('agent.working_dir') }}</span>
        <input
          type="text"
          v-model="draft.workingDir"
          :placeholder="t('agent.working_dir_placeholder')"
          class="setting-input"
        />
      </label>
      <label class="setting-row">
        <span class="setting-label">{{ t('agent.prompt_suffix') }}</span>
        <input
          type="text"
          v-model="draft.promptSuffix"
          :placeholder="t('agent.prompt_suffix_placeholder')"
          class="setting-input"
        />
      </label>
      <label class="setting-row">
        <span class="setting-label">{{ t('agent.timeout_seconds') }}</span>
        <input
          type="number"
          v-model.number="draft.timeout"
          :min="AGENT_TIMEOUT_MIN"
          :max="AGENT_TIMEOUT_MAX"
          class="setting-input setting-input-narrow agent-num-input"
        />
      </label>
      <label class="setting-row">
        <span class="setting-label">{{ t('agent.max_concurrency') }}</span>
        <input
          type="number"
          v-model.number="draft.maxConcurrency"
          min="1"
          :max="AGENT_MAX_CONCURRENCY_LIMIT"
          class="setting-input setting-input-narrow agent-num-input"
          :title="t('agent.max_concurrency_hint')"
        />
      </label>
      <p class="setting-section-desc agent-concurrency-hint">{{ t('agent.max_concurrency_hint') }}</p>
      <div class="setting-row setting-row-skills">
        <span class="setting-label">{{ t('agent.skill_path') }}</span>
        <div class="agent-skill-list">
          <div v-for="(sp, i) in draft.skills" :key="sp" class="agent-skill-item">
            <span class="agent-skill-path" :title="sp">{{ skillShortName(sp) }}</span>
            <button type="button" class="agent-skill-remove" :title="t('agent.remove_skill')" @click="removeAgentSkill(i)">
              <AppIcon name="close-icon" :size="12" />
            </button>
          </div>
          <div class="agent-skill-add">
            <input
              v-model="newAgentSkill"
              class="setting-input"
              :placeholder="t('agent.skill_path_placeholder')"
              @keydown.enter.prevent="addAgentSkill"
            />
            <button type="button" class="btn-secondary" :disabled="!newAgentSkill.trim()" @click="addAgentSkill">{{ t('agent.add_skill') }}</button>
          </div>
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
.agent-num-input {
  width: calc(8ch * 1.25);
}

.agent-concurrency-hint {
  margin-top: -4px;
}

.setting-row-skills {
  align-items: flex-start;
}

.setting-row-skills .setting-label {
  padding-top: 8px;
}

.agent-skill-list {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.agent-skill-item {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 5px 8px;
  border: 1px solid var(--border);
  border-radius: var(--radius);
  background: var(--bg-subtle);
}

.agent-skill-path {
  flex: 1;
  min-width: 0;
  font-size: var(--fs-meta);
  font-family: var(--mono-font);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.agent-skill-remove {
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  width: 20px;
  height: 20px;
  padding: 0;
  border: none;
  background: transparent;
  color: var(--text-muted);
  cursor: pointer;
  border-radius: var(--radius-sm);
}

.agent-skill-remove:hover {
  color: var(--danger);
  background: var(--bg-hover);
}

.agent-skill-add {
  display: flex;
  gap: 6px;
}

.agent-skill-add .setting-input {
  flex: 1;
}
</style>
