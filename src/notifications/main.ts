import { createApp } from 'vue'
import Notifications from './Notifications.vue'
import { bindSettings } from '../lib/theme'
import '../styles/tokens.css'
import '../styles/skins.css'

void bindSettings(() => {}).catch((error) => {
  console.error('[host-notifications] failed to apply host skin', error)
})

createApp(Notifications).mount('#app')
