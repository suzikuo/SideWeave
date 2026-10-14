import { invoke } from '@tauri-apps/api/core'
import type { PluginMeta, Settings, StateSnapshot } from './types'
import type { SkinAppearanceDefaults } from './marketApi'

/** 宿主状态 */
export const getState = () => invoke<StateSnapshot>('get_state')
export const appReady = () => invoke<void>('app_ready')

/** 修改单个设置项（Rust 侧校验并应用副作用），返回最新 settings */
export const setSetting = (category: string, key: string, value: unknown) =>
  invoke<Settings>('set_setting', { category, key, value })

/** Switch skins and apply the skin's recommended opacity values in one settings update. */
export const setSkinAppearance = (skinId: string, opacityDefaults: SkinAppearanceDefaults) =>
  invoke<Settings>('set_skin_appearance', { skinId, opacityDefaults })

/** 插件启用/禁用与排序 */
export const setPluginEnabled = (pluginId: string, enabled: boolean) =>
  invoke<PluginMeta[]>('set_plugin_enabled', { pluginId, enabled })

/** Show or hide a plugin in the sidebar without changing whether it is enabled. */
export const setPluginSidebarVisible = (pluginId: string, visible: boolean) =>
  invoke<PluginMeta[]>('set_plugin_sidebar_visible', { pluginId, visible })

export const setPluginOrder = (order: string[]) =>
  invoke<PluginMeta[]>('set_plugin_order', { order })

/** Update a single manifest-declared permission for an installed plugin. */
export const setPluginPermission = (pluginId: string, permission: string, granted: boolean) =>
  invoke<Settings>('set_plugin_permission', { pluginId, permission, granted })

/** 窗口控制 */
export const openDetail = (page: string) => invoke<void>('open_detail', { page })
/** 打开已安装插件详情，使用独立命名空间避免和宿主页面路由冲突。 */
export const openPluginDetail = (pluginId: string) => openDetail(`user:${pluginId}`)
export const getMainSurface = () => invoke<'sidebar' | 'detail' | 'control'>('get_main_surface')
export const openControlCenter = () => invoke<void>('open_control_center')
export const closeDetail = () => invoke<void>('close_detail')
export const minimizeDetail = () => invoke<boolean>('minimize_detail')
export const toggleSidebar = () => invoke<void>('toggle_sidebar')
export const sidebarHoverEnter = () => invoke<number>('sidebar_hover_enter')
export const sidebarCollapseIfUnhovered = (token: number) =>
  invoke<void>('sidebar_collapse_if_unhovered', { token })

/** 通用 */
export const openDataDir = () => invoke<void>('open_data_dir')
export const restartApp = () => invoke<void>('restart_app')
export const openExternalUrl = (url: string) => invoke<void>('open_external_url', { url })

/** 事件名（Rust → 前端） */
export const EVENTS = {
  settingsChanged: 'settings://changed',
  sidebarExpanded: 'sidebar://expanded',
  mainSurface: 'main://surface',
  previewUpdate: 'preview://update'
} as const

// ---------- 在线更新 ----------

export interface UpdateInfo {
  available: boolean
  currentVersion: string
  version: string
  notes: string
}

export const updateCheck = () => invoke<UpdateInfo>('update_check')
export const updateInstall = () => invoke<string>('update_install')

/** 忽略指定版本（空串 = 撤销忽略） */
export const updateIgnoreSet = (version: string) =>
  invoke<void>('update_ignore_set', { version })

// ---------- 设置备份 ----------

/** 导出备份 zip，返回保存路径（取消返回空串） */
export const backupExport = () => invoke<string>('backup_export')

/** 导入备份 zip（恢复后应用自动重启） */
export const backupImport = () => invoke<void>('backup_import')

// ---------- 插件悬浮预览卡片 (Hover Preview) ----------

export type PreviewMode = 'declarative' | 'custom' | 'static'

export interface PreviewProgress {
  value: number
  max?: number
  label?: string
}

export interface PreviewMetric {
  label: string
  value: string
  subvalue?: string
}

export interface PreviewItem {
  id: string
  title: string
  subtitle?: string
  tooltip?: string
  icon?: string
  badge?: string
  method?: string
  payload?: any
}

export interface PreviewAction {
  id: string
  label: string
  primary?: boolean
  danger?: boolean
  method?: string
  payload?: any
}

export interface PreviewPayload {
  mode?: PreviewMode
  pluginId: string
  title: string
  status?: 'online' | 'offline' | 'busy' | 'error' | 'info' | string
  statusText?: string
  subtitle?: string
  metrics?: PreviewMetric[]
  items?: PreviewItem[]
  progress?: PreviewProgress
  actions?: PreviewAction[]
  customUrl?: string
}

export interface ButtonRect {
  x: number
  y: number
  width: number
  height: number
}

export const previewSidebarEnter = (pluginId: string, rect: ButtonRect) =>
  invoke<void>('preview_sidebar_enter', { pluginId, rect })

export const previewSidebarLeave = (pluginId: string) =>
  invoke<void>('preview_sidebar_leave', { pluginId })

export const previewWindowEnter = () =>
  invoke<void>('preview_window_enter')

export const previewWindowLeave = () =>
  invoke<void>('preview_window_leave')

export const previewHideNow = () =>
  invoke<void>('preview_hide_now')

export const getCurrentPreview = () =>
  invoke<PreviewPayload | null>('get_current_preview')

export const refreshPreview = () =>
  invoke<PreviewPayload | null>('refresh_preview')

export const getPerformanceStatus = () =>
  invoke<import('./types').PerformanceStatusPayload>('get_performance_status')

