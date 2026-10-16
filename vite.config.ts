import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'
import tailwindcss from '@tailwindcss/vite'
import { fileURLToPath, URL } from 'node:url'

export default defineConfig({
  plugins: [vue(), tailwindcss()],
  base: './',
  clearScreen: false,
  resolve: {
    alias: {
      '@': fileURLToPath(new URL('./src', import.meta.url))
    }
  },
  server: {
    port: 1420,
    strictPort: true,
    watch: {
      // cargo 的 target 目录与 Vite 无关，且 Windows 下编译锁定的 exe 会让 watcher 崩溃
      ignored: ['**/src-tauri/target/**']
    }
  },
  build: {
    target: 'es2022',
    rollupOptions: {
      input: {
        sidebar: fileURLToPath(new URL('./sidebar.html', import.meta.url)),
        detail: fileURLToPath(new URL('./detail.html', import.meta.url)),
        preview: fileURLToPath(new URL('./preview.html', import.meta.url)),
        notifications: fileURLToPath(new URL('./notifications.html', import.meta.url)),
        sidebarNotifications: fileURLToPath(new URL('./sidebar-notifications.html', import.meta.url))
      }
    }
  }
})
