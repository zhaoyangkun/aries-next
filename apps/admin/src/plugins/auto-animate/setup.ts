import type { App } from 'vue'

import { autoAnimatePlugin } from '@formkit/auto-animate/vue'

/** 注册 @formkit/auto-animate 的 v-auto-animate 指令，提供模态/下拉过渡动画。 */
export function setupAutoAnimate(app: App) {
  app.use(autoAnimatePlugin)
}
