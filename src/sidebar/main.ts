import { createApp } from 'vue'
import App from './App.vue'
import { startupDebug } from '../lib/startupDebug'
import '../styles/tokens.css'
import '../styles/skins.css'

startupDebug('sidebar: entry module evaluated')
const app = createApp(App)
startupDebug('sidebar: Vue app created')
app.mount('#app')
startupDebug('sidebar: Vue mount returned')
