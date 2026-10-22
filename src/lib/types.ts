export type PerformanceProfile = 'eco' | 'balanced' | 'performance' | 'auto'
export type PreviewDisplayMode = 'default' | 'compact'

export interface PerformancePolicy {
  detailPrewarm: boolean
  detailKeepAlive: boolean
  previewPrewarm: boolean
  previewKeepAlive: boolean
  hoverDelayMs: number
}

export interface SystemMemorySnapshot {
  totalPhysBytes: number
  availPhysBytes: number
  memoryLoadPct: number
}

export interface PerformanceStatusPayload {
  userProfile: PerformanceProfile
  effectiveProfile: PerformanceProfile
  policy: PerformancePolicy
  memory?: SystemMemorySnapshot | null
}

export interface GeneralSettings {
  run_on_startup: boolean
  enable_mouse_hover: boolean
  auto_hide_delay: number
  hover_preview_delay: number
  control_center_always_on_top: boolean
  performance_profile?: PerformanceProfile
  github_mirror?: string
}

export type SkinId = 'default' | `user:${string}`

export interface AppearanceSettings {
  theme_mode: 'light' | 'dark' | 'system'
  skin: SkinId
  accent_color: string
  background_image_path: string
  background_image_name: string
  background_image_apply_to_sidebar: boolean
  background_image_fit: 'cover' | 'fill' | 'contain'
  background_image_opacity: number
  background_image_blur: number
  interface_scale: number
  sidebar_position: 'left' | 'right' | 'top'
  sidebar_monitor_id: string | null
  collapsed_width: number
  icon_size: number
  peek_width: number
  sidebar_panel_opacity: number
  detail_panel_opacity: number
  control_panel_opacity: number
  preview_panel_opacity: number
  preview_image_opacity: number
  preview_visible_rows: number
  preview_width: number | null
  preview_display_mode: PreviewDisplayMode
  sidebar_image_opacity: number
  detail_image_opacity: number
  component_opacity: number
  glass_effect_enabled: boolean
  sidebar_height_percent: number
  sidebar_y_offset: number
  sidebar_x_offset: number
  sidebar_border_color: string
}

export interface MonitorOption {
  id: string | null
  name: string
  device_name: string | null
  label: string
  width: number
  height: number
  scale: number
  is_primary: boolean
}

export interface PluginsSettings {
  disabled: string[]
  hidden_from_sidebar: string[]
  order: string[]
  permission_grants: Record<string, Record<string, boolean>>
  permission_grants_initialized: boolean
}

export interface NotificationSettings {
  render_mode: 'web' | 'native'
  desktop_background_opacity: number
  bubble_background_opacity: number
  desktop_background_color: string
  bubble_background_color: string
}

/** 全局热键（Rust 侧 serde default，格式如 "alt+space"） */
export interface Shortcuts {
  toggle_sidebar: string
  /** 呼出控制中心（空串 = 不启用） */
  control_center: string
}

export interface Settings {
  general: GeneralSettings
  appearance: AppearanceSettings
  notifications: NotificationSettings
  plugins: PluginsSettings
  shortcuts: Shortcuts
}

export interface PluginMeta {
  id: string
  name: string
  icon: string
  description: string
  version: string
  enabled: boolean
  sidebar_visible: boolean
}

export interface StateSnapshot {
  settings: Settings
  plugins: PluginMeta[]
  autostart_enabled: boolean
  app_version: string
  data_dir: string
}
