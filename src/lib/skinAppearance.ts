import type { SkinAppearanceDefaults } from './marketApi'
import { setSkinAppearance } from './api'

export const OPACITY_SETTING_KEYS = [
  'sidebar_panel_opacity',
  'sidebar_image_opacity',
  'detail_panel_opacity',
  'control_panel_opacity',
  'preview_panel_opacity',
  'preview_image_opacity',
  'detail_image_opacity',
  'component_opacity'
] as const

export type OpacitySettingKey = (typeof OPACITY_SETTING_KEYS)[number]

export interface InstalledSkinAppearance {
  id: string
  name: string
  appearanceDefaults?: SkinAppearanceDefaults | null
}

const HOST_OPACITY_DEFAULTS: Record<OpacitySettingKey, number> = {
  sidebar_panel_opacity: 0,
  sidebar_image_opacity: 0,
  detail_panel_opacity: 0.85,
  control_panel_opacity: 0.85,
  preview_panel_opacity: 0.97,
  preview_image_opacity: 0.7,
  detail_image_opacity: 0,
  component_opacity: 1
}

const MANIFEST_KEY_BY_SETTING: Record<OpacitySettingKey, keyof SkinAppearanceDefaults> = {
  sidebar_panel_opacity: 'sidebarPanelOpacity',
  sidebar_image_opacity: 'sidebarImageOpacity',
  detail_panel_opacity: 'detailPanelOpacity',
  control_panel_opacity: 'controlPanelOpacity',
  preview_panel_opacity: 'previewPanelOpacity',
  preview_image_opacity: 'previewImageOpacity',
  detail_image_opacity: 'detailImageOpacity',
  component_opacity: 'componentOpacity'
}

export function resolveSkinOpacityDefaults(
  skinId: string,
  installedSkins: readonly InstalledSkinAppearance[]
): Record<OpacitySettingKey, number> {
  const skinPackageId = skinId.startsWith('user:') ? skinId.slice('user:'.length) : ''
  const packageDefaults = installedSkins.find((skin) => skin.id === skinPackageId)?.appearanceDefaults
  const resolved = { ...HOST_OPACITY_DEFAULTS }

  if (!packageDefaults) return resolved

  for (const settingKey of OPACITY_SETTING_KEYS) {
    const value = packageDefaults[MANIFEST_KEY_BY_SETTING[settingKey]]
    if (typeof value === 'number' && Number.isFinite(value)) {
      resolved[settingKey] = Math.min(1, Math.max(0, value))
    }
  }

  return resolved
}

export function applySkinWithRecommendedOpacity(
  skinId: string,
  installedSkins: readonly InstalledSkinAppearance[]
) {
  const defaults = resolveSkinOpacityDefaults(skinId, installedSkins)
  return setSkinAppearance(skinId, {
    sidebarPanelOpacity: defaults.sidebar_panel_opacity,
    sidebarImageOpacity: defaults.sidebar_image_opacity,
    detailPanelOpacity: defaults.detail_panel_opacity,
    controlPanelOpacity: defaults.control_panel_opacity,
    previewPanelOpacity: defaults.preview_panel_opacity,
    previewImageOpacity: defaults.preview_image_opacity,
    detailImageOpacity: defaults.detail_image_opacity,
    componentOpacity: defaults.component_opacity
  })
}
