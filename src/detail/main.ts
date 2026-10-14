import { createApp } from 'vue'
import App from './App.vue'
import { startupDebug } from '../lib/startupDebug'
import '../styles/tokens.css'
import '../styles/skins.css'

startupDebug('detail: entry module evaluated')
const app = createApp(App)
startupDebug('detail: Vue app created')
app.mount('#app')
startupDebug('detail: Vue mount returned')
