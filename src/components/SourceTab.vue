<script setup lang="ts">
import { computed, inject, ref, nextTick, onMounted, onUnmounted } from 'vue'
import { ShowToastKey } from '../injection-keys'
import { message } from '@tauri-apps/plugin-dialog'
import { type Source, parseGitHubUrl, addSource, removeSource, updateSource } from '../api/sources'
import { checkSingleSource } from '../api/releases'
import { openReleaseUrl } from '../api/client'
import { useContextMenu } from '../composables/useContextMenu'
import ContextMenu from './common/ContextMenu.vue'
import { t, tm } from '../i18n'
import { formatDate } from '../utils'

const props = defineProps<{ sources: Source[]; polling: boolean; unreadReleaseCounts: Record<string, number>; totalReleaseCounts: Record<string, number> }>()
const emit = defineEmits<{
  update: []
  checkResult: [count: number]
  checkBusy: [busy: boolean]
  openReleases: [query: string]
  openUnreadReleases: [query: string]
}>()

const urlInput = ref('')
const loading = ref(false)
const checkingId = ref<number | null>(null)
const highlightedId = ref<number | null>(null)
const openMoreId = ref<number | null>(null)
const showToast = inject(ShowToastKey, () => {})

const { contextMenu, handleContextMenu, handleCopyLink, handleOpenLink } = useContextMenu()

function openSourceUrl(owner: string, repo: string) {
  openReleaseUrl(`https://github.com/${owner}/${repo}`)
}

function sourceQuery(source: Source): string {
  return `${source.owner}/${source.repo}`
}

function sourceKey(source: Source): string {
  return sourceQuery(source).toLowerCase()
}

function unreadReleaseCount(source: Source): number {
  return props.unreadReleaseCounts[sourceKey(source)] || 0
}

function openSourceReleases(source: Source) {
  emit('openReleases', sourceQuery(source))
}

function openSourceUnreadReleases(source: Source) {
  emit('openUnreadReleases', sourceQuery(source))
}

function sourceExists(owner: string, repo: string): boolean {
  return props.sources.some(source =>
    source.owner.toLowerCase() === owner.toLowerCase() &&
    source.repo.toLowerCase() === repo.toLowerCase()
  )
}

async function handleAdd() {
  const raw = urlInput.value.trim()
  if (!raw) return
  const parsed = parseGitHubUrl(raw)
  if (!parsed) {
    await message(t('source.invalid_url'), { title: t('source.input_invalid'), kind: 'warning' })
    return
  }
  if (sourceExists(parsed.owner, parsed.repo)) {
    showToast?.(t('source.exists'))
    return
  }
  loading.value = true
  try {
    const id = await addSource('github', parsed.owner, parsed.repo)
    if (id === 0) {
      showToast?.(t('source.exists'))
      return
    }
    urlInput.value = ''
    highlightedId.value = id
    setTimeout(() => { highlightedId.value = null }, 2200)
    emit('update')
  } catch (e: unknown) {
    const errMsg = e instanceof Error ? e.message : String(e)
    await message(tm('source.add_failed', { source_type: 'github', owner: parsed.owner, repo: parsed.repo, error: errMsg }), { title: t('settings.error'), kind: 'error' })
    emit('update')
  } finally {
    loading.value = false
  }
}

async function handleRemove(id: number) {
  try {
    await removeSource(id)
    emit('update')
  } catch (e: unknown) {
    await message(t('source.delete_failed') + (e instanceof Error ? e.message : String(e)), { title: t('settings.error'), kind: 'error' })
  }
}

async function handleToggle(source: Source) {
  try {
    await updateSource(source.id, !source.enabled, source.poll_interval_minutes)
    emit('update')
  } catch (e: unknown) {
    await message(t('source.operation_failed') + (e instanceof Error ? e.message : String(e)), { title: t('settings.error'), kind: 'error' })
  }
  openMoreId.value = null
}

async function handleMuteToggle(source: Source) {
  if (!source.enabled) return
  try {
    await updateSource(source.id, source.enabled, source.poll_interval_minutes, !source.muted)
    emit('update')
  } catch (e: unknown) {
    await message(t('source.operation_failed') + (e instanceof Error ? e.message : String(e)), { title: t('settings.error'), kind: 'error' })
  }
  openMoreId.value = null
}

function toggleMore(id: number) {
  openMoreId.value = openMoreId.value === id ? null : id
}

function onDocumentClick() {
  openMoreId.value = null
}

onMounted(() => document.addEventListener('click', onDocumentClick))
onUnmounted(() => document.removeEventListener('click', onDocumentClick))

async function handleCheckSingle(id: number) {
  if (props.polling || checkingId.value !== null) return
  checkingId.value = id
  emit('checkBusy', true)
  try {
    const result = await checkSingleSource(id)
    emit('update')
    emit('checkResult', result.new_releases.length)
  } catch (e: unknown) {
    await message(t('source.check_failed') + (e instanceof Error ? e.message : String(e)), { title: t('settings.error'), kind: 'error' })
  } finally {
    checkingId.value = null
    emit('checkBusy', false)
  }
}

function sourceHealthClass(source: Source): string {
  if (!source.enabled) return 'health-paused'
  if (source.last_check_status === 'ok') return 'health-ok'
  if (source.last_check_status === 'error') return 'health-error'
  return 'health-unknown'
}

function sourceHealthLabel(source: Source): string {
  if (!source.enabled) return t('source.health_paused')
  if (source.muted) return t('source.muted')
  if (source.last_check_status === 'ok') {
    return t('source.no_pending_updates')
  }
  if (source.last_check_status === 'error') return t('source.health_error')
  return t('source.health_unknown')
}

const sortedSources = computed(() => {
  return [...props.sources].sort((a, b) => {
    const aPending = props.unreadReleaseCounts[sourceKey(a)] || 0
    const bPending = props.unreadReleaseCounts[sourceKey(b)] || 0
    return bPending - aPending || b.id - a.id
  })
})

function sourceCheckedText(source: Source): string {
  if (!source.last_checked_at) return t('source.never_checked')
  return t('source.last_checked', formatDate(source.last_checked_at))
}

function sourceHealthAriaLabel(source: Source): string {
  if (!source.enabled) return t('source.health_paused')
  if (source.last_check_status === 'ok') return t('source.health_normal')
  if (source.last_check_status === 'error') return t('source.health_error')
  return t('source.health_unknown')
}

const tooltip = ref<{ visible: boolean; x: number; y: number; lines: { text: string; wrap: boolean }[] }>({ visible: false, x: 0, y: 0, lines: [] })

function showHealthTooltip(e: MouseEvent, source: Source) {
  const lines: { text: string; wrap: boolean }[] = []
  let statusText: string
  if (!source.enabled) statusText = t('source.health_paused')
  else if (source.last_check_status === 'ok') statusText = t('source.health_normal')
  else if (source.last_check_status === 'error') statusText = t('source.health_error')
  else statusText = t('source.health_unknown')
  lines.push({ text: t('source.tooltip_status') + statusText, wrap: false })
  const count = props.totalReleaseCounts[sourceKey(source)]
  if (count > 0) {
    lines.push({ text: t('source.tooltip_history') + t('source.recorded_versions', String(count)), wrap: false })
  }
  if (source.description) {
    lines.push({ text: t('source.tooltip_about') + source.description, wrap: true })
  }
  tooltip.value = { visible: true, x: e.clientX + 10, y: e.clientY + 10, lines }
  nextTick(() => {
    const el = document.querySelector('.source-health-tooltip') as HTMLElement | null
    if (!el) return
    const rect = el.getBoundingClientRect()
    let { x, y } = tooltip.value
    if (rect.right > window.innerWidth - 4) x = e.clientX - rect.width - 10
    if (rect.bottom > window.innerHeight - 4) y = e.clientY - rect.height - 10
    tooltip.value.x = Math.max(4, x)
    tooltip.value.y = Math.max(4, y)
  })
}

function hideHealthTooltip() {
  tooltip.value.visible = false
}
</script>

<template>
  <section class="tab-content">
    <div class="add-source">
      <div class="input-clear-wrap">
        <input
          v-model="urlInput"
          :placeholder="t('source.placeholder')"
          @keyup.enter="handleAdd"
        />
        <button v-if="urlInput" type="button" class="input-clear-btn" :title="t('input.clear')" @click="urlInput = ''">✕</button>
      </div>
      <button :disabled="loading || !urlInput" @click="handleAdd">{{ t('source.add') }}</button>
    </div>
    <div class="source-list">
      <div v-if="props.sources.length === 0" class="empty">{{ t('source.empty') }}</div>
      <div v-for="source in sortedSources" :key="source.id" class="source-item" :class="{ 'source-highlight': source.id === highlightedId }">
        <div class="source-main">
          <div class="source-info">
            <span class="source-name">{{ source.owner }}/{{ source.repo }}</span>
            <button class="btn-icon-link" @click="openSourceUrl(source.owner, source.repo)" @contextmenu.prevent.stop="handleContextMenu($event, `https://github.com/${source.owner}/${source.repo}`)" :title="t('source.visit')">
              <svg><use href="/icons.svg#link-icon"/></svg>
            </button>
            <button class="btn-icon-link" @click="openSourceReleases(source)" :title="t('source.view_releases')">
              <svg><use href="/icons.svg#search-icon"/></svg>
            </button>
            <span v-if="source.enabled && source.muted" class="badge badge-muted">{{ t('source.muted') }}</span>
            <span v-else-if="source.enabled" class="badge badge-on">{{ t('source.enabled') }}</span>
            <span v-else class="badge badge-off">{{ t('source.paused') }}</span>
          </div>
          <div class="source-health">
            <span class="health-dot" :class="sourceHealthClass(source)" :aria-label="sourceHealthAriaLabel(source)" @mouseenter="showHealthTooltip($event, source)" @mouseleave="hideHealthTooltip"></span>
            <template v-if="source.enabled && source.last_check_status === 'ok'">
              <button
                v-if="unreadReleaseCount(source) > 0"
                class="source-pending-link"
                @click="openSourceUnreadReleases(source)"
              >
                {{ t('source.pending_updates', String(unreadReleaseCount(source))) }}
              </button>
              <span class="source-health-meta">{{ sourceCheckedText(source) }}</span>
            </template>
            <template v-else>
              <span class="source-health-label">{{ sourceHealthLabel(source) }}</span>
              <span class="source-health-meta">{{ sourceCheckedText(source) }}</span>
              <span v-if="source.consecutive_failures > 0" class="source-health-meta">
                {{ t('source.failure_count', String(source.consecutive_failures)) }}
              </span>
            </template>
          </div>
          <div v-if="source.last_check_status === 'error' && source.last_check_message" class="source-error" :title="source.last_check_message">
            {{ source.last_check_message }}
          </div>
        </div>
        <div class="source-actions">
          <button class="btn-icon-action btn-check" :disabled="props.polling || checkingId !== null" @click="handleCheckSingle(source.id)" :title="checkingId === source.id ? t('source.checking') : t('source.check')">
            <svg><use href="/icons.svg#refresh-icon"/></svg>
          </button>
          <button class="btn-icon-action" :class="source.enabled ? 'btn-pause' : 'btn-resume'" @click="handleToggle(source)" :title="source.enabled ? t('source.pause') : t('source.resume')">
            <svg><use :href="source.enabled ? '/icons.svg#pause-icon' : '/icons.svg#play-icon'"/></svg>
          </button>
          <div class="dropdown-more">
            <button class="btn-icon-action btn-more" @click.stop="toggleMore(source.id)" :title="t('source.more')">
              <svg><use href="/icons.svg#more-icon"/></svg>
            </button>
            <div v-if="openMoreId === source.id" class="dropdown-more-panel" @click.stop>
              <button class="dropdown-item" :disabled="!source.enabled" :title="!source.enabled ? t('source.mute_disabled_tip') : ''" @click="handleMuteToggle(source)">
                <span class="dropdown-icon"><svg><use :href="source.muted ? '/icons.svg#bell-icon' : '/icons.svg#bell-off-icon'"/></svg></span>
                {{ source.muted ? t('source.unmute') : t('source.mute') }}
              </button>
              <button class="dropdown-item dropdown-item-danger" @click="handleRemove(source.id)">
                <span class="dropdown-icon"><svg><use href="/icons.svg#trash-icon"/></svg></span>
                {{ t('source.delete') }}
              </button>
            </div>
          </div>
        </div>
      </div>
    </div>
    <ContextMenu v-if="contextMenu" :x="contextMenu.x" :y="contextMenu.y" @open="handleOpenLink" @copy="handleCopyLink" />
    <div v-if="tooltip.visible" class="source-health-tooltip" :style="{ left: tooltip.x + 'px', top: tooltip.y + 'px' }">
      <div v-for="(line, i) in tooltip.lines" :key="i" class="tooltip-line" :class="{ 'tooltip-line-wrap': line.wrap }">{{ line.text }}</div>
    </div>
  </section>
</template>
