import { listen } from '@tauri-apps/api/event'
import { convertFileSrc } from '@tauri-apps/api/core'
import { EVENTS, getState } from './api'
import type { Settings, StateSnapshot } from './types'

/** 皮肤清单（与 Rust 侧 appearance.skin 枚举一致） */
export const SKINS = [
  { id: 'default', label: '默认' },
] as const

export type SkinId = (typeof SKINS)[number]['id']

/** 应用主题（data-theme）× 皮肤（data-skin）与强调色到 documentElement */
export function applyTheme(settings: Settings) {
  const {
    theme_mode,
    accent_color,
    skin,
    background_image_path,
    background_image_fit,
    background_image_apply_to_sidebar,
    background_image_opacity,
    background_image_blur
  } = settings.appearance
  const desktopNotificationOpacity = settings.notifications?.desktop_background_opacity ?? 0.94
  const bubbleNotificationOpacity = settings.notifications?.bubble_background_opacity ?? 0.9
  const previewPanelOpacity = settings.appearance.preview_panel_opacity ?? 0.97
  const notificationRoot = document.documentElement
  const mode =
    theme_mode === 'system'
      ? window.matchMedia('(prefers-color-scheme: dark)').matches
        ? 'dark'
        : 'light'
      : theme_mode
  document.documentElement.dataset.theme = mode
  if (skin.startsWith('user:')) {
    // Keep a selector-safe id on the root so skin CSS can replace every host surface.
    document.documentElement.dataset.skin = `user-${skin.slice(5)}`
    void ensureUserSkinStyle(skin.slice(5))
  } else {
    document.documentElement.dataset.skin = SKINS.some((s) => s.id === skin)
      ? skin
      : 'default'
    removeUserSkinStyle()
  }
  applyAppBackgroundImage({
    path: background_image_path,
    fit: background_image_fit,
    applyToSidebar: background_image_apply_to_sidebar,
    opacity: background_image_opacity,
    previewOpacity: settings.appearance.preview_image_opacity ?? 0.7,
    blur: background_image_blur
  })
  // 强调色为出厂默认值时不内联注入，让皮肤调色板决定 accent；
  // 用户显式改过强调色则覆盖皮肤
  if (/^#[0-9A-Fa-f]{6}$/.test(accent_color) && accent_color.toLowerCase() !== '#006874') {
    document.documentElement.style.setProperty('--accent', accent_color)
  } else {
    document.documentElement.style.removeProperty('--accent')
  }

  notificationRoot.style.setProperty('--notification-desktop-background-opacity', String(desktopNotificationOpacity))
  notificationRoot.style.setProperty('--notification-bubble-background-strength', `${Math.round(bubbleNotificationOpacity * 100)}%`)
  notificationRoot.style.setProperty('--preview-panel-opacity', String(previewPanelOpacity))
  notificationRoot.style.setProperty('--preview-panel-opacity-percent', `${Math.round(previewPanelOpacity * 100)}%`)
  // Keep the old variables available to custom skins that still reference them.
  notificationRoot.style.setProperty('--notification-background-opacity', String(desktopNotificationOpacity))
  notificationRoot.style.setProperty(
    '--notification-background-strength',
    `${Math.round(bubbleNotificationOpacity * 100)}%`
  )
  const notificationColors = [
    ['--notification-custom-desktop-background', settings.notifications?.desktop_background_color],
    ['--notification-custom-bubble-background', settings.notifications?.bubble_background_color]
  ] as const
  const bubbleBackground = settings.notifications?.bubble_background_color
  if (bubbleBackground && /^#[0-9A-Fa-f]{6}$/.test(bubbleBackground)) {
    notificationRoot.style.setProperty(
      '--notification-custom-bubble-background-fill',
      hexToRgba(bubbleBackground, Math.max(0, Math.min(1, bubbleNotificationOpacity)))
    )
  } else {
    notificationRoot.style.removeProperty('--notification-custom-bubble-background-fill')
  }
  for (const [property, color] of notificationColors) {
    if (color && /^#[0-9A-Fa-f]{6}$/.test(color)) {
      notificationRoot.style.setProperty(property, color)
      const red = Number.parseInt(color.slice(1, 3), 16)
      const green = Number.parseInt(color.slice(3, 5), 16)
      const blue = Number.parseInt(color.slice(5, 7), 16)
      const isLight = red * 0.299 + green * 0.587 + blue * 0.114 > 150
      const type = property.includes('desktop') ? 'desktop' : 'bubble'
      const foreground = isLight ? '#202124' : '#f3f4f7'
      const body = isLight ? '#4b4d52' : '#c8cad1'
      notificationRoot.style.setProperty(`--notification-custom-${type}-color`, foreground)
      notificationRoot.style.setProperty(`--notification-custom-${type}-body-color`, body)
      if (type === 'bubble') {
        notificationRoot.style.setProperty('--notification-custom-bubble-title', foreground)
      }
    } else {
      notificationRoot.style.removeProperty(property)
      const type = property.includes('desktop') ? 'desktop' : 'bubble'
      notificationRoot.style.removeProperty(`--notification-custom-${type}-color`)
      notificationRoot.style.removeProperty(`--notification-custom-${type}-body-color`)
      if (type === 'bubble') {
        notificationRoot.style.removeProperty('--notification-custom-bubble-title')
      }
    }
  }
}

function applyAppBackgroundImage(background: {
  path: string
  fit: 'cover' | 'fill' | 'contain'
  applyToSidebar: boolean
  opacity: number
  previewOpacity: number
  blur: number
}) {
  const root = document.documentElement
  const pathname = window.location.pathname
  const isSidebarSurface = /\/sidebar\.html$/.test(pathname)
  const isDetailSurface = /\/detail\.html$/.test(pathname)
  const isPreviewSurface = /\/preview\.html$/.test(pathname)
  const shouldApply = Boolean(background.path) && (isDetailSurface || isPreviewSurface || (isSidebarSurface && background.applyToSidebar))
  if (!shouldApply) {
    root.style.removeProperty('--app-wallpaper-image')
    root.style.removeProperty('--app-wallpaper-size')
    root.style.removeProperty('--app-wallpaper-opacity')
    root.style.removeProperty('--app-wallpaper-blur')
    if (isDetailSurface) {
      root.style.removeProperty('--detail-shell-overlay')
      root.style.removeProperty('--detail-image')
    }
    if (isSidebarSurface) root.style.removeProperty('--rail-image')
    return
  }

  try {
    const source = convertFileSrc(background.path)
    root.style.setProperty('--app-wallpaper-image', `url(${JSON.stringify(source)})`)
    root.style.setProperty('--app-wallpaper-size', background.fit === 'fill' ? '100% 100%' : background.fit)
    const opacity = isPreviewSurface ? background.previewOpacity : background.opacity
    root.style.setProperty('--app-wallpaper-opacity', String(Math.max(0, Math.min(1, opacity))))
    root.style.setProperty('--app-wallpaper-blur', `${Math.max(0, Math.min(24, background.blur))}px`)
    if (isDetailSurface) {
      root.style.setProperty('--detail-shell-overlay', 'transparent')
      // Use the app-wide wallpaper in place of the skin's separate detail wallpaper layer.
      root.style.setProperty('--detail-image', 'none')
      root.style.removeProperty('--rail-image')
    } else if (isSidebarSurface) {
      // Use the app-wide wallpaper in place of the skin's separate sidebar wallpaper layer.
      root.style.setProperty('--rail-image', 'none')
      root.style.removeProperty('--detail-shell-overlay')
      root.style.removeProperty('--detail-image')
    }
  } catch {
    root.style.removeProperty('--app-wallpaper-image')
    root.style.removeProperty('--app-wallpaper-size')
    root.style.removeProperty('--app-wallpaper-opacity')
    root.style.removeProperty('--app-wallpaper-blur')
    if (isDetailSurface) {
      root.style.removeProperty('--detail-shell-overlay')
      root.style.removeProperty('--detail-image')
    }
    if (isSidebarSurface) root.style.removeProperty('--rail-image')
  }
}

/** 用户皮肤包（市场下载的 CSS）注入：<style data-user-skin>，切走时移除 */
let lastUserSkinId = ''

async function ensureUserSkinStyle(id: string): Promise<void> {
  if (lastUserSkinId === id && document.getElementById('user-skin-style')) return
  lastUserSkinId = id
  try {
    const { skinUserCss } = await import('./marketApi')
    const css = await skinUserCss(id)
    if (
      lastUserSkinId !== id ||
      document.documentElement.dataset.skin !== `user-${id}`
    ) return
    let tag = document.getElementById('user-skin-style') as HTMLStyleElement | null
    if (!tag) {
      tag = document.createElement('style')
      tag.id = 'user-skin-style'
      document.head.appendChild(tag)
    }
    tag.textContent = css
  } catch {
    if (lastUserSkinId === id) removeUserSkinStyle()
  }
}

function removeUserSkinStyle(): void {
  lastUserSkinId = ''
  document.getElementById('user-skin-style')?.remove()
}

/**
 * 监听系统深浅色变化。
 * 注意：不在此处捕获 settings 快照——OS 主题翻转时由调用方读取最新值，
 * 否则会把 bind 时刻的旧设置回写覆盖新值。
 */
export function watchSystemTheme(onChange: () => void): void {
  window.matchMedia('(prefers-color-scheme: dark)').addEventListener('change', onChange)
}

export function hexToRgba(hex: string, alpha: number): string {
  const m = /^#?([0-9A-Fa-f]{6})$/.exec(hex)
  if (!m) return `rgba(32, 32, 32, ${alpha})`
  const v = parseInt(m[1], 16)
  const r = (v >> 16) & 0xff
  const g = (v >> 8) & 0xff
  const b = v & 0xff
  return `rgba(${r}, ${g}, ${b}, ${alpha})`
}

/**
 * 初始加载宿主状态并订阅设置变更。
 * 返回完整 StateSnapshot，调用方可复用其中的 plugins / app_version 等，
 * 无需再重复调用 getState。
 */
export async function bindSettings(
  onUpdate: (settings: Settings) => void
): Promise<StateSnapshot> {
  const snapshot = await getState()
  // 可变局部量始终指向最新 settings：settings://changed 回调里更新它，
  // 系统主题回调读它，避免闭包捕获 bind 时刻的旧快照
  let latest = snapshot.settings
  applyTheme(latest)
  watchSystemTheme(() => {
    applyTheme(latest)
    onUpdate(latest)
  })
  await listen<Partial<Settings>>(EVENTS.settingsChanged, async () => {
    const fresh = (await getState()).settings
    latest = fresh
    applyTheme(fresh)
    onUpdate(fresh)
  })
  return snapshot
}
