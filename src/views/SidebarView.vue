<script setup lang="ts">
import Button from '@/components/ui/button/Button.vue'
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import Icon from '../ui/Icon.vue'
import Toast from '../ui/Toast.vue'
import {
  openPluginDetail,
  openDetail,
  sidebarHoverEnter,
  sidebarCollapseIfUnhovered,
  previewSidebarEnter,
  previewSidebarLeave,
  previewHideNow
} from '../lib/api'
import { hexToRgba } from '../lib/theme'
import type { AppearanceSettings, PluginMeta, Settings } from '../lib/types'

const props = defineProps<{
  appearance: AppearanceSettings
  plugins: PluginMeta[]
  expanded: boolean
  general: Settings['general']
}>()

/**
 * 前端只保留 150ms 防抖（连续划过边缘不抖动），
 * 真正的 auto_hide_delay 延迟由 Rust 侧 sidebar_collapse_if_unhovered 负责，
 * 避免前后端各睡一次导致延迟翻倍。
 */
const LEAVE_DEBOUNCE_MS = 150

let hoverToken = 0
let leaveTimer = -1
/** 未决的 hover_enter Promise：mouseleave 可能发生在 IPC 返回之前，离开时先等它拿到真实 token */
let pendingEnter: Promise<number> | null = null

function onMouseEnter() {
  // 悬停展开被禁用时，鼠标碰到 peek 不应强制展开，也不调度收起
  if (!props.general.enable_mouse_hover) return
  window.clearTimeout(leaveTimer)
  if (!pendingEnter) {
    pendingEnter = sidebarHoverEnter()
      .then((t) => {
        hoverToken = t
        return t
      })
      .catch(() => 0)
      .finally(() => {
        pendingEnter = null
      })
  }
}

function onMouseLeave() {
  if (!props.general.enable_mouse_hover) return
  window.clearTimeout(leaveTimer)
  leaveTimer = window.setTimeout(() => {
    const pending = pendingEnter
    void (async () => {
      // 等 hover_enter 返回真实 token；0 表示没有未决/成功的展开（含 IPC 失败），直接跳过
      const token = pending ? await pending : hoverToken
      if (token === 0) return
      await sidebarCollapseIfUnhovered(token).catch(() => {})
    })()
  }, LEAVE_DEBOUNCE_MS)
}

// ---------- 圆环悬停 tooltip：锚定按钮（不锚定鼠标），持续跟随按钮位置 ----------

interface TipState {
  label: string
  left: number
  top: number
}
const tooltip = ref<TipState | null>(null)
const tooltipEl = ref<HTMLElement | null>(null)
let tipButton: HTMLElement | null = null
let tipDelay = 0
let tipFollow = 0

/** 按当前按钮矩形重定位 tooltip。
 * 左右停靠放图标正上方：鼠标指针只会从落点向右下延伸，上方永远不被指针盖住
 * （含高 DPI 大指针的情况）；顶部停靠放图标下方并留足纵向间距 */
function placeTip() {
  const el = tooltipEl.value
  const cur = tooltip.value
  const btn = tipButton
  if (!el || !cur || !btn) return
  const rect = btn.getBoundingClientRect()
  const w = el.offsetWidth
  const h = el.offsetHeight
  const pad = 8
  const pos = props.appearance.sidebar_position
  let left: number
  let top: number
  if (pos === 'left') {
    left = rect.left
    top = rect.top >= h + 16 ? rect.top - h - 8 : rect.bottom + 12
  } else if (pos === 'right') {
    left = rect.right - w
    top = rect.top >= h + 16 ? rect.top - h - 8 : rect.bottom + 12
  } else {
    left = rect.left + rect.width / 2 - w / 2
    top = rect.bottom + 22
  }
  tooltip.value = {
    label: cur.label,
    left: Math.min(Math.max(pad, left), window.innerWidth - w - pad),
    top: Math.min(Math.max(pad, top), window.innerHeight - h - pad)
  }
}

function showTip(e: MouseEvent, label: string) {
  const btn = e.currentTarget as HTMLElement | null
  if (!btn) return
  tipButton = btn
  // 先离屏挂载以便测量尺寸
  tooltip.value = { label, left: -9999, top: -9999 }
  window.clearTimeout(tipDelay)
  cancelAnimationFrame(tipFollow)
  // 悬停会触发展开动画，按钮位置会移动：延迟到动画基本稳定后再定位，
  // 并在后续一段时间内逐帧跟随按钮矩形，保证 tooltip 始终贴着按钮而不被鼠标压住
  tipDelay = window.setTimeout(() => {
    void nextTick(() => {
      placeTip()
      const start = performance.now()
      const follow = () => {
        if (!tooltip.value || !tipButton) return
        placeTip()
        if (performance.now() - start < 700) {
          tipFollow = requestAnimationFrame(follow)
        }
      }
      tipFollow = requestAnimationFrame(follow)
    })
  }, 220)
}



function hideTip() {
  tooltip.value = null
  tipButton = null
  window.clearTimeout(tipDelay)
  cancelAnimationFrame(tipFollow)
}

watch(
  () => props.expanded,
  (v) => {
    if (!v) {
      tooltip.value = null
      void previewHideNow().catch(() => {})
    }
  }
)

onMounted(() => {
  document.documentElement.addEventListener('mouseenter', onMouseEnter)
  document.documentElement.addEventListener('mouseleave', onMouseLeave)
})
onUnmounted(() => {
  document.documentElement.removeEventListener('mouseenter', onMouseEnter)
  document.documentElement.removeEventListener('mouseleave', onMouseLeave)
  window.clearTimeout(leaveTimer)
  window.clearTimeout(tipDelay)
  cancelAnimationFrame(tipFollow)
  void previewHideNow().catch(() => {})
})

// ---------- 外观 ----------

const orderedPlugins = computed(() =>
  props.plugins.filter((p) => p.enabled && p.sidebar_visible)
)

// 响应式系统深浅色（matchMedia），供 theme_mode=system 时推导生效主题
const systemDark = ref(window.matchMedia('(prefers-color-scheme: dark)').matches)
function onSchemeChange(e: MediaQueryListEvent) {
  systemDark.value = e.matches
}
window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', onSchemeChange)
onUnmounted(() => {
  window.matchMedia('(prefers-color-scheme: dark)').removeEventListener('change', onSchemeChange)
})

const effectiveTheme = computed(() => {
  const mode = props.appearance.theme_mode
  if (mode === 'system') return systemDark.value ? 'dark' : 'light'
  return mode
})

const surfaceColor = computed(() => (effectiveTheme.value === 'light' ? '#f2f4f6' : '#0a0a0c'))

const railStyle = computed(() => {
  const panelOpacity = props.appearance.sidebar_panel_opacity
  // 透明度只作用于背景：底色与皮肤背景图一起淡出（0% = 完全透明只剩图标芯片）；
  // 图标圆环走实心芯片，永远清晰
  return {
    width: `${props.appearance.collapsed_width}px`,
    '--rail-color': hexToRgba(surfaceColor.value, panelOpacity),
    '--rail-img-opacity': String(props.appearance.sidebar_image_opacity),
    '--component-opacity': String(props.appearance.component_opacity),
    borderRadius: props.appearance.sidebar_position === 'top'
      ? `${Math.round(props.appearance.collapsed_width / 2) - 2}px`
      : '16px'
  }
})

const ringSize = computed(() => Math.min(32, Math.max(30, props.appearance.collapsed_width - 18)))
const iconSize = computed(() => 14)
const openError = ref('')

function onPluginMouseEnter(e: MouseEvent, plugin: PluginMeta) {
  showTip(e, plugin.name)
  const target = e.currentTarget as HTMLElement | null
  if (!target) return
  const rect = target.getBoundingClientRect()
  void previewSidebarEnter(plugin.id, {
    x: rect.left,
    y: rect.top,
    width: rect.width,
    height: rect.height
  }).catch(() => {})
}

function onPluginMouseLeave(pluginId: string) {
  hideTip()
  void previewSidebarLeave(pluginId).catch(() => {})
}

function open(page: string) {
  tooltip.value = null
  void previewHideNow().catch(() => {})
  void openDetail(page).catch((error) => {
    console.error('打开详情页失败', error)
    openError.value = String(error)
  })
}

function openPlugin(pluginId: string) {
  tooltip.value = null
  void previewHideNow().catch(() => {})
  void openPluginDetail(pluginId).catch((error) => {
    console.error('打开插件详情页失败', error)
    openError.value = String(error)
  })
}

let unlistenPreviewOpened: UnlistenFn | null = null

onMounted(async () => {
  unlistenPreviewOpened = await listen('preview://opened', () => {
    hideTip()
  })
})

onUnmounted(() => {
  if (unlistenPreviewOpened) {
    unlistenPreviewOpened()
  }
})
</script>

<template>
  <!-- Pulse 风格圆环轨道：贴边胶囊面板 + 圆环入口，收起时仅露屏幕边缘细缝 -->
  <div class="rail-wrap" @contextmenu.prevent :class="[`pos-${appearance.sidebar_position}`, { collapsed: !expanded }]">
    <div class="rail" :style="railStyle">
      <Button
        variant="ghost"
        size="icon-sm"
        v-for="p in orderedPlugins"
        :key="p.id"
        class="ring"
        :style="{ width: ringSize + 'px', height: ringSize + 'px' }"
        :aria-label="p.name"
        @click="openPlugin(p.id)"
        @mouseenter="onPluginMouseEnter($event, p)"
        @mouseleave="onPluginMouseLeave(p.id)"
      >
        <Icon :name="p.icon || 'puzzle'" :size="iconSize" :stroke-width="1.8" />
      </Button>

      <div class="spacer" />

      <Button
        variant="ghost"
        size="icon-sm"
        class="ring"
        :style="{ width: ringSize + 'px', height: ringSize + 'px' }"
        aria-label="设置"
        @click="open('settings')"
        @mouseenter="showTip($event, '设置')"
        @mouseleave="hideTip"
      >
        <Icon name="settings" :size="iconSize" :stroke-width="1.8" />
      </Button>
    </div>
    <div class="peek" :style="{ background: appearance.sidebar_border_color }" />

    <!-- 悬停 tooltip：深色圆角微卡，浮在轨道内侧 -->
    <div
      v-if="tooltip"
      ref="tooltipEl"
      class="tip"
      :style="{ left: tooltip.left + 'px', top: tooltip.top + 'px' }"
    >
      {{ tooltip.label }}
    </div>

    <Toast :message="openError" kind="error" @done="openError = ''" />
  </div>
</template>

<style scoped>
.rail-wrap {
  position: fixed;
  inset: 0;
  display: flex;
  pointer-events: none;
}

.rail {
  pointer-events: auto;
  display: flex;
  flex-direction: column;
  align-items: center;
  gap: 14px;
  padding: 12px 0;
  margin: auto;
  position: relative;
  background-color: transparent;
  border: 1px solid rgba(255, 255, 255, 0.07);
  box-shadow: 0 8px 28px rgba(0, 0, 0, 0.35);
  /* Pulse 弹性：展开时带轻微回弹的弹簧曲线 */
  transition:
    transform 0.34s cubic-bezier(0.3, 1.36, 0.42, 1),
    opacity 0.24s ease;
}

/* Keep the wallpaper, panel fill, and icon content on separate layers. */
.rail::before {
  content: '';
  position: absolute;
  inset: 0;
  border-radius: inherit;
  background-image: var(--rail-image, none);
  background-size: cover;
  background-position: center;
  opacity: var(--rail-img-opacity, 1);
  z-index: 0;
}
.rail::after {
  content: '';
  position: absolute;
  inset: 0;
  border-radius: inherit;
  background: var(--rail-color, rgba(10, 10, 12, 0.92));
  z-index: 0;
}
.rail > * {
  position: relative;
  z-index: 1;
}

:global([data-theme='light']) .rail {
  border-color: rgba(0, 0, 0, 0.11);
  box-shadow: 0 8px 24px rgba(24, 28, 36, 0.18);
}

.pos-left .rail {
  margin-left: 4px;
  margin-right: auto;
}
.pos-right .rail {
  margin-left: auto;
  margin-right: 4px;
}
.pos-top .rail {
  flex-direction: row;
  /* 横向 dock 条：横贯窗口全宽，圆环靠左、设置齿轮最右（spacer 撑开） */
  width: calc(100% - 12px) !important;
  height: v-bind('appearance.collapsed_width + "px"');
  padding: 0 14px;
  margin-top: 4px;
  margin-bottom: auto;
  margin-left: 6px;
  margin-right: auto;
  border-radius: v-bind('Math.round(appearance.collapsed_width / 2) - 2 + "px"');
}

.spacer {
  flex: 1;
}
.pos-top .spacer {
  flex: 1;
  width: auto;
}

.ring {
  flex: none;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 0;
  border-radius: 50%;
  border: 2px solid rgba(255, 255, 255, 0.16);
  /* 实心芯片：背景透明度只影响轨道底，图标始终清晰 */
  background: color-mix(in srgb, var(--surface) calc(var(--component-opacity, 1) * 100%), transparent);
  color: var(--text);
  transition:
    border-color 0.18s ease,
    color 0.18s ease,
    box-shadow 0.18s ease,
    transform 0.24s cubic-bezier(0.34, 1.56, 0.64, 1),
    opacity 0.2s ease;
}

:global([data-theme='light']) .ring {
  border-color: rgba(0, 0, 0, 0.24);
}

.ring:hover {
  border-color: var(--accent);
  color: var(--accent);
  box-shadow: 0 0 0 4px color-mix(in srgb, var(--accent) 18%, transparent);
  transform: scale(1.06);
}

.ring:active {
  transform: scale(0.96);
}

/* 展开瞬间：圆环从缩小态弹性放大浮出（带轻微逐个错峰） */
.rail-wrap:not(.collapsed) .ring {
  animation: ring-pop 0.36s cubic-bezier(0.34, 1.56, 0.64, 1) backwards;
}
.rail button:nth-of-type(2) {
  animation-delay: 22ms;
}
.rail button:nth-of-type(3) {
  animation-delay: 44ms;
}
.rail button:nth-of-type(4) {
  animation-delay: 66ms;
}
.rail button:nth-of-type(5) {
  animation-delay: 88ms;
}
.rail button:nth-of-type(6) {
  animation-delay: 110ms;
}
@keyframes ring-pop {
  from {
    transform: scale(0.72);
    opacity: 0.35;
  }
  to {
    transform: scale(1);
    opacity: 1;
  }
}

/* 收起时由宿主移动原生窗口；这里仅淡化轨道，避免内容与窗口重复位移。 */
.collapsed .rail {
  opacity: 0.9;
}
.collapsed .ring {
  transform: scale(0.82);
  opacity: 1;
}
.collapsed .ring:hover {
  transform: scale(0.9);
}

/* 细缝：收起时从边缘渐显并弹性生长 */
.peek {
  position: absolute;
  pointer-events: none;
  opacity: 0;
  transition:
    opacity 0.24s ease,
    transform 0.32s cubic-bezier(0.3, 1.36, 0.42, 1);
}
.pos-right .peek {
  left: 0;
  top: 50%;
  height: 34%;
  width: 3px;
  transform: translateY(-50%) scaleY(0.4);
  border-radius: 0 3px 3px 0;
}
.pos-left .peek {
  right: 0;
  top: 50%;
  height: 34%;
  width: 3px;
  transform: translateY(-50%) scaleY(0.4);
  border-radius: 3px 0 0 3px;
}
.pos-top .peek {
  bottom: 0;
  left: 50%;
  width: 34%;
  height: 3px;
  transform: translateX(-50%) scaleX(0.4);
  border-radius: 0 0 3px 3px;
}
.collapsed .peek {
  opacity: 0.85;
}
.pos-right.collapsed .peek,
.pos-left.collapsed .peek {
  transform: translateY(-50%) scaleY(1);
}
.pos-top.collapsed .peek {
  transform: translateX(-50%) scaleX(1);
}

/* 悬停 tooltip：深色圆角微卡 */
.tip {
  position: absolute;
  z-index: 10;
  max-width: 180px;
  padding: 5px 10px;
  border-radius: 8px;
  background: var(--popover, var(--surface));
  border: 1px solid var(--border);
  color: var(--popover-foreground, var(--text));
  font-size: 11.5px;
  white-space: nowrap;
  box-shadow: var(--shadow);
  pointer-events: none;
  animation: tip-in 0.16s ease both;
}
@keyframes tip-in {
  from {
    opacity: 0;
  }
  to {
    opacity: 1;
  }
}

.rail {
  gap: 12px;
  padding: 8px 0;
  border: 1px solid var(--border);
  border-radius: 16px;
  background: transparent;
  box-shadow: 0 8px 24px rgb(0 0 0 / 14%);
  backdrop-filter: blur(18px) saturate(140%);
}
.pos-top .rail { gap: 6px; padding: 0 8px; }
.ring {
  border: 2px solid var(--border-strong);
  border-radius: 50%;
  background: color-mix(in srgb, var(--surface) calc(var(--component-opacity, 1) * 100%), transparent);
  color: var(--foreground);
  box-shadow: inset 0 1px 0 rgb(255 255 255 / 4%);
  transition: color .15s, border-color .15s, background-color .15s, box-shadow .15s, transform .12s;
}
.ring:hover {
  transform: none;
  border-color: var(--primary);
  background: color-mix(in srgb, var(--primary) 10%, var(--surface));
  color: var(--primary);
  box-shadow: 0 0 0 3px color-mix(in srgb, var(--primary) 14%, transparent);
}
.ring:active { transform: scale(.96); }
.tip { padding: 5px 8px; border-radius: 6px; background: var(--popover); color: var(--popover-foreground); font-size: 10px; box-shadow: 0 4px 12px rgb(0 0 0 / 12%); }
</style>
