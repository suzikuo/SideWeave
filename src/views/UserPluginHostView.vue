<script setup lang="ts">
/** 插件详情页宿主：读取 manifest UI 入口 → 静态资源协议渲染 + 受限桥接。
 *  文件名暂时保留以兼容现有引用；新插件只依赖 manifest 和 Host API。
 *  安全边界：Web 插件保持 opaque-origin sandbox；可信本地插件仅在受信运行载体中允许 same-origin 资源加载，宿主能力仍通过 window.agileTiles 使用。 */
import { computed, onMounted, onUnmounted, reactive, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import Icon from '../ui/Icon.vue'
import { closeDetail } from '../lib/api'
import { pluginCall } from '../lib/pluginApi'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { pluginRegistryList, type RuntimePluginInfo } from '../lib/marketApi'
import { settingsRef } from '../lib/store'

const props = defineProps<{ pluginId: string; active: boolean }>()

interface PluginSlot {
  pluginId: string
  src: string
  allowSameOrigin: boolean
  ready: boolean
  lifecycleActive: boolean | null
}

interface PluginEventPayload {
  plugin_id: string
  topic: string
  payload?: unknown
}

const pluginSlots = reactive<PluginSlot[]>([])
const activePluginId = ref<string>('')
const frameRefs = new Map<string, HTMLIFrameElement>()

const loading = ref(false)
const error = ref('')

const MAX_MESSAGE_BYTES = 256 * 1024
const MAX_MESSAGES_PER_SECOND = 120
const THEME_VARIABLES = [
  '--accent',
  '--bg',
  '--surface',
  '--surface-subtle',
  '--surface-container',
  '--text',
  '--text-muted',
  '--border',
  '--border-strong',
  '--success',
  '--warning',
  '--danger',
  '--shadow',
  '--radius',
  '--radius-lg',
  '--font-ui'
]
let messageWindowStartedAt = 0
let messageWindowCount = 0
let unlistenPluginEvent: UnlistenFn | null = null
let themeObserver: MutationObserver | null = null
let frameLoadTimer: number | null = null
const FRAME_LOAD_TIMEOUT_MS = 10_000

function clearFrameLoadTimer() {
  if (frameLoadTimer !== null) {
    window.clearTimeout(frameLoadTimer)
    frameLoadTimer = null
  }
}

function pluginResourceUrl(pluginId: string, resource: string) {
  const encodedResource = resource.split('/').filter(Boolean).map(encodeURIComponent).join('/')
  return `http://agile-plugin.localhost/${encodeURIComponent(pluginId)}/${encodedResource}`
}

function notifyDetailContentReady(id: string) {
  void invoke('detail_content_ready', { pluginId: id }).catch(() => {})
}

function setFrameRef(id: string, element: unknown) {
  if (element instanceof HTMLIFrameElement) {
    frameRefs.set(id, element)
  } else {
    frameRefs.delete(id)
  }
}

function syncPluginLifecycles() {
  for (const slot of pluginSlots) {
    if (!slot.ready) continue
    const active = props.active && slot.pluginId === activePluginId.value
    if (slot.lifecycleActive === active) continue
    const target = frameRefs.get(slot.pluginId)?.contentWindow
    if (!target) continue
    slot.lifecycleActive = active
    target.postMessage(
      {
        __agileTiles: true,
        type: 'lifecycle',
        plugin_id: slot.pluginId,
        active
      },
      '*'
    )
  }
}

function resolvePluginEntry(runtime: RuntimePluginInfo): { src: string; sameOrigin: boolean } {
  const id = runtime.manifest.id
  const sameOrigin =
    runtime.runtime !== 'web_sandbox' && runtime.effectiveTrust === 'trusted_local'

  if (!runtime.manifest.ui?.entry) {
    throw new Error(`当前插件没有详情页: ${runtime.runtime}`)
  }

  const entry = runtime.manifest.ui.entry
  if (entry.includes('..') || entry.startsWith('/') || entry.startsWith('\\')) {
    throw new Error('插件详情页入口无效')
  }

  if (
    runtime.runtime !== 'web_sandbox' &&
    runtime.source !== 'installed-external' &&
    runtime.source !== 'installed-dylib'
  ) {
    return { src: '/' + entry, sameOrigin }
  }

  const assetRoot = runtime.manifest.ui.assetRoot || '.'
  const resource =
    assetRoot === '.' || entry === assetRoot || entry.startsWith(`${assetRoot}/`)
      ? entry
      : `${assetRoot}/${entry}`

  if (resource.includes('..') || resource.startsWith('/') || resource.startsWith('\\')) {
    throw new Error('插件详情页入口无效')
  }

  return { src: pluginResourceUrl(id, resource), sameOrigin }
}

const inFlightLoads = new Map<string, Promise<void>>()

async function loadPlugin(id: string) {
  if (!id) return

  // 1. 若已经在插槽池中，直接复用（0ms 瞬间切换）
  const existing = pluginSlots.find((s) => s.pluginId === id)
  if (existing) {
    activePluginId.value = id
    syncPluginLifecycles()
    error.value = ''
    if (existing.ready) {
      clearFrameLoadTimer()
      loading.value = false
      syncFrameTheme()
      notifyDetailContentReady(id)
    } else {
      loading.value = true
    }
    return
  }

  // 2. 若已经在并发加载中，等待其完成
  if (inFlightLoads.has(id)) {
    activePluginId.value = id
    syncPluginLifecycles()
    error.value = ''
    loading.value = true
    await inFlightLoads.get(id)
    const slot = pluginSlots.find((s) => s.pluginId === id)
    if (slot?.ready && activePluginId.value === id) {
      clearFrameLoadTimer()
      loading.value = false
      syncFrameTheme()
      notifyDetailContentReady(id)
    }
    return
  }

  activePluginId.value = id
  syncPluginLifecycles()
  loading.value = true
  error.value = ''

  const loadPromise = (async () => {
    try {
      let registry = await pluginRegistryList(false)
      let runtime = registry.find((item) => item.manifest.id === id)
      if (!runtime) {
        registry = await pluginRegistryList(true)
        runtime = registry.find((item) => item.manifest.id === id)
      }
      if (!runtime) throw new Error(`插件未在运行时注册: ${id}`)

      const { src, sameOrigin } = resolvePluginEntry(runtime)
      pluginSlots.push({
        pluginId: id,
        src,
        allowSameOrigin: sameOrigin,
        ready: false,
        lifecycleActive: null
      })

      clearFrameLoadTimer()
      frameLoadTimer = window.setTimeout(() => {
        const slot = pluginSlots.find((s) => s.pluginId === id)
        if (slot && !slot.ready && activePluginId.value === id) {
          loading.value = false
          error.value = '插件详情页加载超时，请检查插件 UI 包或资源协议'
          notifyDetailContentReady(id)
        }
        frameLoadTimer = null
      }, FRAME_LOAD_TIMEOUT_MS)
    } catch (e) {
      if (activePluginId.value === id) {
        error.value = String(e)
        loading.value = false
        notifyDetailContentReady(id)
      }
    } finally {
      inFlightLoads.delete(id)
    }
  })()

  inFlightLoads.set(id, loadPromise)
  await loadPromise
}

function onFrameLoad(id: string) {
  const slot = pluginSlots.find((s) => s.pluginId === id)
  if (slot) {
    slot.ready = true
    slot.lifecycleActive = null
  }
  syncPluginLifecycles()
  if (activePluginId.value === id) {
    clearFrameLoadTimer()
    loading.value = false
    error.value = ''
    syncFrameTheme()
    notifyDetailContentReady(id)
  }
}

function onFrameError(id: string) {
  if (activePluginId.value === id) {
    clearFrameLoadTimer()
    loading.value = false
    error.value = '插件详情页加载失败'
    notifyDetailContentReady(id)
  }
}

onMounted(() => {
  void loadPlugin(props.pluginId)
})

watch(
  () => props.pluginId,
  (newId) => {
    if (newId) {
      void loadPlugin(newId)
    }
  }
)

watch(() => props.active, (active) => {
  syncPluginLifecycles()
  if (active) syncFrameTheme()
})

watch(
  () => [
    settingsRef.value?.appearance.skin,
    settingsRef.value?.appearance.detail_panel_opacity,
    settingsRef.value?.appearance.control_panel_opacity,
    settingsRef.value?.appearance.component_opacity,
    settingsRef.value?.appearance.glass_effect_enabled,
    settingsRef.value?.appearance.detail_image_opacity
  ],
  syncFrameTheme
)

// 桥接和主题同步
interface UopRequest {
  __agileTiles: true
  type?: string
  id: number | string
  plugin_id: string
  action: string
  payload?: {
    key?: unknown
    value?: unknown
    url?: unknown
    method?: unknown
    payload?: unknown
    timeout_ms?: unknown
  }
}

interface UopResponse {
  __agileTiles: true
  id: number | string
  plugin_id: string
  result?: unknown
  error?: string
}

function reply(source: MessageEventSource | null, message: UopResponse) {
  postMessageSafe(source as Window | null, message)
}

function themeMessage(id: string) {
  const root = document.documentElement
  const styles = getComputedStyle(root)
  const appearance = settingsRef.value?.appearance
  const variables: Record<string, string> = {}
  for (const name of THEME_VARIABLES) {
    const value = styles.getPropertyValue(name).trim()
    if (value) variables[name] = value
  }
  variables['--detail-panel-opacity'] = String(
    appearance?.skin === 'default'
      ? appearance.control_panel_opacity
      : appearance?.detail_panel_opacity ?? 1
  )
  variables['--component-opacity'] = String(appearance?.component_opacity ?? 1)
  variables['--component-glass-filter'] = appearance?.glass_effect_enabled ? 'blur(4px)' : 'none'
  variables['--detail-image-opacity'] = String(appearance?.detail_image_opacity ?? 1)
  return {
    __agileTiles: true,
    type: 'theme',
    plugin_id: id,
    theme: {
      variables,
      theme: root.dataset.theme ?? '',
      skin: root.dataset.skin ?? '',
      colorScheme: styles.colorScheme || 'dark'
    }
  }
}

function syncFrameTheme() {
  if (!props.active) return
  const id = activePluginId.value
  const frame = frameRefs.get(id)
  if (frame?.contentWindow) postMessageSafe(frame.contentWindow, themeMessage(id))
}

function cloneMessage<T>(value: T): T {
  try {
    const serialized = JSON.stringify(value)
    return (serialized === undefined ? null : JSON.parse(serialized)) as T
  } catch {
    throw new Error('插件消息不可序列化')
  }
}

function postMessageSafe(target: Window | null | undefined, message: unknown) {
  target?.postMessage(cloneMessage(message), '*')
}

function framePluginId(source: MessageEventSource | null): string | null {
  for (const [pluginId, frame] of frameRefs) {
    if (frame?.contentWindow === source) return pluginId
  }
  return null
}

function messageAllowed(data: UopRequest): boolean {
  const serialized = JSON.stringify(data)
  if (!serialized || new TextEncoder().encode(serialized).byteLength > MAX_MESSAGE_BYTES)
    return false
  const now = Date.now()
  if (now - messageWindowStartedAt >= 1000) {
    messageWindowStartedAt = now
    messageWindowCount = 0
  }
  messageWindowCount += 1
  return messageWindowCount <= MAX_MESSAGES_PER_SECOND
}

async function onMessage(e: MessageEvent) {
  const d = e.data as UopRequest | null
  if (!d || d.__agileTiles !== true) return
  const sourcePluginId = framePluginId(e.source)
  if (
    !sourcePluginId ||
    sourcePluginId !== d.plugin_id ||
    !props.active ||
    activePluginId.value !== sourcePluginId
  ) return
  const slot = pluginSlots.find((item) => item.pluginId === sourcePluginId)
  if (!slot || slot.lifecycleActive === false) return
  if (d.type === 'close-detail') {
    void closeDetail().catch(() => {})
    return
  }
  if ((typeof d.id !== 'number' && typeof d.id !== 'string') || !messageAllowed(d)) return
  const payload = d.payload ?? {}
  try {
    let result: unknown = null
    switch (d.action) {
      case 'call':
        result = await pluginCall(
          d.plugin_id,
          String(payload.method ?? ''),
          payload.payload ?? {},
          Number(payload.timeout_ms) || undefined
        )
        break
      default:
        throw new Error(`不支持的操作: ${String(d.action)}`)
    }
    reply(e.source, { __agileTiles: true, id: d.id, plugin_id: d.plugin_id, result })
  } catch (err) {
    reply(e.source, {
      __agileTiles: true,
      id: d.id,
      plugin_id: d.plugin_id,
      error: err instanceof Error ? err.message : String(err)
    })
  }
}

onMounted(() => {
  window.addEventListener('message', onMessage)
  themeObserver = new MutationObserver(syncFrameTheme)
  themeObserver.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['data-theme', 'data-skin', 'style']
  })
  void listen<PluginEventPayload>('plugin://event', (event) => {
    const payload = event.payload
    const slot = pluginSlots.find((item) => item.pluginId === payload.plugin_id)
    if (!slot?.lifecycleActive) return
    const target = frameRefs.get(payload.plugin_id)?.contentWindow
    if (!target) return
    postMessageSafe(target, {
      __agileTiles: true,
      type: 'event',
      plugin_id: payload.plugin_id,
      event: payload
    })
  }).then((stop) => {
    unlistenPluginEvent = stop
  })
})

onUnmounted(() => {
  clearFrameLoadTimer()
  pluginSlots.length = 0
  frameRefs.clear()
  window.removeEventListener('message', onMessage)
  themeObserver?.disconnect()
  themeObserver = null
  unlistenPluginEvent?.()
  unlistenPluginEvent = null
})

const isCurrentSlotReady = computed(() => {
  const slot = pluginSlots.find((s) => s.pluginId === activePluginId.value)
  return Boolean(slot?.ready)
})
</script>

<template>
  <div class="host">
    <div v-if="pluginSlots.length > 0" class="host-viewport">
      <iframe
        v-for="slot in pluginSlots"
        :key="slot.pluginId"
        :ref="(element) => setFrameRef(slot.pluginId, element)"
        class="host-frame"
        :class="{
          'frame-active': props.active && slot.pluginId === activePluginId,
          'frame-ready': slot.ready
        }"
        @load="onFrameLoad(slot.pluginId)"
        @error="onFrameError(slot.pluginId)"
        :sandbox="
          slot.allowSameOrigin
            ? 'allow-scripts allow-modals allow-forms allow-same-origin'
            : 'allow-scripts allow-modals allow-forms'
        "
        :src="slot.src"
        :title="slot.pluginId"
      />
      <div v-if="error" class="host-state">
        <Icon name="info" :size="22" />
        <p class="host-title">无法加载插件</p>
        <p class="host-desc">{{ error }}</p>
      </div>
      <div v-else-if="loading && !isCurrentSlotReady" class="host-state">
        <Icon name="refresh" :size="22" class="spin" />
        <p class="host-title">正在加载插件…</p>
      </div>
    </div>

    <div v-else-if="error" class="host-state">
      <Icon name="info" :size="22" />
      <p class="host-title">无法加载插件</p>
      <p class="host-desc">{{ error }}</p>
    </div>

    <div v-else-if="loading" class="host-state">
      <Icon name="refresh" :size="22" class="spin" />
      <p class="host-title">正在加载插件…</p>
    </div>
  </div>
</template>

<style scoped>
.host {
  display: flex;
  flex-direction: column;
  width: 100%;
  height: 100%;
  min-height: 0;
  flex: 1;
}

.host-viewport {
  position: relative;
  width: 100%;
  height: 100%;
  flex: 1;
  min-height: 0;
  overflow: hidden;
}

.host-viewport > .host-state {
  position: absolute;
  inset: 0;
  z-index: 3;
  background: var(--surface, #1e1e24);
}

.host-frame {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  border: none;
  background: transparent;
  visibility: hidden;
  opacity: 0;
  pointer-events: none;
  z-index: 1;
  transition: opacity 0.1s ease;
}

.host-frame:not(.frame-active) {
  display: none;
}

.host-frame.frame-active {
  z-index: 2;
}

.host-frame.frame-active.frame-ready {
  visibility: visible;
  opacity: 1;
  pointer-events: auto;
}

.host-state {
  flex: 1;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 6px;
  text-align: center;
  color: var(--text-muted);
}
.host-title {
  margin: 4px 0 0;
  font-size: 13.5px;
  font-weight: 600;
  color: var(--text);
}
.host-desc {
  margin: 0;
  max-width: 420px;
  font-size: 12px;
  word-break: break-all;
}

.spin {
  animation: spin 0.9s linear infinite;
}
@keyframes spin {
  to {
    transform: rotate(360deg);
  }
}
</style>
