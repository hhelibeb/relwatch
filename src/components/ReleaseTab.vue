<script setup lang="ts">
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { type ReleaseInfo, openReleaseUrl } from '../api'
import { t } from '../i18n'
import { importanceLabel, formatDate } from '../utils'

const props = defineProps<{ releases: ReleaseInfo[] }>()

const releaseSearch = ref('')

const filteredReleases = computed(() => {
  const q = releaseSearch.value.trim().toLowerCase()
  if (!q) return props.releases
  return props.releases.filter(r =>
    r.owner.toLowerCase().includes(q) ||
    r.repo.toLowerCase().includes(q) ||
    r.tag_name.toLowerCase().includes(q) ||
    r.release_name.toLowerCase().includes(q) ||
    (r.body || '').toLowerCase().includes(q)
  )
})

const contextMenu = ref<{ x: number; y: number; url: string } | null>(null)

function closeContextMenu() { contextMenu.value = null }
onMounted(() => document.addEventListener('click', closeContextMenu))
onUnmounted(() => document.removeEventListener('click', closeContextMenu))

function handleOpenUrl(url: string) {
  openReleaseUrl(url)
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
</script>

<template>
  <section class="tab-content">
    <div class="log-search">
      <input
        v-model="releaseSearch"
        :placeholder="t('release.search')"
        class="search-input"
      />
    </div>
    <div class="release-list">
      <div v-if="filteredReleases.length === 0" class="empty">{{ releaseSearch ? t('release.no_match') : t('release.empty') }}</div>
      <div v-for="release in filteredReleases" :key="release.id" class="release-item"
         :class="{ 'is-prerelease': release.prerelease }">
        <div class="release-header">
          <span class="release-repo">{{ release.owner }}/{{ release.repo }}</span>
          <span class="release-tag">{{ release.tag_name }}</span>
          <button class="btn-icon-link" @click="handleOpenUrl(release.html_url)" @contextmenu.prevent.stop="handleContextMenu($event, release.html_url)" :title="t('release.open_link')">
            <svg><use href="/icons.svg#link-icon"/></svg>
          </button>
          <span v-if="release.prerelease" class="badge badge-pre">{{ t('release.prerelease') }}</span>
          <span class="release-date">{{ t('release.published_at', formatDate(release.published_at)) }}</span>
        </div>
        <div class="release-title">{{ release.release_name }}</div>
        <div v-if="release.ai_summary" class="release-ai-summary">
          <span class="ai-badge" :class="'ai-' + (release.ai_importance || '')">{{ importanceLabel(release.ai_importance) }}</span>
          {{ release.ai_summary }}
        </div>
      </div>
    </div>
    <div v-if="contextMenu" class="context-menu" :style="{ left: contextMenu.x + 'px', top: contextMenu.y + 'px' }" @click.stop>
      <button @click="handleOpenLink">{{ t('context.open') }}</button>
      <button @click="handleCopyLink">{{ t('context.copy_link') }}</button>
    </div>
  </section>
</template>
