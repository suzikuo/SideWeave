import { invoke } from '@tauri-apps/api/core'

const DEFAULT_TIMEOUT_MS = 8_000

interface PluginResponse<T> {
  request_id: string
  plugin_id: string
  ok: boolean
  result: T
  error?: { code: string; message: string } | null
}

function requestId(): string {
  return globalThis.crypto?.randomUUID?.() ?? `req-${Date.now()}-${Math.random()}`
}

function clonePluginPayload(payload: unknown): unknown {
  if (payload === undefined || payload === null) return payload
  try {
    const serialized = JSON.stringify(payload)
    return serialized === undefined ? null : JSON.parse(serialized)
  } catch {
    throw new Error('插件请求参数不可序列化')
  }
}

export async function pluginCall<T>(
  pluginId: string,
  method: string,
  payload: unknown = {},
  timeoutMs = DEFAULT_TIMEOUT_MS
) {
  const safePayload = clonePluginPayload(payload)
  const request = {
    request_id: requestId(),
    plugin_id: pluginId,
    kind: 'call',
    method,
    payload: safePayload,
    timeout_ms: timeoutMs
  }
  const response = await invoke<PluginResponse<T>>('plugin_call', {
    request
  })
  if (!response.ok) {
    throw new Error(response.error?.message ?? `插件调用失败: ${pluginId}/${method}`)
  }
  return response.result
}
