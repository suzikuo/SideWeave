import { ref } from 'vue'
import type { PluginMeta, Settings } from './types'

/** 详情页与控制中心共享的宿主状态；侧边栏只通过状态快照同步 */
export const settingsRef = ref<Settings | null>(null)
export const pluginsRef = ref<PluginMeta[]>([])
export const autostartRef = ref(false)
export const appVersionRef = ref('')
export const dataDirRef = ref('')
