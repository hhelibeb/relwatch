<script setup lang="ts">
import { computed, inject, ref, onMounted, onUnmounted, nextTick } from 'vue'
import { ShowToastKey } from '../injection-keys'
import { message } from '@tauri-apps/plugin-dialog'
import {
  type Source,
  parseGitHubUrl,
  addSource,
  removeSource,
  updateSource,
  checkSingleSource,
  openReleaseUrl,
} from '../api'
import { t } from '../i18n'
import { formatDate } from '../utils'

const props = defineProps<{ sources: Source[]; polling: boolean; unreadReleaseCounts: Record<string, number>; totalReleaseCounts: Record<string, number> }>()
const emit = defineEmits<{
  update: []
  checkResult: [count: number]
  checkBusy: [busy: boolean]
  openReleases: [query: string]
}>()

const urlInput = ref('')
const loading = ref(false)
const checkingId = ref<number | null>(null)
const highlightedId = ref<number | null>(null)
const showToast = inject(ShowToastKey, () => {})

const contextMenu = ref<{ x: number; y: number; url: string } | null>(null)

function closeContextMenu() { contextMenu.value = null }
onMounted(() => document.addEventListener('click', closeContextMenu))
onUnmounted(() => document.removeEventListener('click', closeContextMenu))

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

function sourceExists(owner: string, repo: string): boolean {
  return props.sources.some(source =>
    source.owner.toLowerCase() === owner.toLowerCase() &&
    source.repo.toLowerCase() === repo.toLowerCase()
  )
}

function handleContextMenu(e: MouseEvent, url: string) {
  contextMenu.value = { x: e.clientX, y: e.clientY, url }
}

async function handleCopyLink() {
  try { await navigator.clipboard.writeText(contextMenu.value!.url) } catch { /* ignore */ }
  closeContextMenu()
}

function handleOpenLink() {
  openReleaseUrl(contextMenu.value!.url)
  closeContextMenu()
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
  } catch (e: any) {
    await message(t('source.add_failed') + (e?.toString?.() ?? String(e)), { title: t('settings.error'), kind: 'error' })
  } finally {
    loading.value = false
  }
}

async function handleRemove(id: number) {
  try {
    await removeSource(id)
    emit('update')
  } catch (e: any) {
    await message(t('source.delete_failed') + (e?.toString?.() ?? String(e)), { title: t('settings.error'), kind: 'error' })
  }
}

async function handleToggle(source: Source) {
  try {
    await updateSource(source.id, !source.enabled, source.poll_interval_minutes)
    emit('update')
  } catch (e: any) {
    await message(t('source.operation_failed') + (e?.toString?.() ?? String(e)), { title: t('settings.error'), kind: 'error' })
  }
}

async function handleCheckSingle(id: number) {
  if (props.polling || checkingId.value !== null) return
  checkingId.value = id
  emit('checkBusy', true)
  try {
    const result = await checkSingleSource(id)
    emit('update')
    emit('checkResult', result.new_releases.length)
  } catch (e: any) {
    await message(t('source.check_failed') + (e?.toString?.() ?? String(e)), { title: t('settings.error'), kind: 'error' })
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
            <span v-if="source.enabled" class="badge badge-on">{{ t('source.enabled') }}</span>
            <span v-else class="badge badge-off">{{ t('source.paused') }}</span>
          </div>
          <div class="source-health">
            <span class="health-dot" :class="sourceHealthClass(source)" :aria-label="sourceHealthAriaLabel(source)" @mouseenter="showHealthTooltip($event, source)" @mouseleave="hideHealthTooltip"></span>
            <template v-if="source.enabled && source.last_check_status === 'ok'">
              <button
                v-if="unreadReleaseCount(source) > 0"
                class="source-pending-link"
                @click="openSourceReleases(source)"
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
          <button class="btn-sm btn-check" :disabled="props.polling || checkingId !== null" @click="handleCheckSingle(source.id)">{{ checkingId === source.id ? t('source.checking') : t('source.check') }}</button>
          <button class="btn-sm" :class="source.enabled ? 'btn-yellow' : 'btn-green'" @click="handleToggle(source)">
            {{ source.enabled ? t('source.pause') : t('source.resume') }}
          </button>
          <button class="btn-sm btn-danger" @click="handleRemove(source.id)">{{ t('source.delete') }}</button>
        </div>
      </div>
    </div>
    <div v-if="contextMenu" class="context-menu" :style="{ left: contextMenu.x + 'px', top: contextMenu.y + 'px' }" @click.stop>
      <button @click="handleOpenLink">{{ t('context.open') }}</button>
      <button @click="handleCopyLink">{{ t('context.copy_link') }}</button>
    </div>
    <div v-if="tooltip.visible" class="source-health-tooltip" :style="{ left: tooltip.x + 'px', top: tooltip.y + 'px' }">
      <div v-for="(line, i) in tooltip.lines" :key="i" class="tooltip-line" :class="{ 'tooltip-line-wrap': line.wrap }">{{ line.text }}</div>
    </div>
  </section>
</template>
