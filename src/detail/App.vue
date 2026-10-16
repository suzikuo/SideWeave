<script setup lang="ts">
import Button from '@/components/ui/button/Button.vue'
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import Icon from '../ui/Icon.vue'
import ControlCenterView from '../views/ControlCenterView.vue'
import DetailView from '../views/DetailView.vue'
import SkinSurfaceHost from '../views/SkinSurfaceHost.vue'
import { closeDetail, EVENTS, getMainSurface } from '../lib/api'
import { bindSettings } from '../lib/theme'
import {
  appVersionRef,
  autostartRef,
  dataDirRef,
  pluginsRef,
  settingsRef
} from '../lib/store'
import type { Settings } from '../lib/types'

type Surface = 'sidebar' | 'detail' | 'control'

const surface = ref<Surface>('detail')
const loaded = ref(false)
const page = ref('')
const showNativeControl = ref(false)
const skinSurfaceReady = ref(false)
const alwaysOnTop = ref(false)
const nativeWindow = getCurrentWindow()
const darkQuery = window.matchMedia('(prefers-color-scheme: dark)')
const systemDark = ref(darkQuery.matches)
let unlistenSurface: UnlistenFn | null = null

function onSchemeChange(event: MediaQueryListEvent) {
  systemDark.value = event.matches
}

async function syncAlwaysOnTop() {
  try {
    alwaysOnTop.value = await nativeWindow.isAlwaysOnTop()
  } catch {
    alwaysOnTop.value = false
  }
}

async function toggleAlwaysOnTop() {
  const next = !alwaysOnTop.value
  try {
    await nativeWindow.setAlwaysOnTop(next)
    alwaysOnTop.value = next
  } catch (error) {
    console.error('切换窗口置顶状态失败', error)
  }
}

async function minimizeWindow() {
  try {
    await nativeWindow.minimize()
  } catch (error) {
    console.error('最小化窗口失败', error)
  }
}

const effectiveTheme = computed(() => {
  const mode = settingsRef.value?.appearance.theme_mode
  if (!mode || mode === 'system') return systemDark.value ? 'dark' : 'light'
  return mode
})

const detailStyle = computed(() => ({
  '--detail-bg': effectiveTheme.value === 'light' ? '#f4f5f7' : '#101013',
  '--detail-panel-opacity': String(
    settingsRef.value?.appearance.skin === 'default'
      ? settingsRef.value.appearance.control_panel_opacity
      : surface.value === 'control'
        ? settingsRef.value?.appearance.control_panel_opacity ?? 1
        : settingsRef.value?.appearance.detail_panel_opacity ?? 1
  ),
  '--detail-image-opacity': String(settingsRef.value?.appearance.detail_image_opacity ?? 1),
  '--component-opacity': String(settingsRef.value?.appearance.component_opacity ?? 1),
  '--component-glass-filter': settingsRef.value?.appearance.glass_effect_enabled ? 'blur(4px)' : 'none'
}))

const isSkinHomePage = computed(() => !page.value || page.value === 'home')
const isPluginDetailPage = computed(() =>
  Boolean(page.value) && !['home', 'settings', 'market-plugins', 'market-skins'].includes(page.value)
)

watch(() => [surface.value, page.value] as const, () => {
  showNativeControl.value = false
})

function syncPage() {
  page.value = window.location.hash.replace(/^#\//, '')
}

onMounted(async () => {
  syncPage()
  window.addEventListener('hashchange', syncPage)
  darkQuery.addEventListener('change', onSchemeChange)
  unlistenSurface = await listen<Surface>(EVENTS.mainSurface, (event) => {
    surface.value = event.payload
    void syncAlwaysOnTop()
  }).catch(() => null)

  const snapshot = await bindSettings((fresh: Settings) => {
    settingsRef.value = fresh
  })
  settingsRef.value = snapshot.settings
  pluginsRef.value = snapshot.plugins
  autostartRef.value = snapshot.autostart_enabled
  appVersionRef.value = snapshot.app_version
  dataDirRef.value = snapshot.data_dir
  surface.value = (await getMainSurface()) === 'control' ? 'control' : 'detail'
  await syncAlwaysOnTop()
  loaded.value = true
})

function onEscKey(event: KeyboardEvent) {
  if (event.key !== 'Escape' || event.defaultPrevented || surface.value === 'sidebar') return
  event.preventDefault()
  void closeDetail().catch(() => {})
}

onMounted(() => window.addEventListener('keydown', onEscKey))
onUnmounted(() => {
  window.removeEventListener('hashchange', syncPage)
  darkQuery.removeEventListener('change', onSchemeChange)
  window.removeEventListener('keydown', onEscKey)
  unlistenSurface?.()
})
</script>

<template>
  <template v-if="loaded">
    <div v-show="surface === 'detail' && !(skinSurfaceReady && isSkinHomePage)" class="detail-surface" :style="detailStyle" @contextmenu.prevent>
      <div class="detail-shell" :class="{ 'plugin-detail-shell': isPluginDetailPage }">
        <DetailView
          :active="surface === 'detail'"
          :always-on-top="alwaysOnTop"
          @toggle-always-on-top="toggleAlwaysOnTop"
          @minimize-window="minimizeWindow"
        />
      </div>
    </div>

    <div v-show="surface === 'control' && (!skinSurfaceReady || showNativeControl)" class="control-surface" :style="detailStyle" @contextmenu.prevent>
      <header class="control-header" data-tauri-drag-region>
        <div class="control-title" data-tauri-drag-region>
          <span class="control-mark" data-tauri-drag-region><Icon name="command" :size="15" /></span>
          <span class="control-breadcrumb" data-tauri-drag-region><strong data-tauri-drag-region>SideWeave</strong><Icon name="chevron-right" :size="13" /><small data-tauri-drag-region>控制中心</small></span>
        </div>
        <button
          v-if="showNativeControl"
          class="control-return"
          type="button"
          @click="showNativeControl = false"
        >返回皮肤控制中心</button>
        <div class="control-window-actions" :class="{ 'push-right': !showNativeControl }" data-tauri-drag-region="false">
          <Button
            variant="ghost"
            size="icon-sm"
            class="control-window-button"
            :class="{ pinned: alwaysOnTop }"
            type="button"
            :title="alwaysOnTop ? '取消置顶' : '置顶'"
            :aria-label="alwaysOnTop ? '取消窗口置顶' : '窗口置顶'"
            :aria-pressed="alwaysOnTop"
            @click="toggleAlwaysOnTop"
          >
            <Icon name="pin" :size="16" />
          </Button>
          <Button variant="ghost" size="icon-sm" class="control-window-button" type="button" aria-label="最小化控制中心" title="最小化" @click="minimizeWindow">
            <Icon name="minus" :size="17" />
          </Button>
          <Button variant="ghost" size="icon-sm" class="control-close" type="button" aria-label="关闭控制中心" title="关闭" @click="closeDetail">
            <Icon name="x" :size="17" />
          </Button>
        </div>
      </header>
      <div class="control-body">
        <ControlCenterView :active="surface === 'control'" />
      </div>
    </div>

    <SkinSurfaceHost
      v-if="settingsRef && surface !== 'sidebar'"
      :skin-id="settingsRef.appearance.skin"
      :surface="surface"
      :enabled="surface === 'control' ? !showNativeControl : isSkinHomePage"
      @show-native-ui="showNativeControl = true"
      @surface-ready="skinSurfaceReady = $event"
      :context="{
        settings: settingsRef,
        plugins: pluginsRef,
        surface,
        page,
        appVersion: appVersionRef,
        dataDir: dataDirRef
      }"
    />
  </template>
</template>

<style scoped>
.detail-surface,
.control-surface {
  width: 100%;
  height: 100%;
}

.detail-surface {
  position: relative;
  overflow: hidden;
  box-sizing: border-box;
  padding: 7px;
  background: transparent;
}

.detail-surface::before {
  content: '';
  position: absolute;
  inset: 0;
  background-image: var(--detail-image, none);
  background-size: cover;
  background-position: center;
  opacity: var(--detail-image-opacity, 1);
  pointer-events: none;
}

.detail-shell {
  position: relative;
  z-index: 1;
  width: 100%;
  height: 100%;
  overflow: hidden;
  background: transparent;
}

.detail-shell.plugin-detail-shell {
  background: transparent;
}

.control-surface {
  position: relative;
  display: flex;
  flex-direction: column;
  overflow: hidden;
  background: transparent;
  color: var(--text);
}

.control-surface::before {
  content: '';
  position: absolute;
  inset: 0;
  z-index: 0;
  background-image: var(--detail-image, none);
  background-size: cover;
  background-position: center;
  opacity: var(--detail-image-opacity, 1);
  pointer-events: none;
}

.control-surface::after {
  content: '';
  position: absolute;
  inset: 0;
  z-index: 0;
  background: var(
    --detail-shell-overlay,
    color-mix(in srgb, var(--detail-bg, var(--bg)) calc(var(--detail-panel-opacity, 1) * 100%), transparent)
  );
  pointer-events: none;
}

.control-header {
  position: relative;
  z-index: 1;
  display: flex;
  align-items: center;
  justify-content: space-between;
  min-height: 44px;
  padding: 0 12px 0 14px;
  border-bottom: 1px solid var(--border);
  background: color-mix(in srgb, var(--surface) calc(var(--component-opacity, 1) * 100%), transparent);
}

.control-title {
  display: inline-flex;
  align-items: center;
  gap: 9px;
  min-width: 0;
  color: var(--text);
  font-size: 14px;
  font-weight: 650;
}

.control-title small {
  margin-left: 5px;
  color: var(--text-muted);
  font-size: 11px;
  font-weight: 500;
}

.control-mark {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  border-radius: 7px;
  background: color-mix(in srgb, var(--primary) calc(var(--component-opacity, 1) * 18%), transparent);
  color: var(--primary);
  font-size: 10px;
  font-weight: 750;
}

.control-close {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  padding: 0;
  border: 0;
  border-radius: 8px;
  background: transparent;
  color: var(--text-muted);
}

.control-window-actions {
  display: inline-flex;
  align-items: center;
  flex: none;
  margin-left: 8px;
  gap: 2px;
}

.control-window-actions.push-right {
  margin-left: auto;
}

.control-window-button {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  padding: 0;
  border: 0;
  border-radius: 8px;
  background: transparent;
  color: var(--text-muted);
}

.control-window-button:hover,
.control-window-button.pinned {
  background: color-mix(in srgb, var(--primary) calc(var(--component-opacity, 1) * 14%), transparent);
  color: var(--primary);
}

.control-close:hover {
  background: color-mix(in srgb, var(--surface-subtle) calc(var(--component-opacity, 1) * 100%), transparent);
  color: var(--text);
}

.control-return {
  margin-left: auto;
  margin-right: 8px;
  padding: 6px 10px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: color-mix(in srgb, var(--surface-subtle) calc(var(--component-opacity, 1) * 100%), transparent);
  color: var(--text);
  font: inherit;
  font-size: 12px;
}

.control-return:hover {
  border-color: var(--primary);
  color: var(--primary);
}

.control-body {
  position: relative;
  z-index: 1;
  flex: 1;
  min-height: 0;
  overflow: hidden;
}

.control-header {
  min-height: 42px;
  padding: 0 13px;
  border-bottom: 1px solid var(--border);
  background: color-mix(in srgb, var(--background) 94%, transparent);
  backdrop-filter: blur(16px);
}
.control-title { gap: 9px; font-size: 11px; }
.control-mark { width: 25px; height: 25px; border: 1px solid color-mix(in srgb, var(--primary) 35%, var(--border)); border-radius: 6px; background: var(--primary); }
.control-breadcrumb { display: flex; align-items: center; gap: 8px; color: var(--muted-foreground); }
.control-breadcrumb strong { color: var(--foreground); font-size: 11px; font-weight: 600; }
.control-breadcrumb svg { opacity: .65; }
.control-breadcrumb small { color: var(--muted-foreground); font-size: 10px; font-weight: 450; }
.control-window-actions { gap: 3px; }
.control-window-button,
.control-close { width: 28px; height: 28px; padding: 0; border: 1px solid transparent; border-radius: 6px; background: transparent; color: var(--muted-foreground); box-shadow: none; }
.control-window-button:hover,
.control-window-button.pinned { border-color: var(--border); background: var(--muted); color: var(--foreground); }
.control-close:hover { border-color: color-mix(in srgb, var(--destructive) 30%, transparent); background: color-mix(in srgb, var(--destructive) 10%, transparent); color: var(--destructive); }
.control-return { margin-left: auto; padding: 5px 9px; border: 1px solid var(--border); border-radius: 6px; background: var(--background); color: var(--foreground); font-size: 10px; }
.control-return:hover { border-color: var(--ring); color: var(--primary); }
</style>
