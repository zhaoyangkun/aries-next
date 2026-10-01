// 昼夜主题切换：偏好存 localStorage，未设置时跟随 prefers-color-scheme。
// 防 SSR 首屏闪烁的做法：在 <head> 内联一段同步执行的 init script，首帧绘制前
// 就给 <html> 挂上 dark class；因此图标显隐也直接由 html.dark 的 CSS 控制，
// 不经过 Vue 状态，避免 Hydration 前后不一致。
const THEME_INIT_SCRIPT = `(function(){try{var t=localStorage.getItem('aries-theme');if(t!=='light'&&t!=='dark'){t=window.matchMedia('(prefers-color-scheme: dark)').matches?'dark':'light';}document.documentElement.classList.toggle('dark',t==='dark');}catch(e){}})();`

export function useTheme() {
  useHead({
    script: [{ key: 'theme-init', innerHTML: THEME_INIT_SCRIPT }],
  })

  function toggleTheme() {
    if (!import.meta.client) return
    const dark = !document.documentElement.classList.contains('dark')
    document.documentElement.classList.toggle('dark', dark)
    try {
      localStorage.setItem('aries-theme', dark ? 'dark' : 'light')
    }
    catch {
      // 隐私模式下 localStorage 可能不可用，主题仅在当前会话生效
    }
  }

  return { toggleTheme }
}
