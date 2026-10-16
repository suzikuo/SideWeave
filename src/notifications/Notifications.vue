<script setup lang="ts">
import { invoke } from '@tauri-apps/api/core'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import { nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
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
  notifications.value = snapshot.notifications
  loadPluginIcons(snapshot.notifications)
  tailSide.value = snapshot.tailSide
}

function notificationIcon(kind: Kind): string {
  if (kind === 'success') return 'check'
  if (kind === 'warning') return 'triangle-alert'
  if (kind === 'alarm') return 'bell'
  return 'info'
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
            <Icon :name="pluginIcons[item.sourcePluginId || ''] || 'puzzle'" :size="16" :stroke-width="1.8" />
          </div>
          <div v-else class="mark" :class="`tone-${item.kind}`" aria-hidden="true">
            <Icon :name="notificationIcon(item.kind)" :size="15" :stroke-width="2" />
          </div>
          <div class="copy">
            <strong v-if="!isSidebar && item.title">{{ item.title }}</strong>
            <p v-if="item.body || (isSidebar && item.title)">{{ isSidebar && !item.body ? item.title : item.body }}</p>
          </div>
          <button
            v-if="!isSidebar"
            class="toast-close"
            type="button"
            aria-label="关闭通知"
            @click.stop="dismiss(item.id)"
          >
            <Icon name="x" :size="14" :stroke-width="1.8" />
          </button>
        </article>
      </TransitionGroup>
    </div>
  </main>
</template>

<style>
.notification-surface { width: 100%; height: 100%; padding: 10px; overflow: hidden; pointer-events: none; }
.notification-stack { max-height: 100%; overflow-y: auto; padding: 2px 5px 8px 2px; pointer-events: none; scrollbar-width: none; }
.notification-stack::-webkit-scrollbar { display: none; }
.notification-list { display: grid; gap: 9px; pointer-events: none; }
.notification {
  --tone: var(--notification-info, #4d80ff);
  position: relative;
  display: grid;
  grid-template-columns: 32px minmax(0, 1fr) 24px;
  align-items: center;
  gap: 12px;
  overflow: hidden;
  padding: 12px 13px;
  border: 1px solid color-mix(in srgb, var(--notification-desktop-border-base, var(--border, #292a2e)) 92%, transparent);
  border-radius: var(--notification-desktop-radius, 10px);
  background: transparent;
  color: var(--notification-custom-desktop-color, var(--notification-desktop-color, var(--card-foreground, #fafafa)));
  box-shadow: var(--notification-desktop-shadow, 0 1px 2px rgb(0 0 0 / 24%), 0 8px 24px rgb(0 0 0 / 20%));
  pointer-events: auto;
  backdrop-filter: var(--notification-desktop-backdrop-filter, blur(12px) saturate(125%));
  -webkit-backdrop-filter: var(--notification-desktop-backdrop-filter, blur(12px) saturate(125%));
}
.notification::before {
  position: absolute;
  z-index: 0;
  inset: 1px;
  border-radius: inherit;
  background: var(--notification-custom-desktop-background, var(--notification-desktop-background, var(--card, #18181b)));
  content: '';
  opacity: var(--notification-desktop-background-opacity, var(--notification-background-opacity, .94));
  pointer-events: none;
}
.notification.success { --tone: var(--notification-success, #22c55e); }
.notification.warning { --tone: var(--notification-warning, #eab308); }
.notification.alarm { --tone: var(--notification-alarm, #ef4444); }
.mark, .copy, .toast-close { position: relative; z-index: 1; }
.mark {
  display: grid;
  width: 32px;
  height: 32px;
  place-items: center;
  border: 1px solid color-mix(in srgb, var(--border, #292a2e) 72%, var(--tone) 28%);
  border-radius: 9px;
  background: color-mix(in srgb, var(--muted, #18191b) 82%, var(--tone) 18%);
  color: var(--tone);
  font-size: 0;
  line-height: 0;
}
.mark.tone-info { --tone: var(--notification-info, #4d80ff); }
.mark.tone-success { --tone: var(--notification-success, #22c55e); }
.mark.tone-warning { --tone: var(--notification-warning, #eab308); }
.mark.tone-alarm { --tone: var(--notification-alarm, #ef4444); }
.copy { min-width: 0; }
.copy strong { display: block; margin-bottom: 3px; font-size: 13px; font-weight: 600; line-height: 1.35; letter-spacing: -.01em; overflow-wrap: anywhere; }
.copy p { margin: 0; color: var(--notification-custom-desktop-body-color, var(--notification-desktop-body-color, var(--muted-foreground, #a1a1aa))); font-size: 12px; line-height: 1.45; white-space: pre-wrap; overflow-wrap: anywhere; }
.toast-close {
  display: grid;
  width: 24px;
  height: 24px;
  place-items: center;
  padding: 0;
  border: 0;
  border-radius: 6px;
  background: transparent;
  color: var(--muted-foreground, #a1a1aa);
  cursor: pointer;
  opacity: .62;
  transition: background-color .15s ease, color .15s ease, opacity .15s ease;
}
.toast-close:hover, .toast-close:focus-visible { background: var(--muted, #18191b); color: var(--foreground, #fafafa); opacity: 1; }
.toast-close:focus-visible { outline: 2px solid var(--ring, var(--accent, #4d80ff)); outline-offset: 2px; }
.toast-enter-active, .toast-leave-active { transition: opacity .18s ease, transform .18s ease; }
.toast-enter-from, .toast-leave-to { opacity: 0; transform: translateX(16px); }
.toast-leave-active { position: absolute; right: 5px; left: 2px; }
.notification-list.batch-update .toast-enter-active,
.notification-list.batch-update .toast-leave-active { transition: none; }

.sidebar-surface { padding: 4px; }
.sidebar-surface .notification-stack { overflow-x: hidden; padding: 3px 8px 6px; }
.sidebar-surface .notification-list { gap: 8px; }
.notification.sidebar-bubble {
  --tone: var(--notification-bubble-info, var(--accent, #4d80ff));
  --notification-bubble-fill: var(--notification-custom-bubble-background-fill, color-mix(in srgb, var(--notification-custom-bubble-background, var(--notification-bubble-background, #fff)) var(--notification-bubble-background-strength, var(--notification-background-strength, 90%)), transparent));
  isolation: isolate;
  grid-template-columns: 26px minmax(0, 1fr);
  align-items: center;
  gap: 8px;
  overflow: visible;
  margin: 1px;
  padding: 9px 11px;
  border: 1px solid color-mix(in srgb, var(--notification-bubble-border, #d4d4d8) 88%, transparent);
  border-radius: var(--notification-bubble-radius, 14px);
  background: var(--notification-bubble-fill);
  color: var(--notification-custom-bubble-color, var(--notification-bubble-color, #18181b));
  box-shadow: var(--notification-bubble-shadow, 0 8px 22px rgb(0 0 0 / 16%));
  backdrop-filter: blur(12px) saturate(130%);
  -webkit-backdrop-filter: blur(12px) saturate(130%);
}
.notification.sidebar-bubble::before,
.notification.sidebar-bubble::after { content: none; }
.sidebar-bubble .toast-close { width: 22px; height: 22px; color: var(--notification-custom-bubble-body-color, var(--notification-bubble-body, #52525b)); opacity: .65; }
.sidebar-bubble .toast-close:hover, .sidebar-bubble .toast-close:focus-visible { background: color-mix(in srgb, var(--notification-bubble-border, #d4d4d8) 40%, transparent); color: var(--notification-custom-bubble-color, var(--notification-bubble-color, #18181b)); }
.notification.sidebar-bubble.success { --tone: var(--notification-bubble-success, #16845b); }
.notification.sidebar-bubble.warning { --tone: var(--notification-bubble-warning, #a96a0d); }
.notification.sidebar-bubble.alarm { --tone: var(--notification-bubble-alarm, #c3444b); }
.sidebar-bubble .copy { display: flex; min-width: 0; min-height: 26px; align-items: center; padding: 0; }
.sidebar-bubble .copy strong { display: none; }
.sidebar-bubble .copy p {
  display: -webkit-box;
  width: 100%;
  min-width: 0;
  overflow: hidden;
  margin: 0;
  color: var(--notification-custom-bubble-body-color, var(--notification-bubble-body, #52525b));
  font-size: 11px;
  line-height: 1.4;
  white-space: pre-wrap;
  overflow-wrap: anywhere;
  -webkit-box-orient: vertical;
  -webkit-line-clamp: 2;
}
.plugin-mark {
  position: relative;
  z-index: 1;
  display: grid;
  width: 26px;
  height: 26px;
  place-items: center;
  overflow: hidden;
  border: 0;
  border-radius: 0;
  background: transparent;
  color: var(--notification-bubble-info, #4d80ff);
}
.tail-left .toast-enter-from, .tail-left .toast-leave-to { transform: translateX(10px); }
.tail-right .toast-enter-from, .tail-right .toast-leave-to { transform: translateX(-10px); }
.tail-top .toast-enter-from, .tail-top .toast-leave-to { transform: translateY(10px); }
@media (prefers-reduced-motion: reduce) { .toast-enter-active, .toast-leave-active { transition: none; } }
</style>
