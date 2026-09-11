import { createApp } from 'vue'
import App from './App.vue'
import './style.css'
import { installGlobalErrorHandlers } from './api/report-error'

const app = createApp(App)
// 全局错误兜底（V2）必须先于 mount：挂载/首渲染期的异常也要能被捕获上报
installGlobalErrorHandlers(app)
app.mount('#app')
