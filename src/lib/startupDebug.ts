import { invoke } from '@tauri-apps/api/core'

export function startupDebug(stage: string): void {
  console.info(`[startup] ${stage}`)
  void invoke('startup_frontend_log', { stage }).catch(() => {})
}
