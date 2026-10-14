import { createApp } from 'vue'
import App from './App.vue'
import { startupDebug } from '../lib/startupDebug'
import '../styles/tokens.css'
import '../styles/skins.css'

startupDebug('preview: entry module evaluated')
const app = createApp(App)
startupDebug('preview: Vue app created')
app.mount('#app')
startupDebug('preview: Vue mount returned')
