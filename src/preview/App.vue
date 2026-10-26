<script setup lang="ts">
import Button from '@/components/ui/button/Button.vue'
import { computed, onMounted, onUnmounted, reactive, ref } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import {
  EVENTS,
  getCurrentPreview,
  getState,
  previewHideNow,
  previewWindowEnter,
  previewWindowLeave,
  refreshPreview,
  type PreviewAction,
  type PreviewItem,
  type PreviewPayload
} from '../lib/api'
import Icon from '../ui/Icon.vue'
import { pluginCall } from '../lib/pluginApi'
import { applyTheme } from '../lib/theme'
import SkinSurfaceHost from '../views/SkinSurfaceHost.vue'
import type { Settings } from '../lib/types'

const preview = ref<PreviewPayload | null>(null)
const settings = ref<Settings | null>(null)
const actionLoading = ref<string | null>(null)
const actionError = ref<string>('')
const displayMode = computed(() =>
  settings.value?.appearance.preview_display_mode
    ?? preview.value?.hostLayout?.displayMode
    ?? 'default'
)
const isCompact = computed(() => displayMode.value === 'compact')
const defaultStatusText = computed(() =>
  preview.value?.statusText ?? preview.value?.stateText ?? ''
)

function previewTagStyle(color?: string) {
  return { '--group-color': color && /^#[0-9a-f]{6}$/i.test(color) ? color : '#94a3b8' }
}
const visibleActions = computed(() =>
  preview.value?.actions?.filter((action) => Boolean(action.method)) ?? []
)
const itemListStyle = computed(() => {
  const items = preview.value?.items ?? []
  const rows = preview.value?.hostLayout?.visibleRows
    ?? Math.min(items.length, settings.value?.appearance.preview_visible_rows ?? 3)
  const height = rows > 0 ? rows * 36 + rows - 1 + 10 : 0
  return height > 0 ? { maxHeight: `${height}px` } : {}
})
interface CustomPreviewSlot {
  pluginId: string
  customUrl: string
  ready: boolean
}
const customSlots = reactive<CustomPreviewSlot[]>([])
const customFrameRefs = new Map<string, HTMLIFrameElement>()

function setCustomFrameRef(id: string, el: unknown) {
  if (el instanceof HTMLIFrameElement) {
    customFrameRefs.set(id, el)
  } else {
    customFrameRefs.delete(id)
  }
}

function ensureCustomSlot(p: PreviewPayload | null) {
  if (!p || p.mode !== 'custom' || !p.customUrl) return
  let slot = customSlots.find((s) => s.pluginId === p.pluginId)
  if (!slot) {
    if (customSlots.length >= 6) {
      const removed = customSlots.shift()
      if (removed) customFrameRefs.delete(removed.pluginId)
    }
    customSlots.push({ pluginId: p.pluginId, customUrl: p.customUrl, ready: false })
  } else if (slot.customUrl !== p.customUrl) {
    slot.customUrl = p.customUrl
    slot.ready = false
  }
  if (slot.ready) {
    postMessageSafe(customFrameRefs.get(p.pluginId)?.contentWindow, {
      __agileTiles: true,
      type: 'preview-update',
      plugin_id: p.pluginId
    })
  }
}

let unlistenUpdate: UnlistenFn | null = null
let unlistenSettings: UnlistenFn | null = null
let unlistenPluginEvent: UnlistenFn | null = null
let themeObserver: MutationObserver | null = null

const MAX_MESSAGE_BYTES = 256 * 1024
const MAX_MESSAGES_PER_SECOND = 120
const THEME_VARIABLES = [
  '--accent',
  '--bg',
  '--surface',
  '--surface-subtle',
  '--surface-container',
  '--primary',
  '--primary-foreground',
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
  '--font-ui',
  '--font-mono',
  '--preview-panel-opacity',
  '--preview-panel-opacity-percent'
]
let messageWindowStartedAt = 0
let messageWindowCount = 0

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

function themeMessage(id: string) {
  const root = document.documentElement
  const styles = getComputedStyle(root)
  const variables: Record<string, string> = {}
  for (const name of THEME_VARIABLES) {
    const value = styles.getPropertyValue(name).trim()
    if (value) variables[name] = value
  }
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

function previewContextMessage(id: string) {
  return {
    __agileTiles: true,
    type: 'preview-context',
    plugin_id: id,
    displayMode: displayMode.value
  }
}

function syncFrameTheme(targetPluginId?: string) {
  const pids = targetPluginId ? [targetPluginId] : customSlots.map((s) => s.pluginId)
  for (const pid of pids) {
    const frame = customFrameRefs.get(pid)
    if (frame?.contentWindow) {
      postMessageSafe(frame.contentWindow, themeMessage(pid))
    }
  }
}

function syncCustomPreviewContext(targetPluginId?: string) {
  const pids = targetPluginId ? [targetPluginId] : customSlots.map((slot) => slot.pluginId)
  for (const pid of pids) {
    const frame = customFrameRefs.get(pid)
    if (frame?.contentWindow) {
      postMessageSafe(frame.contentWindow, previewContextMessage(pid))
    }
  }
}

function onCustomFrameLoad(pluginId: string) {
  const slot = customSlots.find((s) => s.pluginId === pluginId)
  if (slot) slot.ready = true
  syncFrameTheme(pluginId)
  syncCustomPreviewContext(pluginId)
}

interface UopRequest {
  __agileTiles: true
  type?: string
  id: number | string
  plugin_id?: string
  action: string
  payload?: {
    method?: unknown
    payload?: unknown
    timeout_ms?: unknown
  }
}

interface PluginEventPayload {
  plugin_id: string
  topic: string
  payload?: unknown
}

function messageAllowed(data: UopRequest): boolean {
  const serialized = JSON.stringify(data)
  if (!serialized || new TextEncoder().encode(serialized).byteLength > MAX_MESSAGE_BYTES) return false
  const now = Date.now()
  if (now - messageWindowStartedAt >= 1000) {
    messageWindowStartedAt = now
    messageWindowCount = 0
  }
  messageWindowCount += 1
  return messageWindowCount <= MAX_MESSAGES_PER_SECOND
}

function frameSourceMatches(source: MessageEventSource | null): boolean {
  for (const frame of customFrameRefs.values()) {
    if (frame?.contentWindow === source) return true
  }
  return false
}

async function onMessage(e: MessageEvent) {
  const d = e.data as UopRequest | null
  if (!d || d.__agileTiles !== true) return
  if (!frameSourceMatches(e.source)) return
  const currentPluginId = d.plugin_id || preview.value?.pluginId
  if (!currentPluginId) return

  if (d.type === 'close-preview' || d.type === 'close-detail' || d.action === 'close') {
    void previewHideNow().catch(() => {})
    return
  }
  if ((typeof d.id !== 'number' && typeof d.id !== 'string') || !messageAllowed(d)) return

  const payload = d.payload ?? {}
  try {
    let result: unknown = null
    switch (d.action) {
      case 'call':
        // 强制使用当前插件 ID，杜绝越权
        result = await pluginCall(
          currentPluginId,
          String(payload.method ?? ''),
          payload.payload ?? {},
          Number(payload.timeout_ms) || undefined
        )
        break
      default:
        throw new Error(`不支持的操作: ${String(d.action)}`)
    }
    postMessageSafe(e.source as Window, {
      __agileTiles: true,
      id: d.id,
      plugin_id: currentPluginId,
      result
    })
  } catch (err) {
    postMessageSafe(e.source as Window, {
      __agileTiles: true,
      id: d.id,
      plugin_id: currentPluginId,
      error: err instanceof Error ? err.message : String(err)
    })
  }
}

async function handleAction(action: PreviewAction) {
  if (!preview.value) return
  if (!action.method) return

  // 插件通用自定义 Action 调用
  try {
    actionLoading.value = action.id
    actionError.value = ''
    await pluginCall(preview.value.pluginId, action.method, action.payload ?? {})

    // 执行完毕后，刷新预览数据
    const latest = await refreshPreview().catch(() => null)
    if (latest) {
      preview.value = latest
    }
  } catch (err) {
    console.error('执行插件操作失败:', err)
    actionError.value = err instanceof Error ? err.message : String(err)
  } finally {
    actionLoading.value = null
  }
}

async function handleItemClick(item: PreviewItem) {
  if (!preview.value) return
  try {
    actionLoading.value = item.id
    actionError.value = ''
    await pluginCall(
      preview.value.pluginId,
      item.method || 'launch',
      item.payload || { id: item.id }
    )
    if (!item.method || item.method === 'launch' || item.method === 'open_terminal') {
      await previewHideNow().catch(() => {})
    } else {
      const latest = await refreshPreview().catch(() => null)
      if (latest) preview.value = latest
    }
  } catch (err) {
    console.error('启动或执行失败:', err)
    actionError.value = err instanceof Error ? err.message : String(err)
  } finally {
    actionLoading.value = null
  }
}

function onCardMouseEnter() {
  previewWindowEnter().catch(() => {})
}

function onCardMouseLeave() {
  previewWindowLeave().catch(() => {})
}

onMounted(async () => {
  window.addEventListener('message', onMessage)

  // 监听主题属性变更并同步给 Custom Preview iframe
  themeObserver = new MutationObserver(() => syncFrameTheme())
  themeObserver.observe(document.documentElement, {
    attributes: true,
    attributeFilter: ['data-theme', 'data-skin', 'style']
  })

  // 同步宿主主题与外观
  try {
    const state = await getState()
    settings.value = state.settings
    applyTheme(state.settings)
  } catch {}

  unlistenSettings = await listen<Settings>(EVENTS.settingsChanged, (e) => {
    settings.value = e.payload
    applyTheme(e.payload)
    syncFrameTheme()
    syncCustomPreviewContext()
  })

  // 主动读取当前预览数据（防止首屏冷启动时错过广播事件）
  try {
    const initial = await getCurrentPreview()
    if (initial) {
      preview.value = initial
      ensureCustomSlot(initial)
    }
  } catch {}

  // 监听后续动态广播更新
  unlistenUpdate = await listen<PreviewPayload>(EVENTS.previewUpdate, (e) => {
    preview.value = e.payload
    ensureCustomSlot(e.payload)
  })

  // 转发插件级事件至 iframe
  unlistenPluginEvent = await listen<PluginEventPayload>('plugin://event', (event) => {
    const p = event.payload
    const target = customFrameRefs.get(p.plugin_id)?.contentWindow
    if (!target) return
    postMessageSafe(target, {
      __agileTiles: true,
      type: 'event',
      plugin_id: p.plugin_id,
      event: {
        topic: p.topic,
        payload: p.payload
      }
    })
  })
})

onUnmounted(() => {
  window.removeEventListener('message', onMessage)
  customSlots.length = 0
  customFrameRefs.clear()
  if (themeObserver) themeObserver.disconnect()
  if (unlistenUpdate) unlistenUpdate()
  if (unlistenSettings) unlistenSettings()
  if (unlistenPluginEvent) unlistenPluginEvent()
})
</script>

<template>
  <div
    class="preview-container"
    @mouseenter="onCardMouseEnter"
    @mouseleave="onCardMouseLeave"
    @contextmenu.prevent
  >
    <div
      v-if="preview"
      class="preview-card"
      :class="{ 'is-custom': preview.mode === 'custom', 'is-compact': isCompact && preview.mode !== 'custom' }"
    >
      <!-- Mode 1: Custom Preview (插件全权负责 UI) -->
      <div v-show="preview.mode === 'custom'" class="custom-container">
        <iframe
          v-for="slot in customSlots"
          :key="slot.pluginId"
          :ref="(el) => setCustomFrameRef(slot.pluginId, el)"
          :src="slot.customUrl"
          class="custom-frame"
          :class="{ active: slot.pluginId === preview.pluginId && slot.ready }"
          sandbox="allow-scripts"
          @load="onCustomFrameLoad(slot.pluginId)"
        />
      </div>

      <!-- Mode 2 & 3: Declarative / Static Preview (通用原语结构化渲染) -->
      <template v-if="preview.mode !== 'custom'">
        <!-- 头部：标题、副标题与呼吸状态徽标 -->
        <div v-if="isCompact && preview.stateText" class="compact-status-row">
          <div class="status-badge" :class="`status-${preview.status || 'info'}`">
            <span class="status-dot" />
            <span class="status-text">{{ preview.stateText }}</span>
          </div>
        </div>
        <div v-else-if="!isCompact" class="header">
          <div class="preview-app-icon"><Icon name="puzzle" :size="16" /></div>
          <div class="title-group">
            <div class="title">{{ preview.title }}</div>
            <div v-if="preview.subtitle" class="subtitle">{{ preview.subtitle }}</div>
          </div>

          <div v-if="defaultStatusText" class="status-badge" :class="`status-${preview.status || 'info'}`">
            <span class="status-dot" />
            <span class="status-text">{{ defaultStatusText }}</span>
          </div>
        </div>
        <div v-if="!isCompact" class="preview-divider" />
        <div class="preview-content">

        <!-- 进度条原语 -->
        <div v-if="preview.progress" class="preview-progress">
          <div v-if="preview.progress.label" class="progress-label">{{ preview.progress.label }}</div>
          <div class="progress-track">
            <div
              class="progress-fill"
              :style="{ width: `${Math.min(100, Math.max(0, (preview.progress.value / (preview.progress.max || 100)) * 100))}%` }"
            />
          </div>
        </div>

        <!-- 快捷条目列表原语（针对应用启动器等提供 items 的插件） -->
        <div v-if="preview.items && preview.items.length > 0" class="items-list" :style="itemListStyle">
          <Button variant="ghost" size="sm"
            v-for="item in preview.items"
            :key="item.id"
            class="preview-item-btn"
            :disabled="actionLoading !== null"
            :title="item.tooltip || (item.subtitle ? `${item.title} (${item.subtitle})` : `打开 ${item.title}`)"
            @click="handleItemClick(item)"
          >
            <img v-if="item.icon" :src="item.icon" class="item-icon" alt="" draggable="false" />
            <div v-else class="item-icon-placeholder">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2.5" stroke-linecap="round" stroke-linejoin="round">
                <polygon points="5 3 19 12 5 21 5 3"></polygon>
              </svg>
            </div>
            <span class="item-copy">
              <span class="item-title">{{ item.title }}</span>
              <span v-if="item.tags?.length" class="item-tags" aria-label="所属分组">
                <span v-for="tag in item.tags.slice(0, 2)" :key="tag.label" class="item-group-tag" :style="previewTagStyle(tag.color)" :title="tag.label">
                  <i aria-hidden="true" /><span>{{ tag.label }}</span>
                </span>
                <span v-if="item.tags.length > 2" class="item-tag-more" :title="item.tags.slice(2).map((tag) => tag.label).join('、')">+{{ item.tags.length - 2 }}</span>
              </span>
            </span>
            <span v-if="item.badge" class="item-badge">{{ item.badge }}</span>
            <span v-if="item.subtitle" class="item-subtitle">{{ item.subtitle }}</span>
            <span v-if="actionLoading === item.id" class="spinner item-spinner" />
            <span v-else class="item-arrow">
              <svg width="12" height="12" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2" stroke-linecap="round" stroke-linejoin="round">
                <polyline points="9 18 15 12 9 6"></polyline>
              </svg>
            </span>
          </Button>
        </div>

        <!-- 空条目状态 -->
        <div v-else-if="preview.items && preview.items.length === 0" class="empty-preview">
          <div class="empty-icon">
            <svg width="22" height="22" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.8" stroke-linecap="round" stroke-linejoin="round">
              <path d="M4.5 16.5c-1.5 1.26-2 5-2 5s3.74-.5 5-2c.71-.84.7-2.13-.09-2.91a2.18 2.18 0 0 0-2.91-.09z" />
              <path d="m12 15-3-3a22 22 0 0 1 2-3.95A12.88 12.88 0 0 1 22 2c0 2.72-.78 7.5-6 11a22.35 22.35 0 0 1-4 2z" />
            </svg>
          </div>
          <div class="empty-text">暂无可展示内容</div>
          <div class="empty-hint">可在插件详情页开启或配置常用项展示</div>
        </div>

        <!-- 动态指标网格原语 -->
        <div v-else-if="!isCompact && preview.metrics && preview.metrics.length > 0" class="metrics-grid">
          <div v-for="(metric, idx) in preview.metrics" :key="idx" class="metric-item">
            <span class="metric-label">{{ metric.label }}</span>
            <span class="metric-value">{{ metric.value }}</span>
            <span v-if="metric.subvalue" class="metric-subvalue">{{ metric.subvalue }}</span>
          </div>
        </div>

        <!-- 错误提示 -->
        <div v-if="actionError" class="action-error" :title="actionError">
          {{ actionError }}
        </div>
        </div>

        <!-- 底部快捷操作原语 -->
        <div v-if="visibleActions.length > 0" class="actions-bar">
          <Button :variant="action.primary ? 'default' : action.danger ? 'destructive' : 'outline'" size="sm"
            v-for="action in visibleActions"
            :key="action.id"
            class="action-btn"
            :class="{
              primary: action.primary,
              danger: action.danger,
              loading: actionLoading === action.id
            }"
            :disabled="actionLoading !== null"
            @click="handleAction(action)"
          >
            <span v-if="actionLoading === action.id" class="spinner" />
            <span>{{ action.label }}</span>
          </Button>
        </div>
      </template>
    </div>
  </div>
  <SkinSurfaceHost
    v-if="settings"
    :skin-id="settings.appearance.skin"
    surface="preview"
    :enabled="Boolean(preview)"
    :context="{ settings, preview, hostLayout: preview?.hostLayout ?? null }"
  />
</template>

<style scoped>
.preview-container {
  width: 100vw;
  height: 100vh;
  box-sizing: border-box;
  padding: 6px;
  background: transparent;
  display: flex;
  user-select: none;
  overflow: hidden;
}

.preview-card {
  position: relative;
  isolation: isolate;
  width: 100%;
  height: 100%;
  box-sizing: border-box;
  background: color-mix(in srgb, var(--surface) 88%, transparent);
  backdrop-filter: blur(20px) saturate(180%);
  -webkit-backdrop-filter: blur(20px) saturate(180%);
  border: 1px solid color-mix(in srgb, var(--border) 70%, transparent);
  border-radius: 12px;
  box-shadow: var(--shadow);
  padding: 12px 14px;
  display: flex;
  flex-direction: column;
  justify-content: flex-start;
  gap: 8px;
  animation: card-appear 0.18s cubic-bezier(0.16, 1, 0.3, 1);
  overflow: hidden;
}

.preview-card::before {
  content: '';
  position: absolute;
  z-index: 0;
  inset: 0;
  border-radius: inherit;
  background-image: var(--app-wallpaper-image, none);
  background-size: var(--app-wallpaper-size, cover);
  background-position: center;
  background-repeat: no-repeat;
  filter: blur(var(--app-wallpaper-blur, 0px));
  opacity: var(--app-wallpaper-opacity, 0);
  pointer-events: none;
  transform: scale(1.025);
}

.preview-card > * {
  position: relative;
  z-index: 1;
}

.preview-card.is-custom {
  padding: 0;
}

.custom-container {
  width: 100%;
  height: 100%;
  position: relative;
}

.custom-frame {
  position: absolute;
  inset: 0;
  width: 100%;
  height: 100%;
  border: none;
  background: transparent;
  visibility: hidden;
  opacity: 0;
  pointer-events: none;
  transition: opacity 0.1s ease;
}

.custom-frame.active {
  visibility: visible;
  opacity: 1;
  pointer-events: auto;
}

@keyframes card-appear {
  from {
    opacity: 0;
    transform: scale(0.96);
  }
  to {
    opacity: 1;
    transform: scale(1);
  }
}

.header {
  display: flex;
  align-items: flex-start;
  justify-content: space-between;
  gap: 10px;
}

.compact-status-row {
  display: flex;
  min-height: 18px;
  flex: none;
  align-items: center;
  justify-content: flex-end;
}

.title-group {
  flex: 1;
  min-width: 0;
}

.title {
  font-size: 13px;
  font-weight: 600;
  color: var(--text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  letter-spacing: -0.01em;
}

.subtitle {
  font-size: 11px;
  color: var(--text-muted);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  margin-top: 2px;
}

.status-badge {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  padding: 2px 7px;
  border-radius: 999px;
  font-size: 11px;
  font-weight: 500;
  background: var(--surface-subtle);
  border: 1px solid var(--border);
  flex-shrink: 0;
}

.status-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
}

.status-online .status-dot {
  background: var(--success);
  box-shadow: 0 0 6px var(--success);
}
.status-online {
  color: var(--success);
}

.status-busy .status-dot {
  background: var(--warning);
  box-shadow: 0 0 6px var(--warning);
}
.status-busy {
  color: var(--warning);
}

.status-offline .status-dot {
  background: var(--text-muted);
}
.status-offline {
  color: var(--text-muted);
}

.status-error .status-dot {
  background: var(--danger);
  box-shadow: 0 0 6px var(--danger);
}
.status-error {
  color: var(--danger);
}

.status-info .status-dot {
  background: var(--primary);
  box-shadow: 0 0 6px var(--primary);
}
.status-info {
  color: var(--primary);
}

.preview-progress {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.progress-label {
  font-size: 10.5px;
  color: var(--text-muted);
}

.progress-track {
  width: 100%;
  height: 5px;
  border-radius: 999px;
  background: var(--surface-subtle);
  border: 1px solid var(--border);
  overflow: hidden;
}

.progress-fill {
  height: 100%;
  background: var(--primary);
  border-radius: 999px;
  transition: width 0.25s ease;
}

.items-list {
  display: flex;
  flex-direction: column;
  gap: 4px;
  flex-shrink: 0;
  min-height: 0;
  overflow-y: auto;
  overflow-x: hidden;
  padding-right: 2px;
}

.preview-item-btn {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 5px 8px;
  border-radius: 7px;
  border: 1px solid transparent;
  background: color-mix(in srgb, var(--surface-subtle) 60%, transparent);
  color: var(--text);
  cursor: pointer;
  transition: all 0.15s ease;
  width: 100%;
  height: 36px;
  flex: 0 0 36px;
  box-sizing: border-box;
  text-align: left;
}

.preview-item-btn:hover:not(:disabled) {
  background: var(--surface-subtle);
  border-color: var(--border);
  transform: translateX(2px);
}

.preview-item-btn:active:not(:disabled) {
  transform: scale(0.98);
}

.item-icon {
  width: 18px;
  height: 18px;
  border-radius: 4px;
  flex-shrink: 0;
  object-fit: contain;
}

.item-icon-placeholder {
  width: 18px;
  height: 18px;
  border-radius: 4px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  background: color-mix(in srgb, var(--primary) 15%, transparent);
  color: var(--primary);
}

.item-title {
  flex: 1;
  font-size: 11.5px;
  font-weight: 500;
  color: var(--text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.item-badge {
  font-size: 9.5px;
  padding: 1px 5px;
  border-radius: 4px;
  background: var(--surface-container);
  color: var(--text-muted);
  border: 1px solid var(--border);
  flex-shrink: 0;
}

.item-subtitle {
  font-size: 10px;
  color: var(--text-muted);
  flex-shrink: 0;
}

.item-arrow {
  color: var(--text-muted);
  opacity: 0;
  transition: all 0.15s ease;
  display: flex;
  align-items: center;
  flex-shrink: 0;
}

.preview-item-btn:hover .item-arrow {
  opacity: 1;
  transform: translateX(1px);
}

.item-spinner {
  flex-shrink: 0;
}

.empty-preview {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 16px 8px;
  text-align: center;
  gap: 3px;
}

.empty-icon {
  color: var(--text-muted);
  margin-bottom: 3px;
  opacity: 0.6;
}

.empty-text {
  font-size: 12px;
  font-weight: 600;
  color: var(--text);
}

.empty-hint {
  font-size: 10.5px;
  color: var(--text-muted);
  line-height: 1.4;
}

.metrics-grid {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: 6px;
  padding: 6px 8px;
  background: color-mix(in srgb, var(--surface-subtle) 60%, transparent);
  border-radius: 8px;
  border: 1px solid color-mix(in srgb, var(--border) 40%, transparent);
}

.metric-item {
  display: flex;
  flex-direction: column;
  gap: 1px;
  min-width: 0;
}

.metric-label {
  font-size: 10px;
  color: var(--text-muted);
}

.metric-value {
  font-size: 11px;
  font-weight: 500;
  color: var(--text);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.metric-subvalue {
  font-size: 9.5px;
  color: var(--text-muted);
}

.actions-bar {
  display: flex;
  align-items: center;
  justify-content: flex-end;
  gap: 8px;
  margin-top: 2px;
}

.action-error {
  font-size: 11px;
  color: var(--danger);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
  text-align: right;
  line-height: 1.2;
}

.action-btn {
  height: 26px;
  padding: 0 10px;
  border-radius: 6px;
  font-size: 11px;
  font-weight: 500;
  cursor: pointer;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  transition: all 0.15s ease;
  border: 1px solid var(--border);
  background: var(--surface-subtle);
  color: var(--text);
}

.action-btn:hover:not(:disabled) {
  border-color: var(--border-strong);
  background: color-mix(in srgb, var(--surface-subtle) 80%, var(--text) 10%);
}

.action-btn.primary {
  background: var(--primary);
  border-color: var(--primary);
  color: var(--primary-foreground);
}

.action-btn.primary:hover:not(:disabled) {
  filter: brightness(1.08);
}

.action-btn.danger {
  background: color-mix(in srgb, var(--danger) 12%, transparent);
  border-color: color-mix(in srgb, var(--danger) 45%, var(--border));
  color: var(--danger);
}

.action-btn.danger:hover:not(:disabled) {
  background: color-mix(in srgb, var(--danger) 22%, transparent);
  border-color: var(--danger);
}

.action-btn:disabled {
  opacity: 0.6;
  cursor: not-allowed;
}

.spinner {
  width: 10px;
  height: 10px;
  border: 2px solid currentColor;
  border-right-color: transparent;
  border-radius: 50%;
  animation: spin 0.7s linear infinite;
}

@keyframes spin {
  from {
    transform: rotate(0deg);
  }
  to {
    transform: rotate(360deg);
  }
}

.preview-container { padding: 7px; }
.preview-card {
  --preview-panel-fill: color-mix(in srgb, var(--card) var(--preview-panel-opacity-percent, 97%), transparent);
  --preview-muted-fill: color-mix(in srgb, var(--muted) var(--preview-panel-opacity-percent, 97%), transparent);
  gap: 0;
  padding: 0;
  border: 1px solid color-mix(in srgb, var(--border) var(--preview-panel-opacity-percent, 97%), transparent);
  border-radius: 10px;
  background: var(--preview-panel-fill);
  box-shadow: none;
  backdrop-filter: none;
  -webkit-backdrop-filter: none;
}
.preview-card.is-custom { padding: 0; }
.header { min-height: 46px; align-items: center; gap: 8px; padding: 7px 10px; }
.preview-app-icon { display: grid; width: 30px; height: 30px; flex: none; place-items: center; border: 1px solid color-mix(in srgb, var(--border) var(--preview-panel-opacity-percent, 97%), transparent); border-radius: 7px; background: var(--preview-muted-fill); color: var(--foreground); }
.title-group { display: flex; min-width: 0; flex-direction: column; gap: 2px; }
.title { color: var(--foreground); font-size: 10px; font-weight: 600; line-height: 1.15; letter-spacing: -.01em; }
.subtitle { margin-top: 0; color: var(--muted-foreground); font-size: 8px; }
.status-badge { padding: 2px 6px; border-color: color-mix(in srgb, var(--border) var(--preview-panel-opacity-percent, 97%), transparent); border-radius: 5px; background: var(--preview-muted-fill); font-size: 8px; }
.status-dot { width: 6px; height: 6px; box-shadow: none; }
.preview-divider { height: 1px; flex: none; background: color-mix(in srgb, var(--border) var(--preview-panel-opacity-percent, 97%), transparent); }
.preview-content { display: flex; min-height: 0; flex: 1; flex-direction: column; gap: 7px; overflow: auto; padding: 8px 10px; }
.preview-progress { gap: 5px; padding: 8px 9px; border: 1px solid color-mix(in srgb, var(--border) var(--preview-panel-opacity-percent, 97%), transparent); border-radius: 7px; background: var(--preview-panel-fill); }
.progress-label { color: var(--muted-foreground); font-size: 8px; }
.progress-track { height: 4px; border: 0; background: var(--preview-muted-fill); }
.progress-fill { border-radius: 999px; background: var(--primary); }
.items-list { display: flex; min-height: 0; flex: 0 1 auto; flex-direction: column; gap: 1px; overflow-x: hidden; overflow-y: auto; overscroll-behavior: contain; padding: 4px; border: 1px solid color-mix(in srgb, var(--border) var(--preview-panel-opacity-percent, 97%), transparent); border-radius: 8px; background: var(--preview-panel-fill); }
.preview-item-btn { display: flex; min-height: 36px; height: 36px; width: 100%; flex: 0 0 36px; justify-content: flex-start; gap: 7px; padding: 4px 6px; border: 0; border-bottom: 1px solid color-mix(in srgb, var(--border) var(--preview-panel-opacity-percent, 97%), transparent); border-radius: 0; background: transparent; color: var(--foreground); text-align: left; }
.preview-item-btn:last-child { border-bottom: 0; }
.preview-item-btn:hover:not(:disabled) { background: var(--preview-muted-fill); color: var(--foreground); }
.item-icon,
.item-icon-placeholder { width: 24px; height: 24px; flex: none; border: 1px solid color-mix(in srgb, var(--border) var(--preview-panel-opacity-percent, 97%), transparent); border-radius: 6px; background: var(--preview-panel-fill); }
.item-icon-placeholder { display: grid; place-items: center; color: var(--muted-foreground); }
.item-title { min-width: 0; overflow: hidden; font-size: 10px; font-weight: 550; text-overflow: ellipsis; white-space: nowrap; }
.item-badge { border: 1px solid color-mix(in srgb, var(--border) var(--preview-panel-opacity-percent, 97%), transparent); border-radius: 4px; background: var(--preview-muted-fill); color: var(--muted-foreground); font-size: 8px; }
.item-subtitle { max-width: 82px; overflow: hidden; color: var(--muted-foreground); font-size: 8px; text-overflow: ellipsis; white-space: nowrap; }
.item-arrow { margin-left: auto; color: var(--muted-foreground); }
.item-copy { display: flex; flex: 1; flex-direction: column; justify-content: center; gap: 2px; min-width: 0; overflow: hidden; }
.item-copy .item-title { flex: none; display: block; line-height: 12px; }
.item-tags { display: flex; align-items: center; gap: 4px; min-width: 0; overflow: hidden; }
.item-group-tag { display: inline-flex; align-items: center; gap: 3px; min-width: 0; max-width: 92px; padding: 0 4px; border: 1px solid color-mix(in srgb, var(--group-color) 35%, transparent); border-radius: 3px; background: color-mix(in srgb, var(--group-color) 12%, transparent); color: var(--foreground); font-size: 8px; line-height: 11px; }
.item-group-tag i { width: 4px; height: 4px; flex: none; border-radius: 50%; background: var(--group-color); }
.item-group-tag > span { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
.item-tag-more { flex: none; color: var(--muted-foreground); font-size: 8px; }
.metrics-grid { grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 6px; padding: 5px 6px; }
.metric-item { display: flex; min-height: 48px; flex-direction: column; justify-content: center; gap: 2px; padding: 6px 8px; border: 1px solid color-mix(in srgb, var(--border) var(--preview-panel-opacity-percent, 97%), transparent); border-radius: 7px; background: var(--preview-panel-fill); }
.metric-label { color: var(--muted-foreground); font-size: 8px; }
.metric-value { color: var(--foreground); font-size: 12px; font-weight: 600; font-variant-numeric: tabular-nums; }
.metric-subvalue { color: var(--muted-foreground); font-size: 8px; }
.empty-preview { display: flex; min-height: 96px; flex: 1; flex-direction: column; align-items: center; justify-content: center; gap: 5px; padding: 10px; border: 1px dashed var(--border); border-radius: 8px; background: transparent; text-align: center; }
.empty-icon { width: 28px; height: 28px; margin-bottom: 1px; border: 1px solid color-mix(in srgb, var(--border) var(--preview-panel-opacity-percent, 97%), transparent); border-radius: 7px; background: var(--preview-muted-fill); color: var(--muted-foreground); }
.empty-text { color: var(--foreground); font-size: 9px; font-weight: 550; }
.empty-hint { color: var(--muted-foreground); font-size: 8px; }
.action-error { padding: 6px 8px; border: 1px solid color-mix(in srgb, color-mix(in srgb, var(--destructive) 30%, transparent) var(--preview-panel-opacity-percent, 97%), transparent); border-radius: 6px; background: color-mix(in srgb, color-mix(in srgb, var(--destructive) 8%, transparent) var(--preview-panel-opacity-percent, 97%), transparent); color: var(--destructive); font-size: 8px; }
.actions-bar { flex: none; flex-wrap: wrap; gap: 5px; padding: 7px 10px; border-top: 1px solid color-mix(in srgb, var(--border) var(--preview-panel-opacity-percent, 97%), transparent); background: color-mix(in srgb, color-mix(in srgb, var(--muted) 32%, transparent) var(--preview-panel-opacity-percent, 97%), transparent); }
.actions-bar :deep(button) { min-height: 26px; border-radius: 5px; font-size: 8px; }
.action-btn.primary { background: var(--primary); color: var(--primary-foreground); }
.action-btn.danger { border-color: color-mix(in srgb, var(--destructive) 30%, var(--border)); background: color-mix(in srgb, var(--destructive) 8%, var(--background)); color: var(--destructive); }
.action-btn.danger:hover:not(:disabled) { background: color-mix(in srgb, var(--destructive) 14%, var(--background)); }
</style>
