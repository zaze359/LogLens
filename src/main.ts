import { createApp } from 'vue'
import App from './App.vue'
import './styles/main.css'
import { installGlobalErrorHandlers } from './lib/error-report'

const app = createApp(App)
installGlobalErrorHandlers(app)
app.mount('#app')
