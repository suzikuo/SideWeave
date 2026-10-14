<script setup lang="ts">
import { computed, nextTick, onMounted, onUnmounted, ref, watch } from 'vue'
import { invoke } from '@tauri-apps/api/core'
import { getCurrent, onOpenUrl } from '@tauri-apps/plugin-deep-link'
import HotkeyRecorder from '../ui/HotkeyRecorder.vue'
import Icon from '../ui/Icon.vue'
import RangeSlider from '../ui/RangeSlider.vue'
import Toast from '../ui/Toast.vue'
import {
  getState,
  openDataDir,
  restartApp,
  setPluginEnabled,
  setPluginPermission,
  setPluginSidebarVisible,
  setPluginOrder,
  setSetting,
  toggleSidebar,
  updateCheck,
  updateInstall,
  updateIgnoreSet,
  backupExport,
  backupImport,
  getPerformanceStatus
} from '../lib/api'
import { marketInstalled, pluginRegistryList } from '../lib/marketApi'
import type { RuntimePluginInfo } from '../lib/marketApi'
import { applySkinWithRecommendedOpacity, OPACITY_SETTING_KEYS, resolveSkinOpacityDefaults, type InstalledSkinAppearance } from '../lib/skinAppearance'
import PluginCenterView from './PluginCenterView.vue'
import SkinCenterView from './SkinCenterView.vue'
import {
  appVersionRef,
  autostartRef,
  dataDirRef,
  pluginsRef,
  settingsRef
} from '../lib/store'
import type { PerformanceProfile, PerformanceStatusPayload, Settings } from '../lib/types'

// ---------- get_system_info 返回结构（对应 src-tauri lib.rs 的 SystemInfo，本地声明不改 lib/） ----------

interface MonitorInfo {
  x: number
  y: number
  w: number
  h: number
  scale: number
}

interface SystemInfo {
  app_version: string
  os_name: string
  arch: string
  uptime_secs: number
  work_area: MonitorInfo
  monitors: MonitorInfo[]
  plugins_total: number
  plugins_enabled: number
}

// ---------- 左侧导航 ----------

type Section = 'overview' | 'general' | 'appearance' | 'notifications' | 'plugins' | 'market-plugins' | 'market-skins' | 'about'
const section = ref<Section>('overview')
const sections: { id: Section; label: string; icon: string }[] = [
  { id: 'overview', label: '概览', icon: 'layout-dashboard' },
  { id: 'general', label: '通用', icon: 'settings' },
  { id: 'appearance', label: '外观', icon: 'palette' },
  { id: 'notifications', label: '通知', icon: 'bell' },
  { id: 'plugins', label: '插件', icon: 'blocks' },
  { id: 'market-plugins', label: '插件中心', icon: 'blocks' },
  { id: 'market-skins', label: '皮肤中心', icon: 'palette' },
  { id: 'about', label: '关于', icon: 'info' }
]

const toast = ref('')
const toastKind = ref<'info' | 'error' | 'success'>('info')
function showToast(message: string, kind: 'info' | 'error' | 'success' = 'info') {
  toastKind.value = kind
  toast.value = ''
  requestAnimationFrame(() => {
    toast.value = message
  })
}

const ACCENT_SWATCHES = ['#006874', '#006A6A', '#4F6354', '#7A5900', '#6750A4']
const isWindows = /Windows/i.test(navigator.userAgent)

const settings = computed<Settings | null>(() => settingsRef.value)
const props = defineProps<{ active: boolean }>()
const permissionPlugins = ref<RuntimePluginInfo[]>([])
const permissionSaving = ref<Record<string, boolean>>({})
const openPermissionPluginId = ref<string | null>(null)
const permissionPluginsLoading = ref(false)

interface PluginPermissionDisplay {
  id: string
  label: string
  description: string
  enforceable: boolean
}

const permissionDefinitions: Record<string, Omit<PluginPermissionDisplay, 'id'>> = {
  storage: { label: '插件设置存储', description: '读写本插件独立的键值存储', enforceable: true },
  'notification.system': { label: 'Windows 原生通知', description: '发送由 Windows 显示的系统通知', enforceable: true },
  'notification.desktop': { label: '宿主桌面通知', description: '在桌面右下角显示宿主样式通知', enforceable: true },
  'notification.sidebar': { label: '侧边栏气泡', description: '在侧边栏旁显示消息气泡', enforceable: true },
  'event.publish': { label: '发布宿主事件', description: '向宿主事件总线发布消息', enforceable: true },
  'filesystem.read': { label: '读取插件文件', description: '读取宿主托管的本插件私有文件', enforceable: true },
  'filesystem.write': { label: '写入插件文件', description: '写入或删除宿主托管的本插件私有文件', enforceable: true },
  'filesystem.external_read': { label: '读取外部文件', description: '读取插件通过文件选择器选中的文件', enforceable: true },
  clipboard: { label: '剪贴板', description: '读取或写入系统剪贴板', enforceable: true },
  'process.spawn': { label: '启动进程', description: '通过宿主 API 启动外部程序', enforceable: true },
  'network.listen': { label: '监听网络端口', description: '原生插件可绕过宿主 API 直接访问网络，此开关暂不能拦截', enforceable: false },
  'network.connect': { label: '连接网络', description: '原生插件可绕过宿主 API 直接访问网络，此开关暂不能拦截', enforceable: false },
  'window.create': { label: '创建原生窗口', description: '通过宿主窗口 API 创建插件窗口', enforceable: true },
  'window.manage': { label: '控制插件窗口', description: '通过宿主窗口 API 显示、移动或调整插件窗口', enforceable: true },
  'shortcut.global': { label: '全局快捷键', description: '注册或注销系统级快捷键', enforceable: true },
  'system.open_external': { label: '打开外部链接', description: '使用系统默认浏览器打开网页链接', enforceable: true },
  'system.file_picker': { label: '文件选择器', description: '打开系统文件选择对话框', enforceable: true },
  'system.icon': { label: '提取文件图标', description: '请求宿主读取文件图标', enforceable: true }
}

const pluginPermissionsById = computed<Record<string, { plugin: RuntimePluginInfo; permissions: PluginPermissionDisplay[] }>>(() => {
  const result: Record<string, { plugin: RuntimePluginInfo; permissions: PluginPermissionDisplay[] }> = {}
  for (const plugin of permissionPlugins.value) {
    const requested = plugin.manifest.capabilities.flatMap((permission) =>
      permission === 'notification'
        ? ['notification.system', 'notification.desktop', 'notification.sidebar']
        : [permission]
    )
    result[plugin.manifest.id] = {
      plugin,
      permissions: [...new Set(requested)].map((permission) => ({
        id: permission,
        ...(permissionDefinitions[permission] ?? {
          label: permission,
          description: '插件清单请求的权限',
          enforceable: false
        })
      }))
    }
  }
  return result
})

async function loadPermissionPlugins() {
  permissionPluginsLoading.value = true
  try {
    permissionPlugins.value = await pluginRegistryList(true)
  } catch (error) {
    showToast(String(error), 'error')
  } finally {
    permissionPluginsLoading.value = false
  }
}

watch(section, (value) => {
  if (value === 'plugins') void loadPermissionPlugins()
})

function togglePluginPermissions(pluginId: string) {
  openPermissionPluginId.value = openPermissionPluginId.value === pluginId ? null : pluginId
}

function permissionIsGranted(pluginId: string, permission: string) {
  const overrides = settings.value?.plugins.permission_grants?.[pluginId]
  if (overrides && Object.prototype.hasOwnProperty.call(overrides, permission)) {
    return overrides[permission]
  }
  return false
}

async function changePluginPermission(pluginId: string, permission: string, granted: boolean) {
  const key = `${pluginId}:${permission}`
  permissionSaving.value[key] = true
  try {
    settingsRef.value = await setPluginPermission(pluginId, permission, granted)
  } catch (error) {
    showToast(String(error), 'error')
  } finally {
    permissionSaving.value[key] = false
  }
}

// ---------- 系统信息（概览 / 关于诊断共用） ----------

const sysInfo = ref<SystemInfo | null>(null)
let sysTimer: number | undefined
let initialLoadPromise: Promise<void> | undefined
let initialLoadDone = false

async function loadSystemInfo() {
  try {
    sysInfo.value = await invoke<SystemInfo>('get_system_info')
  } catch {
    // 静默失败：占位符展示，不打断设置主流程（30s 后会重试）
  }
}

/** 运行时长格式化为"x时x分" */
function fmtUptime(secs: number): string {
  const h = Math.floor(secs / 3600)
  const m = Math.floor((secs % 3600) / 60)
  if (h <= 0 && m <= 0) return '不足1分'
  return `${h}时${m}分`
}

/** 缩放比：1.5 → "1.5x"，1 → "1x" */
const fmtScale = (s: number) => `${Number(s.toFixed(2))}x`

const workAreaText = computed(() => {
  const wa = sysInfo.value?.work_area
  return wa ? `${wa.w}×${wa.h}` : '—'
})
const workAreaScale = computed(() => (sysInfo.value ? `@ ${fmtScale(sysInfo.value.work_area.scale)}` : ''))

/** 插件统计：优先取后端统计，快照未就绪时回退到本地 pluginsRef */
const pluginStat = computed(() => {
  const total = sysInfo.value?.plugins_total ?? pluginsRef.value.length
  const enabled =
    sysInfo.value?.plugins_enabled ?? pluginsRef.value.filter((p) => p.enabled).length
  return {
    total,
    enabled,
    percent: total > 0 ? Math.round((enabled / total) * 100) : 0
  }
})

// ---------- 性能与资源运行模式 ----------

const perfStatus = ref<PerformanceStatusPayload | null>(null)

const profileLabels: Record<PerformanceProfile, string> = {
  eco: '省资源',
  balanced: '平衡',
  performance: '极速',
  auto: '自动'
}

async function loadPerfStatus() {
  try {
    perfStatus.value = await getPerformanceStatus()
  } catch (e) {
    console.error('获取性能状态失败', e)
  }
}

const perfEffectiveLabel = computed(() => {
  if (!perfStatus.value) return '—'
  const { userProfile, effectiveProfile } = perfStatus.value
  const eff = profileLabels[effectiveProfile] || effectiveProfile
  if (userProfile === 'auto') {
    return `自动 (${eff})`
  }
  return profileLabels[userProfile] || userProfile
})

const perfMemSubText = computed(() => {
  const mem = perfStatus.value?.memory
  if (!mem) return '自适应调度'
  return `可用 ${(mem.availPhysBytes / (1024 ** 3)).toFixed(1)} GB · 负荷 ${mem.memoryLoadPct}%`
})

const perfDesc = computed(() => {
  if (!perfStatus.value) return '自适应调度策略'
  const { userProfile, effectiveProfile, memory } = perfStatus.value
  const effLabel = profileLabels[effectiveProfile] || effectiveProfile
  const memText = memory
    ? `（可用 ${(memory.availPhysBytes / (1024 ** 3)).toFixed(1)} GB / 负荷 ${memory.memoryLoadPct}%）`
    : ''
  if (userProfile === 'auto') {
    return `根据系统可用内存与负荷动态决策 · 当前生效：${effLabel}${memText}`
  }
  if (userProfile === 'performance') {
    return `窗口后台静默预热 · 双窗口常驻保活 · 极速 120ms 响应${memText}`
  }
  if (userProfile === 'balanced') {
    return `侧边栏与悬浮预览保活 · 详情卡闲置 30s 销毁回收${memText}`
  }
  return `按需创建 WebView · 关闭即刻销毁 · 极低资源占用${memText}`
})

async function changePerformanceProfile(profile: PerformanceProfile) {
  await change('general', 'performance_profile', profile)
  await loadPerfStatus()
}

// ---------- 皮肤选项：内置 + 市场已安装的用户皮肤 ----------

const opacityControls = [
  { key: 'sidebar_panel_opacity', label: '侧边栏面板不透明度', description: '只调面板底色，图标与文字保持清晰' },
  { key: 'sidebar_image_opacity', label: '侧边栏背景图片不透明度', description: '单独调节皮肤背景图片' },
  { key: 'detail_panel_opacity', label: '详情页面板不透明度', description: '只调面板底色，不影响详情内容' },
  { key: 'control_panel_opacity', label: '控制中心面板不透明度', description: '与详情页面板独立调节' },
  { key: 'detail_image_opacity', label: '详情页与控制中心背景图片', description: '单独调节皮肤背景图片的可见度' },
  { key: 'component_opacity', label: '卡片与组件底色不透明度', description: '只调卡片和控件表面，不会淡化文字与图标' }
] as const
const userSkins = ref<InstalledSkinAppearance[]>([])
const activeSkinId = computed(() => settingsRef.value?.appearance.skin ?? 'default')
const activeSkinName = computed(() => {
  if (!activeSkinId.value.startsWith('user:')) return '默认皮肤'
  const id = activeSkinId.value.slice('user:'.length)
  return userSkins.value.find((skin) => skin.id === id)?.name ?? id
})
const activeOpacityDefaults = computed(() => resolveSkinOpacityDefaults(activeSkinId.value, userSkins.value))
const opacityResetting = ref(false)
const skinDefaultsReady = ref(false)
const canResetOpacity = computed(() =>
  !opacityResetting.value && (!activeSkinId.value.startsWith('user:') || skinDefaultsReady.value)
)
const skinOptions = computed(() => [
  { id: 'default', label: '默认' },
  ...userSkins.value.map((s) => ({ id: `user:${s.id}`, label: s.name }))
])

async function reloadUserSkins() {
  skinDefaultsReady.value = false
  try {
    userSkins.value = (await marketInstalled()).skins.map(({ id, name, appearanceDefaults }) => ({
      id,
      name,
      appearanceDefaults
    }))
  } catch {
    // 云端/本地读取失败时保留现有选项
  } finally {
    skinDefaultsReady.value = true
  }
}

async function resetOpacitySettings() {
  if (opacityResetting.value || !settingsRef.value) return
  opacityResetting.value = true
  try {
    const defaults = activeOpacityDefaults.value
    const skinName = activeSkinName.value
    for (const key of OPACITY_SETTING_KEYS) {
      settingsRef.value = await setSetting('appearance', key, defaults[key])
    }
    showToast(`已恢复“${skinName}”的透明度推荐值`, 'success')
  } catch (e) {
    showToast(String(e), 'error')
  } finally {
    opacityResetting.value = false
  }
}

watch(activeSkinId, (skinId, previousSkinId) => {
  if (skinId !== previousSkinId && skinId.startsWith('user:')) void reloadUserSkins()
})

// ---------- 主题生效标记（本地 matchMedia，不写 lib/） ----------

const darkQuery = window.matchMedia('(prefers-color-scheme: dark)')
const systemDark = ref(darkQuery.matches)
function onSchemeChange(e: MediaQueryListEvent) {
  systemDark.value = e.matches
}
/** 实际生效主题：显式取所选，system 时跟随系统 */
const effectiveMode = computed(() => {
  const mode = settingsRef.value?.appearance.theme_mode
  if (!mode || mode === 'system') return systemDark.value ? 'dark' : 'light'
  return mode
})
const effectiveModeLabel = computed(() => (effectiveMode.value === 'dark' ? '深色' : '浅色'))

const themeModeLabel = computed(() => {
  const m = settingsRef.value?.appearance.theme_mode
  return m === 'dark' ? '深色' : m === 'light' ? '浅色' : '跟随系统'
})

// ---------- 滑块气泡格式化 ----------

const fmtMs = (v: number) => `${v} ms`
const fmtPx = (v: number) => `${v} px`
const fmtPercent = (v: number) => `${Math.round(v * 100)}%`

onMounted(() => {
  darkQuery.addEventListener('change', onSchemeChange)
  document.addEventListener('visibilitychange', onDocumentVisibilityChange)
})
onUnmounted(() => {
  darkQuery.removeEventListener('change', onSchemeChange)
  document.removeEventListener('visibilitychange', onDocumentVisibilityChange)
  if (sysTimer !== undefined) window.clearInterval(sysTimer)
})

function syncSystemPolling() {
  if (sysTimer !== undefined) {
    window.clearInterval(sysTimer)
    sysTimer = undefined
  }
  if (!props.active || document.hidden) return
  loadControlDataOnce()
  void loadSystemInfo()
  void loadPerfStatus()
  sysTimer = window.setInterval(() => {
    void loadSystemInfo()
    void loadPerfStatus()
  }, 30_000)
}

function onDocumentVisibilityChange() {
  syncSystemPolling()
}

watch(() => props.active, syncSystemPolling, { immediate: true })

function loadControlDataOnce() {
  if (initialLoadDone || initialLoadPromise) return
  initialLoadPromise = (async () => {
    void reloadUserSkins()
    try {
      const snap = await getState()
      pluginsRef.value = snap.plugins
      autostartRef.value = snap.autostart_enabled
      appVersionRef.value = snap.app_version
      dataDirRef.value = snap.data_dir
      initialLoadDone = true
    } catch (e) {
      showToast(String(e), 'error')
    } finally {
      initialLoadPromise = undefined
    }
  })()
}

async function change(category: string, key: string, value: unknown) {
  try {
    if (category === 'appearance' && key === 'skin') {
      const skinId = String(value)
      if (settingsRef.value?.appearance.skin === skinId) return
      settingsRef.value = await applySkinWithRecommendedOpacity(skinId, userSkins.value)
    } else {
      settingsRef.value = await setSetting(category, key, value)
    }
    if (category === 'general' && key === 'run_on_startup') {
      autostartRef.value = Boolean(value)
    }
  } catch (e) {
    showToast(String(e), 'error')
  }
}

async function togglePlugin(id: string, enabled: boolean) {
  try {
    pluginsRef.value = await setPluginEnabled(id, enabled)
  } catch (e) {
    showToast(String(e), 'error')
  }
}

async function togglePluginSidebar(id: string, visible: boolean) {
  try {
    pluginsRef.value = await setPluginSidebarVisible(id, visible)
  } catch (e) {
    showToast(String(e), 'error')
  }
}

async function movePlugin(id: string, dir: -1 | 1) {
  const ids = pluginsRef.value.map((p) => p.id)
  const knownIds = new Set(ids)
  // Drop retired plugins left in old settings, such as `gateway`.
  const order = (settingsRef.value?.plugins.order ?? []).filter((pluginId) =>
    knownIds.has(pluginId)
  )
  // 把当前完整顺序补齐（order 缺省的按现有展示顺序追加）
  for (const pid of ids) if (!order.includes(pid)) order.push(pid)
  const idx = order.indexOf(id)
  const target = idx + dir
  if (idx === -1 || target < 0 || target >= order.length) return
  ;[order[idx], order[target]] = [order[target], order[idx]]
  try {
    pluginsRef.value = await setPluginOrder(order)
    if (settingsRef.value) {
      settingsRef.value = {
        ...settingsRef.value,
        plugins: { ...settingsRef.value.plugins, order }
      }
    }
  } catch (e) {
    showToast(String(e), 'error')
  }
}

// ---------- 快捷操作 ----------

async function onOpenDataDir() {
  try {
    await openDataDir()
    showToast('已打开数据目录', 'success')
  } catch (e) {
    showToast(String(e), 'error')
  }
}

// ---------- 在线更新 ----------

type UpdateState = 'idle' | 'checking' | 'available' | 'downloading' | 'latest'
const updateState = ref<UpdateState>('idle')
const updateVersion = ref('')
/** 被忽略的版本号（空 = 无） */
const ignoredVersion = ref('')

async function onUpdateAction() {
  if (updateState.value === 'available') {
    updateState.value = 'downloading'
    try {
      await updateInstall()
      // 安装器拉起后应用会自行退出；正常不会执行到这里
      showToast('更新完成，应用将重启', 'success')
    } catch (e) {
      showToast(String(e), 'error')
      updateState.value = 'available'
    }
    return
  }
  updateState.value = 'checking'
  try {
    const info = await updateCheck()
    if (info.available) {
      updateVersion.value = info.version
      ignoredVersion.value = ''
      updateState.value = 'available'
    } else {
      // 被忽略的版本会以 available:false + version≠current 的形式返回
      ignoredVersion.value = info.version !== info.currentVersion ? info.version : ''
      updateState.value = 'latest'
    }
  } catch (e) {
    showToast(String(e), 'error')
    updateState.value = 'idle'
  }
}

/** 忽略此版本：检查更新时不再提示（直至新版本发布） */
async function ignoreVersion() {
  try {
    await updateIgnoreSet(updateVersion.value)
    ignoredVersion.value = updateVersion.value
    updateState.value = 'latest'
    showToast(`已忽略 v${updateVersion.value}`, 'success')
  } catch (e) {
    showToast(String(e), 'error')
  }
}

/** 撤销忽略：恢复对该版本的更新提示 */
async function unignoreVersion() {
  try {
    await updateIgnoreSet('')
    ignoredVersion.value = ''
    showToast('已撤销忽略', 'success')
    await onUpdateAction()
  } catch (e) {
    showToast(String(e), 'error')
  }
}

const backupBusy = ref(false)

async function onBackupExport() {
  if (backupBusy.value) return
  backupBusy.value = true
  try {
    const path = await backupExport()
    showToast(path ? `已备份到 ${path}` : '已取消', path ? 'success' : 'info')
  } catch (e) {
    showToast(String(e), 'error')
  } finally {
    backupBusy.value = false
  }
}

async function onBackupImport() {
  if (backupBusy.value) return
  if (!window.confirm('导入备份会覆盖当前全部设置与插件数据，确定继续？')) return
  backupBusy.value = true
  try {
    await backupImport()
    // 导入成功后应用会自动重启；正常不会执行到这里
    showToast('备份已导入，应用将重启', 'success')
  } catch (e) {
    showToast(String(e), 'error')
    backupBusy.value = false
  }
}

async function onRestartApp() {
  try {
    showToast('正在重启应用…', 'info')
    await restartApp()
  } catch (e) {
    showToast(String(e), 'error')
  }
}

async function onToggleSidebar() {
  try {
    await toggleSidebar()
    showToast('已切换侧边栏显示 / 隐藏', 'info')
  } catch (e) {
    showToast(String(e), 'error')
  }
}

// ---------- 全局热键（显示 / 隐藏侧边栏） ----------

/** 热键提交：Rust 校验并即时重注册全局热键；失败时 settingsRef 未变，自动回显旧值 */
async function changeHotkey(value: string) {
  try {
    settingsRef.value = await setSetting('shortcuts', 'toggle_sidebar', value)
    showToast('热键已更新', 'success')
  } catch (e) {
    showToast(String(e), 'error')
  }
}

const HOTKEY_LABELS: Record<string, string> = {
  ctrl: 'Ctrl',
  alt: 'Alt',
  shift: 'Shift',
  meta: 'Meta',
  space: 'Space'
}
/** 键帽文案（与 HotkeyRecorder 展示一致） */
function hotkeyLabel(seg: string): string {
  const lower = seg.trim().toLowerCase()
  if (!lower) return ''
  if (HOTKEY_LABELS[lower]) return HOTKEY_LABELS[lower]
  if (lower.length === 1 || /^f\d{1,2}$/.test(lower)) return lower.toUpperCase()
  return lower.charAt(0).toUpperCase() + lower.slice(1)
}
const toggleHotkeySegs = computed(() =>
  (settingsRef.value?.shortcuts.toggle_sidebar ?? '')
    .split('+')
    .map((s) => s.trim())
    .filter(Boolean)
)

let unlistenDeepLinks: (() => void) | null = null

async function routeInstallDeepLinks(urls: string[]) {
  for (const raw of urls) {
    const match = /^sideweave:\/\/install\/(plugin|skin)\/([a-z0-9_-]+)\/?$/.exec(raw)
    if (!match) continue
    const kind = match[1] as 'plugin' | 'skin'
    const id = match[2]
    section.value = kind === 'plugin' ? 'market-plugins' : 'market-skins'
    await nextTick()
    window.dispatchEvent(new CustomEvent('sideweave:market-install', { detail: { kind, id } }))
  }
}

onMounted(async () => {
  unlistenDeepLinks = await onOpenUrl((urls) => { void routeInstallDeepLinks(urls) }).catch(() => null)
  const startUrls = await getCurrent().catch(() => null)
  if (startUrls?.length) await routeInstallDeepLinks(startUrls)
})
onUnmounted(() => unlistenDeepLinks?.())

</script>

<template>
  <div v-if="settings" class="settings">
    <!-- 左侧导航：品牌块 + 分区导航 + 版本脚注，随面板滚动保持吸顶 -->
    <aside class="nav">
      <div class="brand">
        <span class="brand-mark">AT</span>
        <span class="brand-text">
          <strong>SideWeave</strong>
          <small>控制中心</small>
        </span>
      </div>
      <nav class="nav-list" aria-label="控制中心导航">
        <button
          v-for="s in sections"
          :key="s.id"
          class="nav-item"
          :class="{ active: section === s.id }"
          @click="section = s.id"
        >
          <Icon :name="s.icon" :size="15" />
          <span>{{ s.label }}</span>
        </button>
      </nav>
      <div class="nav-foot">
        <span>v{{ appVersionRef || '—' }}</span>
        <span
          class="foot-dot"
          :class="effectiveMode"
          :title="`实际生效主题：${effectiveModeLabel}`"
        />
      </div>
    </aside>

    <div class="pane">
      <!-- 概览 -->
      <template v-if="section === 'overview'">
        <section>
          <h2>概览</h2>

          <!-- 应用卡片 -->
          <div class="app-card">
            <span class="brand-mark lg">AT</span>
            <div class="app-meta">
              <strong>SideWeave</strong>
              <small>Rust + Tauri 桌面侧边栏</small>
            </div>
            <div class="app-badges">
              <span class="badge accent">v{{ appVersionRef || '—' }}</span>
              <span class="badge">Rust</span>
              <span class="badge">Tauri</span>
            </div>
          </div>

          <!-- 系统信息网格 -->
          <h3 class="group-label">系统信息</h3>
          <div class="stat-grid">
            <div class="stat-card">
              <span class="stat-label"><Icon name="monitor" :size="13" /> 操作系统</span>
              <strong>{{ sysInfo?.os_name ?? '—' }}</strong>
            </div>
            <div class="stat-card">
              <span class="stat-label"><Icon name="cpu" :size="13" /> 系统架构</span>
              <strong>{{ sysInfo?.arch ?? '—' }}</strong>
            </div>
            <div class="stat-card">
              <span class="stat-label"><Icon name="clock" :size="13" /> 运行时长</span>
              <strong>{{ sysInfo ? fmtUptime(sysInfo.uptime_secs) : '—' }}</strong>
              <small class="stat-sub">每 30 秒自动刷新</small>
            </div>
            <div class="stat-card">
              <span class="stat-label"><Icon name="layout-dashboard" :size="13" /> 工作区</span>
              <strong>{{ workAreaText }}</strong>
              <small class="stat-sub">{{ workAreaScale }}</small>
            </div>
            <div class="stat-card" style="cursor: pointer" title="点击前往通用设置调整" @click="section = 'general'">
              <span class="stat-label"><Icon name="zap" :size="13" /> 运行模式</span>
              <strong>{{ perfEffectiveLabel }}</strong>
              <small class="stat-sub">{{ perfMemSubText }}</small>
            </div>
          </div>

          <!-- 显示器列表 -->
          <h3 class="group-label">显示器（{{ sysInfo?.monitors.length ?? 0 }}）</h3>
          <div v-if="sysInfo && sysInfo.monitors.length" class="monitor-list">
            <div
              v-for="(m, i) in sysInfo.monitors"
              :key="`${m.x}-${m.y}-${m.w}x${m.h}`"
              class="monitor-chip"
              :title="`位置 (${m.x}, ${m.y})`"
            >
              <span class="monitor-idx">{{ i + 1 }}</span>
              <span>{{ m.w }}×{{ m.h }}</span>
              <span class="monitor-scale">@ {{ fmtScale(m.scale) }}</span>
            </div>
          </div>
          <div v-else class="card monitor-empty">暂无显示器信息</div>

          <!-- 插件统计 -->
          <h3 class="group-label">插件</h3>
          <button
            class="stat-card wide plugin-stat"
            type="button"
            title="前往插件管理"
            @click="section = 'plugins'"
          >
            <span class="stat-label"><Icon name="blocks" :size="13" /> 插件统计</span>
            <strong>{{ pluginStat.enabled }} / {{ pluginStat.total }}</strong>
            <small class="stat-sub">已启用 / 总数</small>
            <span class="bar"><span class="bar-fill" :style="{ width: `${pluginStat.percent}%` }" /></span>
          </button>

          <!-- 快捷操作 -->
          <h3 class="group-label">快捷操作</h3>
          <div class="quick-list">
            <button class="quick-btn" type="button" @click="onOpenDataDir">
              <Icon name="folder" :size="15" />
              <span>打开数据目录</span>
            </button>
            <button class="quick-btn" type="button" @click="onRestartApp">
              <Icon name="refresh" :size="15" />
              <span>重启应用</span>
            </button>
            <button class="quick-btn" type="button" @click="onToggleSidebar">
              <Icon name="eye-off" :size="15" />
              <span>显示 / 隐藏侧边栏</span>
            </button>
          </div>
        </section>
      </template>

      <!-- 通用 -->
      <template v-else-if="section === 'general'">
        <section>
          <h2>通用</h2>
          <div class="card row-card">
            <div class="row-main">
              <div class="row-title">开机自启</div>
              <div class="row-desc">登录 Windows 后自动启动 SideWeave</div>
            </div>
            <input
              type="checkbox"
              class="switch"
              role="switch"
              aria-label="开机自启"
              :checked="autostartRef"
              @change="change('general', 'run_on_startup', ($event.target as HTMLInputElement).checked)"
            />
          </div>
          <div class="card row-card">
            <div class="row-main">
              <div class="row-title">悬停展开侧边栏</div>
              <div class="row-desc">鼠标移到屏幕边缘时自动展开</div>
            </div>
            <input
              type="checkbox"
              class="switch"
              role="switch"
              aria-label="悬停展开侧边栏"
              :checked="settings.general.enable_mouse_hover"
              @change="change('general', 'enable_mouse_hover', ($event.target as HTMLInputElement).checked)"
            />
          </div>
          <div class="card row-card">
            <div class="row-main">
              <div class="row-title">自动隐藏延迟</div>
              <div class="row-desc">鼠标离开后 {{ settings.general.auto_hide_delay }} ms 收起</div>
            </div>
            <RangeSlider
              :model-value="settings.general.auto_hide_delay"
              :min="0"
              :max="10000"
              :step="100"
              :format="fmtMs"
              aria-label="自动隐藏延迟"
              @commit="(v) => change('general', 'auto_hide_delay', v)"
            />
          </div>
          <div class="card row-card">
            <div class="row-main">
              <div class="row-title">悬停预览延迟</div>
              <div class="row-desc">鼠标悬停插件图标 {{ settings.general.hover_preview_delay ?? 350 }} ms 后显示卡片</div>
            </div>
            <RangeSlider
              :model-value="settings.general.hover_preview_delay ?? 350"
              :min="50"
              :max="2000"
              :step="50"
              :format="fmtMs"
              aria-label="悬停预览延迟"
              @commit="(v) => change('general', 'hover_preview_delay', v)"
            />
          </div>
          <div class="card row-card">
            <div class="row-main">
              <div class="row-title">运行模式</div>
              <div class="row-desc">{{ perfDesc }}</div>
            </div>
            <div class="segmented">
              <button
                v-for="p in ([
                  { id: 'eco', label: '省资源' },
                  { id: 'balanced', label: '平衡' },
                  { id: 'performance', label: '极速' },
                  { id: 'auto', label: '自动' }
                ] as const)"
                :key="p.id"
                :class="{ on: (settings.general.performance_profile ?? 'balanced') === p.id }"
                :aria-label="`运行模式：${p.label}`"
                @click="changePerformanceProfile(p.id)"
              >
                {{ p.label }}
              </button>
            </div>
          </div>
          <div class="card row-card">
            <div class="row-main">
              <div class="row-title">显示 / 隐藏侧边栏热键</div>
              <div class="row-desc">全局热键，修改后立即生效</div>
            </div>
            <HotkeyRecorder
              :model-value="settings.shortcuts.toggle_sidebar"
              @commit="changeHotkey"
            />
          </div>
        </section>
      </template>

      <!-- 外观 -->
      <template v-else-if="section === 'appearance'">
        <section>
          <h2>外观</h2>
          <div class="card row-card">
            <div class="row-main">
              <div class="row-title">界面缩放</div>
              <div class="row-desc">调整宿主窗口和插件页面的显示大小</div>
            </div>
            <RangeSlider
              :model-value="settings.appearance.interface_scale"
              :min="0.7"
              :max="1.6"
              :step="0.01"
              :format="fmtPercent"
              aria-label="界面缩放"
              @commit="(v) => change('appearance', 'interface_scale', v)"
            />
          </div>
          <div class="card row-card">
            <div class="row-main">
              <div class="row-title">主题</div>
              <div class="row-desc">
                {{
                  settings.appearance.theme_mode === 'system'
                    ? `系统当前生效：${effectiveModeLabel}`
                    : `当前生效：${effectiveModeLabel}`
                }}
              </div>
            </div>
            <div class="segmented">
              <button
                v-for="m in (['light', 'dark', 'system'] as const)"
                :key="m"
                :class="{ on: settings.appearance.theme_mode === m }"
                :aria-label="`主题：${m === 'light' ? '浅色' : m === 'dark' ? '深色' : '跟随系统'}`"
                @click="change('appearance', 'theme_mode', m)"
              >
                {{ m === 'light' ? '浅色' : m === 'dark' ? '深色' : '跟随系统' }}
                <!-- system 模式下用小圆点标注本地 matchMedia 探测到的实际生效主题 -->
                <span
                  v-if="m === 'system' && settings.appearance.theme_mode === 'system'"
                  class="mode-dot"
                  :class="effectiveMode"
                  :title="`系统当前生效：${effectiveModeLabel}`"
                />
              </button>
            </div>
          </div>
          <!-- 皮肤：表现形式，与主题正交，每个皮肤自带浅/深 -->
          <div class="card row-card">
            <div class="row-main">
              <div class="row-title">皮肤</div>
              <div class="row-desc">界面表现形式，与深浅主题独立组合</div>
            </div>
            <div class="segmented">
              <button
                v-for="s in skinOptions"
                :key="s.id"
                :class="{ on: settings.appearance.skin === s.id }"
                :aria-label="`皮肤：${s.label}`"
                @click="change('appearance', 'skin', s.id)"
              >
                {{ s.label }}
              </button>
            </div>
          </div>
          <div class="card row-card">
            <div class="row-main">
              <div class="row-title">强调色</div>
            </div>
            <div class="swatches">
              <button
                v-for="c in ACCENT_SWATCHES"
                :key="c"
                class="swatch"
                :style="{ background: c }"
                :class="{ on: settings.appearance.accent_color.toUpperCase() === c.toUpperCase() }"
                :aria-label="`强调色 ${c}`"
                @click="change('appearance', 'accent_color', c)"
              />
              <input
                type="color"
                class="swatch custom"
                :value="settings.appearance.accent_color"
                aria-label="自定义强调色"
                @change="change('appearance', 'accent_color', ($event.target as HTMLInputElement).value)"
              />
            </div>
          </div>
          <div class="card row-card">
            <div class="row-main">
              <div class="row-title">停靠位置</div>
            </div>
            <div class="segmented">
              <button
                v-for="p in (['left', 'right', 'top'] as const)"
                :key="p"
                :class="{ on: settings.appearance.sidebar_position === p }"
                :aria-label="`停靠位置：${p === 'left' ? '左侧' : p === 'right' ? '右侧' : '顶部'}`"
                @click="change('appearance', 'sidebar_position', p)"
              >
                {{ p === 'left' ? '左侧' : p === 'right' ? '右侧' : '顶部' }}
              </button>
            </div>
          </div>
          <div class="card row-card">
            <div class="row-main">
              <div class="row-title">轨道宽度</div>
              <div class="row-desc">{{ settings.appearance.collapsed_width }} px</div>
            </div>
            <RangeSlider
              :model-value="settings.appearance.collapsed_width"
              :min="56"
              :max="96"
              :step="2"
              :format="fmtPx"
              aria-label="轨道宽度"
              @commit="(v) => change('appearance', 'collapsed_width', v)"
            />
          </div>
          <div class="card row-card">
            <div class="row-main">
              <div class="row-title">边缘细缝宽度</div>
              <div class="row-desc">{{ settings.appearance.peek_width }} px</div>
            </div>
            <RangeSlider
              :model-value="settings.appearance.peek_width"
              :min="0"
              :max="10"
              :step="1"
              :format="fmtPx"
              aria-label="边缘细缝宽度"
              @commit="(v) => change('appearance', 'peek_width', v)"
            />
          </div>
          <div class="card row-card opacity-reset-card">
            <div class="row-main">
              <div class="row-title">透明度推荐值</div>
              <div class="row-desc">按当前皮肤“{{ activeSkinName }}”的建议恢复；未声明项使用宿主默认值</div>
            </div>
            <button class="opacity-reset-button" type="button" :disabled="!canResetOpacity" @click="resetOpacitySettings">
              {{ opacityResetting ? '正在重置…' : '重置透明度' }}
            </button>
          </div>
          <div v-for="opacity in opacityControls" :key="opacity.key" class="card row-card">
            <div class="row-main">
              <div class="row-title">{{ opacity.label }}</div>
              <div class="row-desc">{{ opacity.description }}</div>
            </div>
            <RangeSlider
              :model-value="settings.appearance[opacity.key]"
              :min="0"
              :max="1"
              :step="0.05"
              :format="fmtPercent"
              :aria-label="opacity.label"
              @commit="(v) => change('appearance', opacity.key, v)"
            />
          </div>
          <div class="card row-card">
            <div class="row-main">
              <div class="row-title">通知背景不透明度</div>
              <div class="row-desc">Web 绘制只调背景；原生模式会让整张通知轻微变透明</div>
            </div>
            <RangeSlider
              :model-value="settings.notifications.background_opacity"
              :min="0.65"
              :max="1"
              :step="0.01"
              :format="fmtPercent"
              aria-label="通知背景不透明度"
              @commit="(v) => change('notifications', 'background_opacity', v)"
            />
          </div>
          <div class="card row-card">
            <div class="row-main">
              <div class="row-title">细缝提示色</div>
            </div>
            <input
              type="color"
              class="swatch custom"
              :value="settings.appearance.sidebar_border_color"
              aria-label="细缝提示色"
              @change="change('appearance', 'sidebar_border_color', ($event.target as HTMLInputElement).value)"
            />
          </div>
        </section>
      </template>

      <!-- 通知 -->
      <template v-else-if="section === 'notifications'">
        <section>
          <h2>通知</h2>
          <div class="card row-card">
            <div class="row-main">
              <div class="row-title">通知绘制</div>
              <div class="row-desc">
                {{ settings.notifications.render_mode === 'native'
                  ? '原生绘制不创建通知 WebView；使用宿主内置样式，不应用皮肤 CSS'
                  : 'Web 绘制支持当前皮肤的通知样式' }}
              </div>
            </div>
            <div class="segmented">
              <button
                :class="{ on: settings.notifications.render_mode === 'web' }"
                aria-label="通知绘制：Web"
                @click="change('notifications', 'render_mode', 'web')"
              >Web</button>
              <button
                v-if="isWindows"
                :class="{ on: settings.notifications.render_mode === 'native' }"
                aria-label="通知绘制：原生"
                @click="change('notifications', 'render_mode', 'native')"
              >原生</button>
            </div>
          </div>
          <div class="card row-card">
            <div class="row-main">
              <div class="row-title">桌面通知背景色</div>
              <div class="row-desc">清空自定义值时跟随当前皮肤</div>
            </div>
            <div class="notification-color-control">
              <input
                type="color"
                class="swatch custom"
                :value="settings.notifications.desktop_background_color || '#23252c'"
                aria-label="桌面通知背景色"
                @change="change('notifications', 'desktop_background_color', ($event.target as HTMLInputElement).value)"
              />
              <button
                class="opacity-reset-button"
                type="button"
                :disabled="!settings.notifications.desktop_background_color"
                @click="change('notifications', 'desktop_background_color', '')"
              >跟随皮肤</button>
            </div>
          </div>
          <div class="card row-card">
            <div class="row-main">
              <div class="row-title">侧边栏气泡背景色</div>
              <div class="row-desc">可为漫画气泡单独设置背景色</div>
            </div>
            <div class="notification-color-control">
              <input
                type="color"
                class="swatch custom"
                :value="settings.notifications.bubble_background_color || '#fffdf7'"
                aria-label="侧边栏气泡背景色"
                @change="change('notifications', 'bubble_background_color', ($event.target as HTMLInputElement).value)"
              />
              <button
                class="opacity-reset-button"
                type="button"
                :disabled="!settings.notifications.bubble_background_color"
                @click="change('notifications', 'bubble_background_color', '')"
              >跟随皮肤</button>
            </div>
          </div>
        </section>
      </template>

      <!-- 插件 -->
      <template v-else-if="section === 'plugins'">
        <section>
          <h2>插件</h2>
          <div v-for="(p, i) in pluginsRef" :key="p.id" class="card plugin-card">
            <div class="plugin-row">
              <div class="row-main">
                <div class="row-title">
                  <span class="order-chip">{{ i + 1 }}</span>
                  {{ p.name }}
                  <span class="ver-badge">v{{ p.version }}</span>
                </div>
                <div class="row-desc">{{ p.description }}</div>
              </div>
              <div class="row-side">
                <button
                  class="icon-btn"
                  :disabled="i === 0"
                  title="上移"
                  aria-label="上移插件"
                  @click="movePlugin(p.id, -1)"
                >
                  <Icon name="chevron-up" :size="15" />
                </button>
                <button
                  class="icon-btn"
                  :disabled="i === pluginsRef.length - 1"
                  title="下移"
                  aria-label="下移插件"
                  @click="movePlugin(p.id, 1)"
                >
                  <Icon name="chevron-down" :size="15" />
                </button>
                <button
                  class="icon-btn"
                  :title="p.sidebar_visible ? '从侧边栏隐藏' : '在侧边栏显示'"
                  :aria-label="p.sidebar_visible ? `从侧边栏隐藏 ${p.name}` : `在侧边栏显示 ${p.name}`"
                  @click="togglePluginSidebar(p.id, !p.sidebar_visible)"
                >
                  <Icon :name="p.sidebar_visible ? 'eye' : 'eye-off'" :size="15" />
                </button>
                <button
                  class="plugin-permission-button"
                  type="button"
                  :aria-expanded="openPermissionPluginId === p.id"
                  :title="`管理 ${p.name} 的权限`"
                  @click="togglePluginPermissions(p.id)"
                >权限</button>
                <input
                  type="checkbox"
                  class="switch"
                  role="switch"
                  :aria-label="`${p.enabled ? '禁用' : '启用'}插件 ${p.name}`"
                  :checked="p.enabled"
                  @change="togglePlugin(p.id, ($event.target as HTMLInputElement).checked)"
                />
              </div>
            </div>
            <div v-if="openPermissionPluginId === p.id" class="plugin-permissions">
              <div v-if="permissionPluginsLoading" class="permission-message">正在读取权限清单…</div>
              <template v-else-if="pluginPermissionsById[p.id]">
                <div v-if="pluginPermissionsById[p.id].permissions.length" class="permission-items">
                  <div v-for="permission in pluginPermissionsById[p.id].permissions" :key="permission.id" class="permission-row">
                    <div class="row-main">
                      <div class="row-title">{{ permission.label }}</div>
                      <div class="row-desc">{{ permission.description }}</div>
                    </div>
                    <input
                      v-if="permission.enforceable"
                      type="checkbox"
                      class="switch"
                      role="switch"
                      :aria-label="`${p.name}：${permission.label}`"
                      :checked="permissionIsGranted(p.id, permission.id)"
                      :disabled="permissionSaving[`${p.id}:${permission.id}`]"
                      @change="changePluginPermission(p.id, permission.id, ($event.target as HTMLInputElement).checked)"
                    />
                    <span v-else class="permission-uncontrolled">宿主无法拦截</span>
                  </div>
                  <div class="permission-footnote">
                    开关控制经宿主 API 的调用；受信任的本地 DLL 或独立进程直接访问系统资源时，宿主无法拦截。
                  </div>
                </div>
                <div v-else class="permission-message">
                  该插件清单未声明权限。调用需要授权的宿主 API 时会被拒绝。
                </div>
              </template>
              <div v-else class="permission-message">没有找到该插件的运行时权限清单。</div>
            </div>
          </div>
        </section>
      </template>

      <!-- 插件中心 -->
      <template v-else-if="section === 'market-plugins'">
        <section class="fill">
          <h2>插件中心</h2>
          <div class="fill-body">
            <PluginCenterView />
          </div>
        </section>
      </template>

      <!-- 皮肤中心 -->
      <template v-else-if="section === 'market-skins'">
        <section class="fill">
          <h2>皮肤中心</h2>
          <div class="fill-body">
            <SkinCenterView />
          </div>
        </section>
      </template>

      <!-- 关于 -->
      <template v-else>
        <section>
          <h2>关于</h2>

          <!-- 应用摘要 -->
          <div class="app-card">
            <span class="brand-mark lg">AT</span>
            <div class="app-meta">
              <strong>SideWeave</strong>
              <small>Rust + Tauri 重构版</small>
            </div>
            <span class="badge accent">v{{ appVersionRef || sysInfo?.app_version || '—' }}</span>
          </div>

          <div class="card row-card">
            <div class="row-main">
              <div class="row-title">数据目录</div>
              <div class="row-desc">{{ dataDirRef || '—' }}</div>
            </div>
            <button class="btn" @click="onOpenDataDir"><Icon name="folder" :size="14" /> 打开</button>
          </div>
          <div class="card row-card">
            <div class="row-main">
              <div class="row-title">全局热键</div>
              <div class="row-desc hotkey">
                <template v-if="toggleHotkeySegs.length">
                  <kbd v-for="(seg, i) in toggleHotkeySegs" :key="`${seg}-${i}`">{{
                    hotkeyLabel(seg)
                  }}</kbd>
                </template>
                <template v-else>
                  <kbd>未设置</kbd>
                </template>
                显示 / 隐藏侧边栏
              </div>
            </div>
          </div>
          <div class="card row-card">
            <div class="row-main">
              <div class="row-title">重启应用</div>
              <div class="row-desc">保存的设置将在重启后完整生效</div>
            </div>
            <button class="btn" @click="onRestartApp"><Icon name="refresh" :size="14" /> 重启</button>
          </div>

          <!-- 在线更新 -->
          <div class="card row-card">
            <div class="row-main">
              <div class="row-title">在线更新</div>
              <div class="row-desc">
                {{
                  updateState === 'idle'
                    ? `当前版本 ${appVersionRef}，从云端检查新版本`
                    : updateState === 'checking'
                      ? '正在检查更新…'
                      : updateState === 'downloading'
                        ? '正在下载更新，完成后将自动安装并重启…'
                        : updateState === 'latest'
                          ? ignoredVersion
                            ? `已忽略 v${ignoredVersion}，撤销后可再次提示`
                            : '当前已是最新版本'
                          : `发现新版本 ${updateVersion}，可下载安装`
                }}
              </div>
            </div>
            <div class="update-actions">
              <button
                class="btn"
                :class="{ accent: updateState === 'available' }"
                :disabled="updateState === 'checking' || updateState === 'downloading'"
                @click="onUpdateAction"
              >
                <Icon name="refresh" :size="14" />
                {{ updateState === 'idle' ? '检查更新' : updateState === 'checking' ? '检查中…' : updateState === 'downloading' ? '下载中…' : updateState === 'available' ? '下载并安装' : '重新检查' }}
              </button>
              <button
                v-if="updateState === 'available'"
                class="btn"
                title="检查更新时不再提示此版本（直至新版本发布）"
                @click="ignoreVersion"
              >
                忽略此版本
              </button>
              <button
                v-else-if="updateState === 'latest' && ignoredVersion"
                class="btn"
                title="恢复对该版本的更新提示"
                @click="unignoreVersion"
              >
                撤销忽略 v{{ ignoredVersion }}
              </button>
            </div>
          </div>

          <!-- 数据备份 -->
          <h3 class="group-label">数据备份</h3>
          <div class="card row-card">
            <div class="row-main">
              <div class="row-title">备份与恢复</div>
              <div class="row-desc">
                备份设置、插件数据、已安装插件与皮肤（zip）；导入后应用自动重启生效
              </div>
            </div>
            <div class="update-actions">
              <button class="btn" :disabled="backupBusy" @click="onBackupExport">
                <Icon name="folder" :size="14" /> 导出备份
              </button>
              <button
                class="btn"
                :disabled="backupBusy"
                title="导入备份会覆盖当前全部数据"
                @click="onBackupImport"
              >
                <Icon name="refresh" :size="14" /> 导入备份
              </button>
            </div>
          </div>

          <!-- 诊断小区块 -->
          <h3 class="group-label">诊断</h3>
          <div class="stat-grid">
            <div class="stat-card">
              <span class="stat-label"><Icon name="clock" :size="13" /> 运行时长</span>
              <strong>{{ sysInfo ? fmtUptime(sysInfo.uptime_secs) : '—' }}</strong>
            </div>
            <div class="stat-card">
              <span class="stat-label"><Icon name="monitor" :size="13" /> 显示器数量</span>
              <strong>{{ sysInfo ? `${sysInfo.monitors.length} 台` : '—' }}</strong>
            </div>
            <div class="stat-card">
              <span class="stat-label"><Icon name="palette" :size="13" /> 主题模式</span>
              <strong>{{ themeModeLabel }}</strong>
            </div>
            <div class="stat-card">
              <span class="stat-label"><Icon name="check" :size="13" /> 实际生效</span>
              <strong>{{ effectiveModeLabel }}</strong>
            </div>
          </div>
        </section>
      </template>
    </div>
    <Toast :message="toast" :kind="toastKind" @done="toast = ''" />
  </div>
</template>

<style scoped>
.settings {
  display: flex;
  height: 100%;
  min-height: 0;
  gap: 14px;
}

/* ---------- 左侧导航 ---------- */

.nav {
  flex: none;
  width: 124px;
  display: flex;
  flex-direction: column;
  gap: 10px;
  padding: 2px 0;
}

.brand {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 2px 8px 12px;
  border-bottom: 1px solid color-mix(in srgb, var(--border) 55%, transparent);
}

.brand-mark {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 28px;
  height: 28px;
  flex: none;
  border-radius: 8px;
  background: var(--accent);
  color: #fff;
  font-size: 11px;
  font-weight: 700;
}
.brand-mark.lg {
  width: 38px;
  height: 38px;
  border-radius: 11px;
  font-size: 13px;
}

.brand-text {
  display: flex;
  flex-direction: column;
  min-width: 0;
  line-height: 1.3;
}
.brand-text strong {
  font-size: 12.5px;
}
.brand-text small {
  font-size: 10.5px;
  color: var(--text-muted);
}

.nav-list {
  display: flex;
  flex-direction: column;
  gap: 3px;
}

.nav-item {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 8px 12px;
  border: none;
  border-radius: 999px;
  background: transparent;
  color: var(--text-muted);
  font-size: 13px;
  text-align: left;
}
.nav-item span {
  white-space: nowrap;
}
.nav-item:hover {
  background: var(--surface-subtle);
  color: var(--text);
}
.nav-item.active {
  background: color-mix(in srgb, var(--accent) 22%, transparent);
  color: var(--text);
  font-weight: 600;
}
.nav-item.active svg {
  color: var(--accent);
}

.nav-foot {
  margin-top: auto;
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 10px 8px 0;
  border-top: 1px solid color-mix(in srgb, var(--border) 55%, transparent);
  color: var(--text-muted);
  font-size: 10.5px;
  font-variant-numeric: tabular-nums;
}
.foot-dot {
  width: 7px;
  height: 7px;
  border-radius: 50%;
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--text) 40%, transparent);
}
.foot-dot.dark {
  background: #1a1c1e;
}
.foot-dot.light {
  background: #f4f5f7;
}

/* ---------- 右侧面板 ---------- */

.pane {
  flex: 1;
  min-width: 0;
  overflow: auto;
  overscroll-behavior: contain;
  padding-right: 4px;
}

section h2 {
  position: sticky;
  top: 0;
  z-index: 2;
  margin: 4px 0 12px;
  padding: 8px 12px;
  font-size: 14px;
  font-weight: 600;
  border-radius: 10px;
  /* 吸顶：滚动时分组标题悬浮在卡片之上，半透明 + 背景模糊保证可读 */
  background: color-mix(in srgb, var(--surface) 78%, transparent);
  backdrop-filter: blur(10px);
  -webkit-backdrop-filter: blur(10px);
}

.group-label {
  margin: 14px 2px 8px;
  font-size: 11px;
  font-weight: 600;
  letter-spacing: 0.04em;
  color: var(--text-muted);
}

.card {
  background: color-mix(in srgb, var(--surface-subtle) calc(var(--component-opacity, 1) * 100%), transparent);
  border: 1px solid color-mix(in srgb, var(--border) 65%, transparent);
  border-radius: var(--radius, 12px);
  margin-bottom: 10px;
}

.row-card {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 12px 16px;
}

.row-title {
  font-size: 13px;
  font-weight: 500;
}
.row-desc {
  margin-top: 2px;
  font-size: 11.5px;
  color: var(--text-muted);
  word-break: break-all;
}
.row-desc.hotkey {
  display: flex;
  align-items: center;
  gap: 5px;
  flex-wrap: wrap;
}

kbd {
  padding: 1px 7px;
  border: 1px solid var(--border);
  border-bottom-width: 2px;
  border-radius: 6px;
  background: var(--surface);
  color: var(--text);
  font-family: inherit;
  font-size: 10.5px;
}

.row-side {
  display: flex;
  align-items: center;
  gap: 4px;
}

/* ---------- 概览：应用卡片 / 统计网格 / 显示器 / 快捷操作 ---------- */

.app-card {
  display: flex;
  align-items: center;
  gap: 12px;
  padding: 14px 16px;
  border: 1px solid color-mix(in srgb, var(--accent) 28%, var(--border));
  border-radius: 14px;
  background: linear-gradient(
    135deg,
    color-mix(in srgb, var(--accent) 10%, var(--surface-subtle)),
    var(--surface-subtle)
  );
  margin-bottom: 4px;
}
.app-meta {
  flex: 1;
  min-width: 0;
  display: flex;
  flex-direction: column;
  gap: 2px;
}
.app-meta strong {
  font-size: 15px;
}
.app-meta small {
  font-size: 11px;
  color: var(--text-muted);
}
.app-badges {
  display: flex;
  align-items: center;
  gap: 6px;
  flex-wrap: wrap;
  justify-content: flex-end;
}

.badge {
  padding: 3px 9px;
  border: 1px solid color-mix(in srgb, var(--border) 70%, transparent);
  border-radius: 999px;
  background: var(--surface);
  color: var(--text-muted);
  font-size: 10.5px;
  font-weight: 600;
  white-space: nowrap;
}
.badge.accent {
  border-color: transparent;
  background: color-mix(in srgb, var(--accent) 14%, transparent);
  color: var(--accent);
}

.stat-grid {
  display: grid;
  grid-template-columns: repeat(2, minmax(0, 1fr));
  gap: 10px;
}

.stat-card {
  display: flex;
  flex-direction: column;
  align-items: flex-start;
  gap: 3px;
  padding: 12px 14px;
  border: 1px solid color-mix(in srgb, var(--border) 65%, transparent);
  border-radius: var(--radius, 12px);
  background: var(--surface-subtle);
  text-align: left;
}
.stat-card strong {
  font-size: 15px;
  font-weight: 600;
  font-variant-numeric: tabular-nums;
  word-break: break-all;
}
.stat-card.wide {
  width: 100%;
  margin-bottom: 4px;
}
.stat-label {
  display: inline-flex;
  align-items: center;
  gap: 5px;
  font-size: 11px;
  color: var(--text-muted);
}
.stat-sub {
  font-size: 10.5px;
  color: var(--text-muted);
}

.plugin-stat {
  cursor: pointer;
  transition: border-color 0.15s ease;
}
.plugin-stat:hover {
  border-color: var(--accent);
}
.bar {
  width: 100%;
  height: 4px;
  margin-top: 7px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--border) 55%, transparent);
  overflow: hidden;
}
.bar-fill {
  display: block;
  height: 100%;
  border-radius: 999px;
  background: var(--accent);
  transition: width 0.3s ease;
}

.monitor-list {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-bottom: 4px;
}
.monitor-chip {
  display: inline-flex;
  align-items: center;
  gap: 7px;
  padding: 7px 11px;
  border: 1px solid color-mix(in srgb, var(--border) 65%, transparent);
  border-radius: 10px;
  background: var(--surface-subtle);
  font-size: 11.5px;
  font-variant-numeric: tabular-nums;
}
.monitor-idx {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 16px;
  height: 16px;
  border-radius: 50%;
  background: color-mix(in srgb, var(--accent) 20%, transparent);
  color: var(--accent);
  font-size: 9.5px;
  font-weight: 700;
}
.monitor-scale {
  color: var(--text-muted);
}
.monitor-empty {
  padding: 12px 16px;
  font-size: 11.5px;
  color: var(--text-muted);
}

.quick-list {
  display: flex;
  flex-direction: column;
  gap: 8px;
}
.quick-btn {
  display: flex;
  align-items: center;
  gap: 10px;
  width: 100%;
  padding: 11px 14px;
  border: 1px solid color-mix(in srgb, var(--border) 65%, transparent);
  border-radius: var(--radius, 12px);
  background: var(--surface-subtle);
  color: var(--text);
  font-size: 12.5px;
  text-align: left;
}
.quick-btn svg {
  color: var(--text-muted);
  transition: color 0.15s ease;
}
.quick-btn:hover {
  border-color: var(--accent);
  color: var(--accent);
}
.quick-btn:hover svg {
  color: var(--accent);
}

/* ---------- 插件：排序序号 + 版本徽标 ---------- */

.plugin-card {
  overflow: hidden;
  padding: 0;
}
.plugin-row {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 16px;
  padding: 12px 16px;
}
.plugin-row .row-main { flex: 1; }
.plugin-permission-button {
  padding: 5px 9px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--surface-subtle);
  color: var(--text-muted);
  font: inherit;
  font-size: 11px;
  cursor: pointer;
}
.plugin-permission-button:hover,
.plugin-permission-button[aria-expanded='true'] {
  border-color: var(--accent);
  color: var(--accent);
}
.plugin-permissions {
  padding: 4px 16px 10px;
  border-top: 1px solid color-mix(in srgb, var(--border) 65%, transparent);
}
.permission-row {
  display: flex;
  align-items: center;
  gap: 14px;
  min-height: 48px;
  padding: 7px 3px;
  border-top: 1px solid color-mix(in srgb, var(--border) 65%, transparent);
}
.permission-row:first-child { border-top: 0; }
.permission-row .row-title { font-size: 12px; }
.permission-row .row-desc { font-size: 10.5px; line-height: 1.35; }
.permission-uncontrolled { flex: none; color: var(--text-muted); font-size: 10.5px; }
.permission-message {
  padding: 9px 3px 3px;
  color: var(--text-muted);
  font-size: 11px;
  line-height: 1.5;
}
.permission-footnote {
  padding: 8px 3px 2px;
  color: var(--text-muted);
  font-size: 10.5px;
  line-height: 1.45;
}

.order-chip {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  width: 18px;
  height: 18px;
  margin-right: 6px;
  border-radius: 6px;
  background: color-mix(in srgb, var(--border) 45%, transparent);
  color: var(--text-muted);
  font-size: 10px;
  font-weight: 600;
  vertical-align: 2px;
}
.ver-badge {
  margin-left: 7px;
  padding: 1.5px 8px;
  border-radius: 999px;
  background: color-mix(in srgb, var(--accent) 13%, transparent);
  color: var(--accent);
  font-size: 10px;
  font-weight: 600;
  vertical-align: 1px;
}

/* ---------- 控件（沿用既有 M3 风格） ---------- */

/* switch 样式 */
.switch {
  appearance: none;
  width: 40px;
  height: 22px;
  border-radius: 999px;
  background: var(--border);
  position: relative;
  cursor: pointer;
  transition: background 0.2s ease;
  flex: none;
}
.switch::after {
  content: '';
  position: absolute;
  top: 2px;
  left: 2px;
  width: 18px;
  height: 18px;
  border-radius: 50%;
  background: #fff;
  transition: transform 0.2s ease;
}
.switch:checked {
  background: var(--accent);
}
.switch:checked::after {
  transform: translateX(18px);
}

.segmented {
  display: flex;
  gap: 2px;
  padding: 3px;
  border-radius: 10px;
  background: color-mix(in srgb, var(--border) 40%, transparent);
}
.segmented button {
  padding: 5px 12px;
  border: none;
  border-radius: 8px;
  background: transparent;
  color: var(--text-muted);
  font-size: 12px;
}
.segmented button.on {
  background: var(--surface);
  color: var(--text);
  font-weight: 600;
  box-shadow: 0 1px 3px rgba(0, 0, 0, 0.2);
}

.swatches {
  display: flex;
  align-items: center;
  gap: 8px;
}
.notification-color-control {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: none;
}
.swatch {
  width: 24px;
  height: 24px;
  padding: 0;
  border-radius: 50%;
  border: 2px solid transparent;
  cursor: pointer;
}
.swatch.on {
  border-color: var(--text);
  box-shadow: 0 0 0 2px color-mix(in srgb, var(--text) 25%, transparent);
}
.swatch.custom {
  width: 28px;
  height: 28px;
  border: 1px dashed var(--border-strong);
  background: transparent;
  padding: 0;
}

/* 主题分段控件上标注实际生效主题的小圆点 */
.mode-dot {
  display: inline-block;
  width: 7px;
  height: 7px;
  margin-left: 6px;
  border-radius: 50%;
  vertical-align: 1px;
  box-shadow: 0 0 0 1px color-mix(in srgb, var(--text) 40%, transparent);
}
.mode-dot.dark {
  background: #1a1c1e;
}
.mode-dot.light {
  background: #f4f5f7;
}

.btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 7px 14px;
  border-radius: 10px;
  border: 1px solid var(--border);
  background: var(--surface);
  color: var(--text);
  font-size: 12.5px;
  white-space: nowrap;
}
.btn:hover {
  border-color: var(--accent);
  color: var(--accent);
}

.icon-btn {
  display: flex;
  align-items: center;
  justify-content: center;
  width: 26px;
  height: 26px;
  padding: 0;
  border: none;
  border-radius: 7px;
  background: transparent;
  color: var(--text-muted);
}
.icon-btn:hover:not(:disabled) {
  background: var(--surface-container);
  color: var(--text);
}
.icon-btn:disabled {
  opacity: 0.35;
}

/* ---------- 插件中心 / 皮肤中心：占满面板高度，由视图内部滚动 ---------- */

section.fill {
  height: 100%;
  display: flex;
  flex-direction: column;
  overflow: hidden;
}
section.fill h2 {
  flex: none;
}
.fill-body {
  flex: 1;
  min-height: 0;
}

.update-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex: none;
}
.update-actions .btn {
  white-space: nowrap;
}

.opacity-reset-button {
  flex: none;
  padding: 7px 11px;
  border: 1px solid var(--border);
  border-radius: 8px;
  background: var(--surface-subtle);
  color: var(--text);
  font: inherit;
  font-size: 12px;
}
.opacity-reset-button:hover:not(:disabled) {
  border-color: var(--accent);
  color: var(--accent);
}
.opacity-reset-button:disabled {
  cursor: wait;
  opacity: .65;
}

</style>
