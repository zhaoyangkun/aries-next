import nprogress from 'nprogress'
import 'nprogress/nprogress.css'

import '@/assets/nprogress.css'

/** 配置 NProgress 顶部进度条（路由切换时由通用守卫驱动）。 */
export function setupNProgress() {
  nprogress.configure({
    showSpinner: true,
    speed: 500,
    trickleSpeed: 200,
  })
}
