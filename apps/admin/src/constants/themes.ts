import { MoveHorizontalIcon, UnfoldHorizontalIcon } from '@lucide/vue'

/** tweakcn 主题预设列表（对应 <html> 的 theme-* class 后缀）。 */
export const THEMES = ['zinc', 'red', 'rose', 'orange', 'green', 'blue', 'yellow', 'violet'] as const
export type Theme = typeof THEMES[number]

/** 各主题预设的主色（主题选择器色板展示用）。 */
export const THEME_PRIMARY_COLORS: { theme: Theme, primaryColor: string }[] = [
  { theme: 'zinc', primaryColor: 'oklch(44.2% 0.017 285.786)' },
  { theme: 'red', primaryColor: 'oklch(57.7% 0.245 27.325)' },
  { theme: 'rose', primaryColor: 'oklch(0.645 0.246 16.439)' },
  { theme: 'orange', primaryColor: 'oklch(0.705 0.213 47.604)' },
  { theme: 'green', primaryColor: 'oklch(0.723 0.219 149.579)' },
  { theme: 'blue', primaryColor: 'oklch(56% 0.215 277)' },
  { theme: 'yellow', primaryColor: 'oklch(68.1% 0.162 75.834)' },
  { theme: 'violet', primaryColor: 'oklch(0.606 0.25 292.717)' },
] as const

export type Radius = typeof RADIUS[number]
/** 可选圆角档位（rem），写入 CSS 变量 --radius。 */
export const RADIUS = [0, 0.25, 0.5, 0.75, 1] as const

export type ContentLayout = 'full' | 'centered'
/** 可选内容区布局及其中文标签。 */
export const CONTENT_LAYOUTS = [
  { label: '全宽', value: 'full', icon: UnfoldHorizontalIcon },
  { label: '居中', value: 'centered', icon: MoveHorizontalIcon },
] as const
