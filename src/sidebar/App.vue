<script setup lang="ts">
import { computed, defineAsyncComponent, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import SidebarView from '../views/SidebarView.vue'
const SkinSurfaceHost = defineAsyncComponent(() => import('../views/SkinSurfaceHost.vue'))
import { appReady, EVENTS } from '../lib/api'
import { startupDebug } from '../lib/startupDebug'
import { bindSettings } from '../lib/theme'
import type { PluginMeta, Settings } from '../lib/types'

const settings = ref<Settings | null>(null)
const plugins = ref<PluginMeta[]>([])
const sidebarPlugins = computed(() =>
  plugins.value.filter((plugin) => plugin.enabled && plugin.sidebar_visible)
)
const isUserSkin = computed(() => Boolean(settings.value?.appearance.skin.startsWith('user:')))
const expanded = ref(true)
const customSidebarReady = ref(false)

watch(isUserSkin, (active) => {
  if (!active) customSidebarReady.value = false
})

let disposed = false
let unlistenExpanded: UnlistenFn | null = null
let unlistenPlugins: UnlistenFn | null = null

onMounted(async () => {
  startupDebug('sidebar: App onMounted begin')
  const stopExpanded = await listen<boolean>(EVENTS.sidebarExpanded, (event) => {
    expanded.value = event.payload
  }).catch(() => null)
  if (disposed) {
    stopExpanded?.()
    return
  }
  unlistenExpanded = stopExpanded

  const stopPlugins = await listen<PluginMeta[]>(EVENTS.pluginsChanged, (event) => {
    if (Array.isArray(event.payload)) {
      plugins.value = event.payload
    }
  }).catch(() => null)
  if (disposed) {
    stopPlugins?.()
    return
  }
  unlistenPlugins = stopPlugins

  const snapshot = await bindSettings((fresh) => {
    settings.value = fresh
  })
  if (disposed) return
  startupDebug('sidebar: state and settings bound')
  plugins.value = snapshot.plugins
  if (!settings.value) settings.value = snapshot.settings
  await nextTick()
  startupDebug('sidebar: app_ready begin')
  await appReady()
  startupDebug('sidebar: app_ready finished')
})

onUnmounted(() => {
  disposed = true
  unlistenExpanded?.()
  unlistenExpanded = null
  unlistenPlugins?.()
  unlistenPlugins = null
})
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
    v-if="settings && isUserSkin"
    :skin-id="settings.appearance.skin"
    surface="sidebar"
    @surface-ready="customSidebarReady = $event"
    :context="{ settings, plugins: sidebarPlugins, expanded }"
  />
</template>
