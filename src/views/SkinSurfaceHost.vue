<script setup lang="ts">
import { getCurrentWindow } from '@tauri-apps/api/window'
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { onMounted, onUnmounted, ref, watch } from 'vue'
import {
  backupExport,
  backupImport,
  closeDetail,
  getState,
  openControlCenter,
  openDataDir,
  openExternalUrl,
  openDetail,
  openPluginDetail,
  restartApp,
  setPluginEnabled,
  setPluginSidebarVisible,
  setPluginOrder,
  setSetting,
  toggleSidebar,
  updateCheck,
  updateIgnoreSet,
  updateInstall,
  sidebarHoverEnter,
  sidebarCollapseIfUnhovered,
  previewHideNow,
  previewSidebarEnter,
  previewSidebarLeave,
  previewWindowEnter,
  previewWindowLeave,
  getCurrentPreview,
  refreshPreview
} from '../lib/api'
import { applySkinWithRecommendedOpacity } from '../lib/skinAppearance'
import {
  marketInstall,
  marketInstalled,
  marketRefresh,
  marketUninstall,
  pluginImportLocal,
  skinUserUi
} from '../lib/marketApi'
import { pluginCall } from '../lib/pluginApi'
import type { SkinUi } from '../lib/marketApi'

type Surface = 'sidebar' | 'detail' | 'control' | 'preview'
type SkinContext = Record<string, unknown>

const props = withDefaults(defineProps<{
  skinId: string
  surface: Surface
  context?: SkinContext
  enabled?: boolean
}>(), {
  context: () => ({}),
  enabled: true
})

const emit = defineEmits<{
  (event: 'showNativeUi'): void
  (event: 'surfaceReady', ready: boolean): void
}>()

interface SkinRequest {
  __sideweaveSkin?: boolean
  __agileTilesSkin?: boolean
  skin_id?: string
  type?: string
  id?: number | string
  action?: string
  payload?: Record<string, unknown>
}

interface SkinReply {
  __sideweaveSkin: true
  __agileTilesSkin: true
  skin_id: string
  type: 'reply'
  id: number | string
  result?: unknown
  error?: string
}

const iframe = ref<HTMLIFrameElement | null>(null)
const src = ref('')
const activeSkinId = ref('')
const ready = ref(false)
const currentWindow = getCurrentWindow()
const unlisteners: UnlistenFn[] = []
let themeObserver: MutationObserver | null = null
let disposed = false
let loadToken = 0
let messageStartedAt = 0
let messageCount = 0
let hoverToken = 0
let pendingHoverEnter: Promise<number> | null = null
let hoverLeaveTimer = 0
const MAX_MESSAGE_BYTES = 256 * 1024
const MAX_MESSAGES_PER_SECOND = 120

function skinResourceUrl(id: string, ui: SkinUi, entry: string): string {
  const parts = [ui.assetRoot, entry]
    .filter((part) => part && part !== '.')
    .flatMap((part) => part.split('/'))
    .map((part) => encodeURIComponent(part))
  return `http://agile-plugin.localhost/@skin/${encodeURIComponent(id)}/${parts.join('/')}`
}

async function loadSurface() {
  const token = ++loadToken
  const id = props.skinId.startsWith('user:') ? props.skinId.slice(5) : ''
  ready.value = false
  emit('surfaceReady', false)
  src.value = ''
  activeSkinId.value = id
  if (!id || !props.enabled) return

  try {
    const ui = await skinUserUi(id)
    if (token !== loadToken || !ui) return
    const entry = ui[props.surface]
    if (!entry) return
    src.value = skinResourceUrl(id, ui, entry)
  } catch (error) {
    console.warn(`Could not load the ${props.surface} surface for skin ${id}:`, error)
  }
}

function send(message: Record<string, unknown>) {
  const target = iframe.value?.contentWindow
  if (!target || !activeSkinId.value) return
  try {
    const serialized = JSON.stringify(message)
    if (!serialized) return
    target.postMessage(JSON.parse(serialized), '*')
  } catch {
    // Ignore non-serializable context supplied by the host.
  }
}

function sendContext() {
  if (!ready.value) return
  const root = document.documentElement
  const styles = getComputedStyle(root)
  const variables: Record<string, string> = {}
  for (const name of [
    '--accent', '--bg', '--surface', '--surface-subtle', '--surface-container',
    '--text', '--text-muted', '--border', '--border-strong', '--success', '--warning',
    '--danger', '--shadow', '--radius', '--radius-lg', '--font-ui', '--detail-bg',
  ]) {
    const value = styles.getPropertyValue(name).trim()
    if (value) variables[name] = value
  }
  send({
    __sideweaveSkin: true,
    __agileTilesSkin: true,
    skin_id: activeSkinId.value,
    type: 'context',
    context: {
      ...props.context,
      skinId: activeSkinId.value,
      surface: props.surface,
      theme: {
        mode: root.dataset.theme ?? '',
        skin: root.dataset.skin ?? '',
        colorScheme: styles.colorScheme || 'dark',
        variables
      }
    }
  })
}

function sendEvent(name: string, payload: unknown) {
  if (!ready.value) return
  send({
    __sideweaveSkin: true,
    __agileTilesSkin: true,
    skin_id: activeSkinId.value,
    type: 'event',
    name,
    payload
  })
}

function sidebarHoverEnabled(): boolean {
  const settings = props.context.settings as { general?: { enable_mouse_hover?: boolean } } | undefined
  return settings?.general?.enable_mouse_hover !== false
}

function onSidebarEnter() {
  if (props.surface !== 'sidebar' || !sidebarHoverEnabled()) return
  window.clearTimeout(hoverLeaveTimer)
  if (!pendingHoverEnter) {
    pendingHoverEnter = sidebarHoverEnter()
      .then((token) => {
        hoverToken = token
        return token
      })
      .catch(() => 0)
      .finally(() => { pendingHoverEnter = null })
  }
}

function onSidebarLeave() {
  if (props.surface !== 'sidebar' || !sidebarHoverEnabled()) return
  window.clearTimeout(hoverLeaveTimer)
  hoverLeaveTimer = window.setTimeout(() => {
    const pending = pendingHoverEnter
    void (async () => {
      const token = pending ? await pending : hoverToken
      if (token) await sidebarCollapseIfUnhovered(token).catch(() => {})
    })()
  }, 150)
}

function onSurfaceEnter() {
  onSidebarEnter()
  if (props.surface === 'preview') void previewWindowEnter().catch(() => {})
}

function onSurfaceLeave() {
  onSidebarLeave()
  if (props.surface === 'preview') void previewWindowLeave().catch(() => {})
}

function messageAllowed(message: SkinRequest): boolean {
  let serialized: string
  try {
    serialized = JSON.stringify(message)
  } catch {
    return false
  }
  if (!serialized || new TextEncoder().encode(serialized).byteLength > MAX_MESSAGE_BYTES) return false
  const now = Date.now()
  if (now - messageStartedAt >= 1000) {
    messageStartedAt = now
    messageCount = 0
  }
  messageCount += 1
  return messageCount <= MAX_MESSAGES_PER_SECOND
}

function stringValue(payload: Record<string, unknown>, key: string): string {
  return typeof payload[key] === 'string' ? payload[key] as string : ''
}

async function dispatch(action: string, payload: Record<string, unknown>): Promise<unknown> {
  switch (action) {
    case 'state.get':
      return getState().then((state) => props.surface === 'sidebar'
        ? { ...state, plugins: state.plugins.filter((plugin) => plugin.enabled && plugin.sidebar_visible) }
        : state)
    case 'settings.set':
      return setSetting(stringValue(payload, 'category'), stringValue(payload, 'key'), payload.value)
    case 'plugins.setEnabled':
      return setPluginEnabled(stringValue(payload, 'pluginId'), payload.enabled === true)
    case 'plugins.setSidebarVisible':
      return setPluginSidebarVisible(stringValue(payload, 'pluginId'), payload.visible === true)
    case 'plugins.setOrder':
      if (!Array.isArray(payload.order) || !payload.order.every((id) => typeof id === 'string')) {
        throw new Error('order must be an array of plugin ids')
      }
      return setPluginOrder(payload.order as string[])
    case 'plugins.list':
      return getState().then((state) => props.surface === 'sidebar'
        ? state.plugins.filter((plugin) => plugin.enabled && plugin.sidebar_visible)
        : state.plugins)
    case 'plugins.call':
      return pluginCall(
        stringValue(payload, 'pluginId'),
        stringValue(payload, 'method'),
        payload.payload ?? {},
        typeof payload.timeoutMs === 'number' ? payload.timeoutMs : undefined
      )
    case 'navigation.openDetail': return openDetail(stringValue(payload, 'page'))
    case 'navigation.openSettings': return openDetail('settings')
    case 'navigation.openPlugin': return openPluginDetail(stringValue(payload, 'pluginId'))
    case 'navigation.openControlCenter': return openControlCenter()
    case 'host.showNativeUi':
      window.setTimeout(() => emit('showNativeUi'), 0)
      return true
    case 'window.closeDetail': return closeDetail()
    case 'window.minimizeDetail': return currentWindow.minimize()
    case 'window.toggleSidebar': return toggleSidebar()
    case 'window.startDragging': return currentWindow.startDragging()
    case 'window.startResizeDragging': {
      const directions = ['North', 'South', 'East', 'West', 'NorthEast', 'NorthWest', 'SouthEast', 'SouthWest']
      const direction = stringValue(payload, 'direction')
      if (!directions.includes(direction)) throw new Error('Invalid resize direction')
      return currentWindow.startResizeDragging(direction as
        | 'North' | 'South' | 'East' | 'West' | 'NorthEast' | 'NorthWest' | 'SouthEast' | 'SouthWest')
    }
    case 'sidebar.hoverEnter': return sidebarHoverEnter()
    case 'sidebar.collapseIfUnhovered':
      return sidebarCollapseIfUnhovered(typeof payload.token === 'number' ? payload.token : 0)
    case 'sidebar.previewEnter':
      return previewSidebarEnter(stringValue(payload, 'pluginId'), payload.rect as never)
    case 'sidebar.previewLeave': return previewSidebarLeave(stringValue(payload, 'pluginId'))
    case 'sidebar.previewHide': return previewHideNow()
    case 'preview.getCurrent': return getCurrentPreview()
    case 'preview.refresh': return refreshPreview()
    case 'market.refresh': return marketRefresh()
    case 'market.installed': return marketInstalled()
    case 'market.install': {
      const kind = payload.kind === 'skin' ? 'skin' : 'plugin'
      const id = stringValue(payload, 'id')
      const result = await marketInstall(kind, id)
      if (kind === 'skin' && props.skinId === `user:${id}`) void loadSurface()
      return result
    }
    case 'market.uninstall': {
      const kind = payload.kind === 'skin' ? 'skin' : 'plugin'
      const id = stringValue(payload, 'id')
      if (kind === 'skin' && props.skinId === `user:${id}`) {
        await applySkinWithRecommendedOpacity('default', [])
      }
      return marketUninstall(kind, id)
    }
    case 'market.importLocal':
      return pluginImportLocal(payload.kind === 'skin' ? 'skin' : 'plugin')
    case 'system.getInfo': return invoke('get_system_info')
    case 'performance.getStatus': return invoke('get_performance_status')
    case 'update.check': return updateCheck()
    case 'update.install': return updateInstall()
    case 'update.ignore': return updateIgnoreSet(stringValue(payload, 'version'))
    case 'backup.export': return backupExport()
    case 'backup.import': return backupImport()
    case 'app.openDataDir': return openDataDir()
    case 'app.openExternal': return openExternalUrl(stringValue(payload, 'url'))
    case 'app.restart': return restartApp()
    default: throw new Error(`Unsupported skin UI action: ${action}`)
  }
}

async function onMessage(event: MessageEvent<SkinRequest>) {
  const message = event.data
  if (
    event.source !== iframe.value?.contentWindow ||
    !message ||
    (message.__sideweaveSkin !== true && message.__agileTilesSkin !== true) ||
    message.skin_id !== activeSkinId.value
  ) return

  if (message.type === 'ready') {
    ready.value = true
    emit('surfaceReady', true)
    sendContext()
    return
  }
  if (message.type !== 'request' || (typeof message.id !== 'number' && typeof message.id !== 'string')) return
  if (!messageAllowed(message)) return
  const requestFrame = iframe.value
  const requestSkinId = activeSkinId.value

  try {
    const result = await dispatch(String(message.action ?? ''), message.payload ?? {})
    if (iframe.value !== requestFrame || activeSkinId.value !== requestSkinId) return
    const reply: SkinReply = {
      __sideweaveSkin: true,
      __agileTilesSkin: true,
      skin_id: activeSkinId.value,
      type: 'reply',
      id: message.id,
      result
    }
    send(reply as unknown as Record<string, unknown>)
  } catch (error) {
    if (iframe.value !== requestFrame || activeSkinId.value !== requestSkinId) return
    const reply: SkinReply = {
      __sideweaveSkin: true,
      __agileTilesSkin: true,
      skin_id: activeSkinId.value,
      type: 'reply',
      id: message.id,
      error: error instanceof Error ? error.message : String(error)
    }
    send(reply as unknown as Record<string, unknown>)
  }
}

function subscribe<T>(topic: string, eventName: string) {
  void listen<T>(topic, (event) => sendEvent(eventName, event.payload)).then((stop) => {
    if (disposed) stop()
    else unlisteners.push(stop)
  }).catch(() => {})
}

watch(() => [props.skinId, props.surface, props.enabled] as const, () => void loadSurface(), { immediate: true })
watch(() => props.context, sendContext, { deep: true })
onMounted(() => {
  window.addEventListener('message', onMessage)
  themeObserver = new MutationObserver(() => {
    sendContext()
    if (ready.value) {
      const context = {
        mode: document.documentElement.dataset.theme ?? '',
        skin: document.documentElement.dataset.skin ?? '',
        colorScheme: getComputedStyle(document.documentElement).colorScheme || 'dark'
      }
      sendEvent('themeChanged', context)
    }
  })
  themeObserver.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['data-theme', 'data-skin', 'style']
  })
  themeObserver.observe(document.head, {
    childList: true,
    subtree: true,
    characterData: true
  })
  subscribe('settings://changed', 'settingsChanged')
  subscribe<boolean>('sidebar://expanded', 'sidebarExpanded')
  subscribe<string>('main://surface', 'mainSurface')
})
onUnmounted(() => {
  disposed = true
  loadToken += 1
  window.clearTimeout(hoverLeaveTimer)
  window.removeEventListener('message', onMessage)
  themeObserver?.disconnect()
  themeObserver = null
  for (const stop of unlisteners) stop()
  unlisteners.length = 0
})
</script>

<template>
  <iframe
    v-if="src && enabled"
    ref="iframe"
    class="skin-surface-frame"
    :src="src"
    :title="`${activeSkinId} ${surface} UI`"
    sandbox="allow-scripts allow-forms allow-modals"
    @mouseenter="onSurfaceEnter"
    @mouseleave="onSurfaceLeave"
  />
</template>

<style>
.skin-surface-frame {
  position: fixed;
  inset: 0;
  z-index: 2147483000;
  display: block;
  width: 100vw;
  height: 100vh;
  margin: 0;
  padding: 0;
  border: 0;
  background: transparent;
}
</style>
