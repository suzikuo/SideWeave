<script setup lang="ts">
import { computed, nextTick, onMounted, ref } from 'vue'
import { listen } from '@tauri-apps/api/event'
import SidebarView from '../views/SidebarView.vue'
import SkinSurfaceHost from '../views/SkinSurfaceHost.vue'
import { appReady, EVENTS, getState } from '../lib/api'
import { startupDebug } from '../lib/startupDebug'
import { bindSettings } from '../lib/theme'
import type { PluginMeta, Settings } from '../lib/types'

const settings = ref<Settings | null>(null)
const plugins = ref<PluginMeta[]>([])
const sidebarPlugins = computed(() =>
  plugins.value.filter((plugin) => plugin.enabled && plugin.sidebar_visible)
)
const expanded = ref(true)
const customSidebarReady = ref(false)

void listen<boolean>(EVENTS.sidebarExpanded, (event) => {
  expanded.value = event.payload
}).catch(() => {})

onMounted(async () => {
  startupDebug('sidebar: App onMounted begin')
  const snapshot = await bindSettings((fresh) => {
    settings.value = fresh
    void refreshPlugins()
  })
  startupDebug('sidebar: state and settings bound')
  plugins.value = snapshot.plugins
  if (!settings.value) settings.value = snapshot.settings
  await nextTick()
  startupDebug('sidebar: app_ready begin')
  await appReady()
  startupDebug('sidebar: app_ready finished')
})

async function refreshPlugins() {
  try {
    plugins.value = (await getState()).plugins
  } catch {
  }
}
</script>

<template>
  <SidebarView
    v-if="settings && !customSidebarReady"
    :appearance="settings.appearance"
    :plugins="plugins"
    :expanded="expanded"
    :general="settings.general"
  />
  <SkinSurfaceHost
    v-if="settings"
    :skin-id="settings.appearance.skin"
    surface="sidebar"
    @surface-ready="customSidebarReady = $event"
    :context="{ settings, plugins: sidebarPlugins, expanded }"
  />
</template>
