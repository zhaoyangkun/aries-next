import { storeToRefs } from 'pinia'
import { watch } from 'vue'

import { THEMES } from '@/constants/themes'
import { useThemeStore } from '@/stores/theme'

/**
 * 主题/圆角应用到 DOM 的 Composable：watch 变化并同步 <html> 的 theme-* class 与 --radius 变量，
 * 返回响应式值与 setter 供主题切换面板使用。
 */
export function useSystemTheme() {
  const themeStore = useThemeStore()
  const { setTheme, setRadius } = themeStore
  const { theme, radius } = storeToRefs(themeStore)

  if (typeof document !== 'undefined') {
    watch(theme, (theme) => {
      document.documentElement.classList.remove(...THEMES.map(t => `theme-${t}`))
      document.documentElement.classList.add(`theme-${theme}`)
    }, { immediate: true })

    watch(radius, (radius) => {
      document.documentElement.style.setProperty('--radius', `${radius}rem`)
    }, { immediate: true })
  }

  return {
    theme,
    radius,
    setTheme,
    setRadius,
  }
}
