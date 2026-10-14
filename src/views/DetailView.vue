<script setup lang="ts">
import { computed, onMounted, onUnmounted, ref, watch } from 'vue'
import { getCurrentWindow } from '@tauri-apps/api/window'
import { listen, type UnlistenFn } from '@tauri-apps/api/event'
import Icon from '../ui/Icon.vue'
import { closeDetail, minimizeDetail } from '../lib/api'
import SettingsView from './SettingsView.vue'
import PluginCenterView from './PluginCenterView.vue'
import SkinCenterView from './SkinCenterView.vue'
// Legacy filename retained while this component becomes the generic plugin detail host.
import PluginDetailHostView from './UserPluginHostView.vue'
import { pluginRegistryList } from '../lib/marketApi'
import type { RuntimePluginInfo } from '../lib/marketApi'
import { pluginsRef } from '../lib/store'

const page = ref<string>('')
const props = defineProps<{ active: boolean }>()
const runtimePlugins = ref<RuntimePluginInfo[]>([])
const compact = ref(false)
let unlistenCompact: UnlistenFn | null = null

function syncHash() {
  page.value = window.location.hash.replace(/^#\//, '')
}

async function refreshRuntimePlugins(force = false) {
  try {
    runtimePlugins.value = await pluginRegistryList(force)
  } catch (error) {
    console.error('刷新插件运行时注册表失败', error)
  }
}

onMounted(() => {
  syncHash()
  window.addEventListener('hashchange', syncHash)
  void refreshRuntimePlugins(false)
  void listen<boolean>('detail://compact', (event) => {
    compact.value = event.payload
  }).then((unlisten) => {
    unlistenCompact = unlisten
  })
})
watch(page, () => {
  if (runtimePlugins.value.length === 0) {
    void refreshRuntimePlugins(false)
  }
})
onUnmounted(() => {
  window.removeEventListener('hashchange', syncHash)
  unlistenCompact?.()
})

async function toggleCompact() {
  try {
    compact.value = await minimizeDetail()
  } catch (error) {
    console.error('切换详情页紧凑态失败', error)
  }
}

// 无边框窗口缩放：8 个边缘手柄 → startResizeDragging
const appWindow = getCurrentWindow()
type Dir = 'North' | 'South' | 'East' | 'West' | 'NorthEast' | 'NorthWest' | 'SouthEast' | 'SouthWest'
function beginResize(e: MouseEvent, dir: Dir) {
  e.preventDefault()
  e.stopPropagation()
  void appWindow.startResizeDragging(dir).catch(() => {})
}

// 用户插件路由：插件中心「打开」写入 #/user/<id>（page 呈现为 user/<id>），
// 侧边栏 / open_detail 直开则为 user:<id>，两种形式都归一到插件 id
const userPluginId = computed(() => {
  const p = page.value
  if (p.startsWith('user:') || p.startsWith('user/')) return p.slice(5)
  return ''
})

const runtimePlugin = computed(() =>
  runtimePlugins.value.find((item) => item.manifest.id === (userPluginId.value || page.value))
)

const pluginMeta = computed(() => {
  const id = userPluginId.value
  const pluginId = id ? `user:${id}` : page.value
  return pluginsRef.value.find(
    (item) => item.id === id || item.id === pluginId || item.id === page.value
  )
})

const isPluginDetail = computed(() =>
  Boolean(page.value) && !['settings', 'market-plugins', 'market-skins'].includes(page.value)
)

const pluginName = computed(() => {
  if (runtimePlugin.value?.manifest.name) return runtimePlugin.value.manifest.name
  return pluginMeta.value?.name ?? (userPluginId.value || page.value)
})

const pluginIcon = computed(() => runtimePlugin.value?.manifest.icon || pluginMeta.value?.icon || 'puzzle')

// 内容滚动时在头部下缘浮现渐变遮罩，柔化卡片与滚动内容的交界
const scrolled = ref(false)
function onBodyScroll(e: Event) {
  scrolled.value = (e.target as HTMLElement).scrollTop > 6
}

const emptyTitle = computed(() => '从侧边栏选择一个插件')
const emptyHint = computed(() => '点击屏幕边缘的圆环即可打开对应面板')
</script>

<template>
  <!-- 无边框缩放手柄：贴窗口四边与四角 -->
  <div class="rz n" @mousedown="beginResize($event, 'North')" />
  <div class="rz s" @mousedown="beginResize($event, 'South')" />
  <div class="rz e" @mousedown="beginResize($event, 'East')" />
  <div class="rz w" @mousedown="beginResize($event, 'West')" />
  <div class="rz nw" @mousedown="beginResize($event, 'NorthWest')" />
  <div class="rz ne" @mousedown="beginResize($event, 'NorthEast')" />
  <div class="rz sw" @mousedown="beginResize($event, 'SouthWest')" />
  <div class="rz se" @mousedown="beginResize($event, 'SouthEast')" />

  <!-- Pulse 风格详情卡：贴轨道展开的深色圆角卡片 -->
  <div class="card" :class="{ 'plugin-card': isPluginDetail, compact }">
    <header class="card-head" :class="{ scrolled }" data-tauri-drag-region>
      <div v-if="isPluginDetail" class="plugin-identity" data-tauri-drag-region>
        <span class="plugin-icon" data-tauri-drag-region>
          <Icon :name="pluginIcon" :size="15" :stroke-width="1.8" />
        </span>
        <span class="plugin-name" data-tauri-drag-region>{{ pluginName }}</span>
      </div>
      <div class="window-actions" data-tauri-drag-region="false">
        <button
          class="minimize"
          :title="compact ? '展开' : '最小化'"
          :aria-label="compact ? '展开详情卡' : '最小化详情卡'"
          @click="toggleCompact"
        >
          <Icon :name="compact ? 'chevron-down' : 'minus'" :size="15" />
        </button>
        <button class="close" title="关闭" aria-label="关闭详情卡" @click="closeDetail">
          <Icon name="x" :size="15" />
        </button>
      </div>
    </header>
    <main class="card-body" :class="{ 'plugin-body': isPluginDetail }" @scroll="onBodyScroll">
      <template v-if="page === 'settings'">
        <SettingsView />
      </template>
      <!-- 市场：插件中心 / 皮肤中心 -->
      <template v-else-if="page === 'market-plugins'">
        <PluginCenterView />
      </template>
      <template v-else-if="page === 'market-skins'">
        <SkinCenterView />
      </template>
      <template v-else-if="!isPluginDetail">
        <div class="empty">
          <div class="empty-icon">
            <Icon name="plus" :size="26" :stroke-width="1.6" />
          </div>
          <p class="empty-title">{{ emptyTitle }}</p>
          <p class="empty-hint">{{ emptyHint }}</p>
        </div>
      </template>

      <!-- 保留插件 iframe 状态；非当前插件会暂停并从渲染树隐藏。 -->
      <PluginDetailHostView
        v-show="isPluginDetail"
        :plugin-id="isPluginDetail ? (userPluginId || page) : ''"
        :active="props.active && !compact && isPluginDetail"
      />
    </main>
  </div>
</template>

<style scoped>
.card {
  position: relative;
  z-index: 1;
  display: flex;
  flex-direction: column;
  height: calc(100vh - 6px);
  margin: 3px;
  border-radius: var(--radius-lg, 18px);
  border: 1px solid var(--border);
  background: color-mix(in srgb, color-mix(in srgb, var(--surface) 84%, transparent) calc(var(--component-opacity, 1) * 100%), transparent);
  box-shadow: var(--shadow);
  overflow: hidden;
}

.plugin-card {
  background: transparent;
  box-shadow: none;
}

.card-head {
  flex: none;
  position: relative;
  z-index: 1;
  display: flex;
  align-items: center;
  justify-content: space-between;
  min-height: 40px;
  padding: 6px 10px 6px 12px;
  border-bottom: 1px solid var(--border);
}

.card.compact .card-body {
  display: none;
}

.plugin-card .card-head {
  border-bottom-color: color-mix(in srgb, var(--border) 58%, transparent);
}

.plugin-identity {
  display: flex;
  align-items: center;
  min-width: 0;
  gap: 8px;
  color: var(--text);
}

.plugin-icon {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 24px;
  height: 24px;
  flex: none;
  border-radius: 7px;
  background: color-mix(in srgb, var(--accent) 16%, transparent);
  color: var(--accent);
}

.plugin-name {
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
  font-size: 14px;
  font-weight: 600;
}

.window-actions {
  display: flex;
  align-items: center;
  flex: none;
  margin-left: auto;
  gap: 2px;
}

/* 滚动渐变遮罩：内容滚入头部下缘时渐隐 */
.card-head::after {
  content: '';
  position: absolute;
  left: 0;
  right: 0;
  top: 100%;
  height: 16px;
  background: linear-gradient(
    to bottom,
    color-mix(in srgb, var(--surface) 88%, transparent),
    transparent
  );
  opacity: 0;
  transition: opacity 0.2s ease;
  pointer-events: none;
}
.card-head.scrolled::after {
  opacity: 1;
}

.minimize,
.close {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  padding: 0;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--text-muted);
}
.minimize:hover {
  background: var(--surface-subtle);
  color: var(--text);
}
.close:hover {
  background: var(--surface-subtle);
  color: var(--text);
}

.card-body {
  flex: 1;
  min-height: 0;
  overflow: auto;
  padding: 16px 18px;
}

.plugin-body {
  padding: 0;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  height: 100%;
  width: 100%;
  min-height: 0;
}

/* 精致空状态：圆形图标托 + 主副两行文案 */
.empty {
  height: 100%;
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 6px;
  text-align: center;
}
.empty-icon {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 56px;
  height: 56px;
  margin-bottom: 8px;
  border-radius: 50%;
  border: 1px solid color-mix(in srgb, var(--border) 60%, transparent);
  background: color-mix(in srgb, var(--surface-subtle) 70%, transparent);
  color: var(--text-muted);
}
.empty-title {
  margin: 0;
  font-size: 13.5px;
  font-weight: 600;
  color: var(--text);
}
.empty-hint {
  margin: 0;
  font-size: 12px;
  color: var(--text-muted);
}
/* 无边框窗口缩放手柄：贴视口四边/四角的透明命中区 */
.rz {
  position: fixed;
  z-index: 2000;
}
.rz.n { top: 0; left: 8px; right: 8px; height: 5px; cursor: n-resize; }
.rz.s { bottom: 0; left: 8px; right: 8px; height: 5px; cursor: s-resize; }
.rz.e { right: 0; top: 8px; bottom: 8px; width: 5px; cursor: e-resize; }
.rz.w { left: 0; top: 8px; bottom: 8px; width: 5px; cursor: w-resize; }
.rz.nw { top: 0; left: 0; width: 12px; height: 12px; cursor: nw-resize; }
.rz.ne { top: 0; right: 0; width: 12px; height: 12px; cursor: ne-resize; }
.rz.sw { bottom: 0; left: 0; width: 12px; height: 12px; cursor: sw-resize; }
.rz.se { bottom: 0; right: 0; width: 12px; height: 12px; cursor: se-resize; }
</style>
