<script setup lang="ts">
import { ref, onMounted, onUnmounted } from 'vue'
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

const props = defineProps<{ sources: Source[]; polling: boolean }>()
const emit = defineEmits<{ update: []; checkResult: [count: number] }>()

const urlInput = ref('')
const loading = ref(false)
const checkingId = ref<number | null>(null)

const contextMenu = ref<{ x: number; y: number; url: string } | null>(null)

function closeContextMenu() { contextMenu.value = null }
onMounted(() => document.addEventListener('click', closeContextMenu))
onUnmounted(() => document.removeEventListener('click', closeContextMenu))

function openSourceUrl(owner: string, repo: string) {
  openReleaseUrl(`https://github.com/${owner}/${repo}`)
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
  loading.value = true
  try {
    await addSource('github', parsed.owner, parsed.repo)
    urlInput.value = ''
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
  checkingId.value = id
  try {
    const result = await checkSingleSource(id)
    emit('update')
    emit('checkResult', result.new_releases.length)
  } catch (e: any) {
    await message(t('source.check_failed') + (e?.toString?.() ?? String(e)), { title: t('settings.error'), kind: 'error' })
  } finally {
    checkingId.value = null
  }
}
</script>

<template>
  <section class="tab-content">
    <div class="add-source">
      <input
        v-model="urlInput"
        :placeholder="t('source.placeholder')"
        @keyup.enter="handleAdd"
      />
      <button :disabled="loading || !urlInput" @click="handleAdd">{{ t('source.add') }}</button>
    </div>
    <div class="source-list">
      <div v-if="props.sources.length === 0" class="empty">{{ t('source.empty') }}</div>
      <div v-for="source in props.sources" :key="source.id" class="source-item">
        <div class="source-info">
          <span class="source-name">{{ source.owner }}/{{ source.repo }}</span>
          <button class="btn-icon-link" @click="openSourceUrl(source.owner, source.repo)" @contextmenu.prevent.stop="handleContextMenu($event, `https://github.com/${source.owner}/${source.repo}`)" :title="t('source.visit')">
            <svg><use href="/icons.svg#link-icon"/></svg>
          </button>
          <span v-if="source.enabled" class="badge badge-on">{{ t('source.enabled') }}</span>
          <span v-else class="badge badge-off">{{ t('source.paused') }}</span>
        </div>
        <div class="source-actions">
          <button class="btn-sm btn-check" :disabled="checkingId === source.id || (props.polling && source.enabled)" @click="handleCheckSingle(source.id)">{{ checkingId === source.id || (props.polling && source.enabled) ? t('source.checking') : t('source.check') }}</button>
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
  </section>
</template>
