<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { nextTick, onMounted, onUnmounted, ref, watch, type CSSProperties } from 'vue'
import Icon from '../ui/Icon.vue'

type Kind = 'info' | 'success' | 'warning' | 'alarm'
type TailSide = 'left' | 'right' | 'top'
interface DesktopNotification {
  id: string
  title: string
  body: string
  kind: Kind
  sourcePluginId?: string | null
  createdAtMs: number
  expiresAtMs: number
}
interface NotificationViewSnapshot {
  notifications: DesktopNotification[]
  tailSide: TailSide
}

const isSidebar = window.location.pathname.endsWith('/sidebar-notifications.html')
const notifications = ref<DesktopNotification[]>([])
const pluginIcons = ref<Record<string, string>>({})
const tailSide = ref<TailSide>('left')
const suppressBatchMotion = ref(false)
const stackElement = ref<HTMLElement | null>(null)
const lifetimeElapsedMs = new Map<string, number>()
let unlisten: UnlistenFn | undefined
let skinObserver: MutationObserver | undefined
let disposed = false
let resizeFrame = 0
let snapshotFrame = 0
let snapshotEventVersion = 0
let pendingSnapshot: NotificationViewSnapshot | undefined
let lastRequestedHeight = 0
let motionResetTimer: number | undefined
const pluginIconRequests = new Set<string>()

function loadPluginIcons(items: DesktopNotification[]) {
  if (!isSidebar) return
  const pluginIds = new Set(items.map((item) => item.sourcePluginId).filter((id): id is string => Boolean(id)))
  for (const pluginId of pluginIds) {
    if (pluginIcons.value[pluginId] || pluginIconRequests.has(pluginId)) continue
    pluginIconRequests.add(pluginId)
    void invoke<string>('host_notification_plugin_icon', { pluginId })
      .then((icon) => { pluginIcons.value[pluginId] = icon || 'puzzle' })
      .catch(() => { pluginIcons.value[pluginId] = 'puzzle' })
      .finally(() => { pluginIconRequests.delete(pluginId) })
  }
}

function applySnapshot(snapshot: NotificationViewSnapshot) {
  const activeIds = new Set(snapshot.notifications.map((item) => item.id))
  const previousIds = new Set(notifications.value.map((item) => item.id))
  const addedOrRemovedCount =
    snapshot.notifications.filter((item) => !previousIds.has(item.id)).length +
    notifications.value.filter((item) => !activeIds.has(item.id)).length
  if (addedOrRemovedCount > 1) {
    suppressBatchMotion.value = true
    if (motionResetTimer !== undefined) window.clearTimeout(motionResetTimer)
    motionResetTimer = window.setTimeout(() => {
      suppressBatchMotion.value = false
      motionResetTimer = undefined
    }, 220)
  }
  for (const id of lifetimeElapsedMs.keys()) {
    if (!activeIds.has(id)) lifetimeElapsedMs.delete(id)
  }
  for (const item of snapshot.notifications) {
    if (!lifetimeElapsedMs.has(item.id)) {
      lifetimeElapsedMs.set(item.id, Math.max(0, Date.now() - item.createdAtMs))
    }
  }
  notifications.value = snapshot.notifications
  loadPluginIcons(snapshot.notifications)
  tailSide.value = snapshot.tailSide
}

function lifetimeStyle(item: DesktopNotification): CSSProperties {
  const durationMs = Math.max(1, item.expiresAtMs - item.createdAtMs)
  const elapsedMs = Math.min(durationMs, lifetimeElapsedMs.get(item.id) ?? 0)
  return {
    '--lifetime-duration': `${durationMs}ms`,
    '--lifetime-delay': `-${elapsedMs}ms`,
  } as CSSProperties
}

function scheduleSnapshot(snapshot: NotificationViewSnapshot) {
  pendingSnapshot = snapshot
  if (snapshotFrame) return
  snapshotFrame = requestAnimationFrame(() => {
    snapshotFrame = 0
    const latest = pendingSnapshot
    pendingSnapshot = undefined
    if (latest && !disposed) applySnapshot(latest)
  })
}

async function resizeWindow() {
  await nextTick()
  if (!stackElement.value || disposed) return
  const height = Math.min(isSidebar ? 420 : 640, Math.max(80, stackElement.value.scrollHeight + 20))
  if (Math.abs(height - lastRequestedHeight) < 1) return
  try {
    await invoke('host_notification_resize', { height })
    lastRequestedHeight = height
  } catch (error) {
    console.error('[host-notifications] failed to resize notification view', error)
  }
}

function scheduleResizeWindow() {
  if (disposed || resizeFrame) return
  resizeFrame = requestAnimationFrame(() => {
    resizeFrame = 0
    void resizeWindow()
  })
}

watch(notifications, scheduleResizeWindow, { deep: true, flush: 'post' })

async function dismiss(id: string) {
  try {
    await invoke('host_notification_dismiss', { id })
  } catch (error) {
    console.error('[host-notifications] failed to dismiss notification', error)
  }
}

onMounted(async () => {
  skinObserver = new MutationObserver(scheduleResizeWindow)
  skinObserver.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['data-theme', 'data-skin', 'style']
  })
  skinObserver.observe(document.head, { childList: true, subtree: true, characterData: true })

  try {
    unlisten = await listen<NotificationViewSnapshot>(
      'host-notification://changed',
      (event) => {
        snapshotEventVersion += 1
        scheduleSnapshot(event.payload)
      }
    )
    if (disposed) {
      unlisten()
      return
    }
    const eventVersionAtRequest = snapshotEventVersion
    const snapshot = await invoke<NotificationViewSnapshot>('host_notification_snapshot')
    if (eventVersionAtRequest === snapshotEventVersion) scheduleSnapshot(snapshot)
  } catch (error) {
    console.error('[host-notifications] failed to initialize notification view', error)
  }
})

onUnmounted(() => {
  disposed = true
  if (resizeFrame) cancelAnimationFrame(resizeFrame)
  if (snapshotFrame) cancelAnimationFrame(snapshotFrame)
  pendingSnapshot = undefined
  if (motionResetTimer !== undefined) window.clearTimeout(motionResetTimer)
  unlisten?.()
  skinObserver?.disconnect()
})
</script>

<template>
  <main
    class="notification-surface"
    :class="[
      isSidebar ? 'sidebar-surface' : 'desktop-surface',
      isSidebar ? `tail-${tailSide}` : ''
    ]"
    aria-live="polite"
    @contextmenu.prevent
    :aria-label="isSidebar ? '侧边栏消息通知' : '桌面通知'"
  >
    <div ref="stackElement" class="notification-stack">
      <TransitionGroup
        name="toast"
        tag="div"
        class="notification-list"
        :class="{ 'batch-update': suppressBatchMotion }"
      >
        <article
          v-for="item in notifications"
          :key="item.id"
          class="notification"
          :class="[item.kind, { 'sidebar-bubble': isSidebar }]"
          role="status"
          @contextmenu.prevent.stop="dismiss(item.id)"
        >
          <div v-if="isSidebar" class="plugin-mark" aria-hidden="true">
            <Icon :name="pluginIcons[item.sourcePluginId || ''] || 'puzzle'" :size="21" :stroke-width="1.8" />
          </div>
          <div v-else class="mark" aria-hidden="true">
            {{ item.kind === 'success' ? '✓' : item.kind === 'warning' ? '!' : item.kind === 'alarm' ? '◷' : 'i' }}
          </div>
          <div class="copy">
            <strong v-if="!isSidebar && item.title">{{ item.title }}</strong>
            <p v-if="item.body || (isSidebar && item.title)">{{ isSidebar && !item.body ? item.title : item.body }}</p>
          </div>
          <div class="lifetime" :style="lifetimeStyle(item)" />
        </article>
      </TransitionGroup>
    </div>
  </main>
</template>

<style>
.notification-surface { width: 100%; height: 100%; padding: 10px; overflow: hidden; pointer-events: none; }
.notification-stack { max-height: 100%; overflow-y: auto; padding: 2px 5px 8px 2px; pointer-events: none; scrollbar-width: none; }
.notification-stack::-webkit-scrollbar { display: none; }
.notification-list { display: grid; gap: 10px; pointer-events: none; }
.notification { --tone: var(--notification-info, #80a7ff); position: relative; display: grid; grid-template-columns: 34px minmax(0, 1fr); align-items: start; gap: 11px; overflow: hidden; padding: 15px 13px 17px; border: 1px solid color-mix(in srgb, var(--tone) 42%, var(--notification-desktop-border-base, #474951)); border-left: 3px solid var(--tone); border-radius: var(--notification-desktop-radius, 13px); background: transparent; color: var(--notification-custom-desktop-color, var(--notification-desktop-color, #f3f4f7)); box-shadow: var(--notification-desktop-shadow, 0 12px 34px rgba(0, 0, 0, .32)); pointer-events: auto; backdrop-filter: var(--notification-desktop-backdrop-filter, none); }
.notification::before { position: absolute; z-index: 0; inset: 1px 1px 1px 3px; border-radius: inherit; background: var(--notification-custom-desktop-background, var(--notification-desktop-background, linear-gradient(145deg, rgba(35, 37, 44, .98), rgba(25, 26, 31, .98)))); content: ''; opacity: var(--notification-desktop-background-opacity, var(--notification-background-opacity, .92)); pointer-events: none; }
.notification.success { --tone: var(--notification-success, #54d6b3); }.notification.warning { --tone: var(--notification-warning, #efbd65); }.notification.alarm { --tone: var(--notification-alarm, #ff7d83); }
.mark { display: grid; width: 32px; height: 32px; place-items: center; border-radius: 10px; background: color-mix(in srgb, var(--tone) 16%, transparent); color: var(--tone); font-size: 15px; font-weight: 750; }
.mark, .copy { position: relative; z-index: 1; }.copy { min-width: 0; padding-top: 1px; }.copy strong { display: block; margin-bottom: 4px; font-size: 14px; font-weight: 650; line-height: 1.35; overflow-wrap: anywhere; }.copy p { margin: 0; color: var(--notification-custom-desktop-body-color, #c8cad1); font-size: 12.5px; line-height: 1.48; white-space: pre-wrap; overflow-wrap: anywhere; }
.lifetime { position: absolute; z-index: 2; right: 0; bottom: 0; left: 0; height: 2px; background: var(--tone); transform: scaleX(1); transform-origin: left center; animation: notification-lifetime var(--lifetime-duration, 5s) linear var(--lifetime-delay, 0ms) both; }
@keyframes notification-lifetime { to { transform: scaleX(0); } }
.toast-enter-active, .toast-leave-active { transition: opacity .18s ease, transform .18s ease; }.toast-enter-from, .toast-leave-to { opacity: 0; transform: translateX(16px); }.toast-leave-active { position: absolute; right: 5px; left: 2px; }
.notification-list.batch-update .toast-enter-active, .notification-list.batch-update .toast-leave-active { transition: none; }

.sidebar-surface { padding: 12px; }
.sidebar-surface .notification-stack { overflow-x: hidden; padding: 5px 10px 8px; }
.sidebar-surface .notification-list { gap: 8px; }
.notification.sidebar-bubble { --tone: var(--notification-bubble-info, #3c76cb); --notification-bubble-fill: color-mix(in srgb, var(--notification-custom-bubble-background, var(--notification-bubble-background, #fffdf7)) var(--notification-bubble-background-strength, var(--notification-background-strength, 92%)), transparent); isolation: isolate; grid-template-columns: 23px minmax(0, 1fr); align-items: center; gap: 8px; overflow: visible; margin: 2px; padding: 6px 11px 9px; border: 2px solid var(--notification-bubble-border, #1c1c1c); border-radius: var(--notification-bubble-radius, 16px); background: var(--notification-bubble-fill); color: var(--notification-custom-bubble-color, var(--notification-bubble-color, #242424)); box-shadow: var(--notification-bubble-shadow, 0 5px 14px rgba(0, 0, 0, .2)); backdrop-filter: none; }
.notification.sidebar-bubble::before { background: none; opacity: 1; }
.notification.sidebar-bubble.success { --tone: var(--notification-bubble-success, #23816a); }.notification.sidebar-bubble.warning { --tone: var(--notification-bubble-warning, #a96a0d); }.notification.sidebar-bubble.alarm { --tone: var(--notification-bubble-alarm, #c3444b); }
.sidebar-bubble .copy { position: relative; z-index: 1; }
.sidebar-bubble .copy { display: block; min-width: 0; padding-top: 0; }
.sidebar-bubble .copy strong { display: none; }
.sidebar-bubble .copy p { display: -webkit-box; min-width: 0; overflow: hidden; color: var(--notification-custom-bubble-body-color, var(--notification-bubble-body, #444)); font-size: 12px; line-height: 1.35; white-space: pre-wrap; overflow-wrap: anywhere; -webkit-box-orient: vertical; -webkit-line-clamp: 2; }
.plugin-mark { position: relative; z-index: 1; display: grid; width: 23px; height: 23px; place-items: center; overflow: hidden; border-radius: 6px; color: var(--tone); }
.sidebar-bubble .lifetime { z-index: 2; right: 12px; bottom: 4px; left: 12px; height: 2px; border-radius: 2px; }
.sidebar-bubble::before, .sidebar-bubble::after { position: absolute; z-index: 0; inset: auto; width: 0; height: 0; content: ''; }
.tail-left .sidebar-bubble::before { top: 50%; left: -11px; border-top: 7px solid transparent; border-right: 11px solid var(--notification-bubble-border, #1c1c1c); border-bottom: 7px solid transparent; transform: translateY(-50%); }
.tail-left .sidebar-bubble::after { top: 50%; left: -8px; border-top: 5px solid transparent; border-right: 9px solid var(--notification-bubble-fill); border-bottom: 5px solid transparent; transform: translateY(-50%); }
.tail-right .sidebar-bubble::before { top: 50%; right: -11px; border-top: 7px solid transparent; border-bottom: 7px solid transparent; border-left: 11px solid var(--notification-bubble-border, #1c1c1c); transform: translateY(-50%); }
.tail-right .sidebar-bubble::after { top: 50%; right: -8px; border-top: 5px solid transparent; border-bottom: 5px solid transparent; border-left: 9px solid var(--notification-bubble-fill); transform: translateY(-50%); }
.tail-top .sidebar-bubble::before { top: -11px; left: 50%; border-right: 7px solid transparent; border-bottom: 11px solid var(--notification-bubble-border, #1c1c1c); border-left: 7px solid transparent; transform: translateX(-50%); }
.tail-top .sidebar-bubble::after { top: -8px; left: 50%; border-right: 5px solid transparent; border-bottom: 9px solid var(--notification-bubble-fill); border-left: 5px solid transparent; transform: translateX(-50%); }
.tail-top .notification.sidebar-bubble { padding-top: 11px; }
.tail-left .toast-enter-from, .tail-left .toast-leave-to { transform: translateX(10px); }
.tail-right .toast-enter-from, .tail-right .toast-leave-to { transform: translateX(-10px); }
.tail-top .toast-enter-from, .tail-top .toast-leave-to { transform: translateY(10px); }
@media (prefers-reduced-motion: reduce) { .toast-enter-active, .toast-leave-active { transition: none; } }
</style>
