// 社交链接图标：白名单映射图标键 → 内联 SVG 子节点（lucide 风格描边，视觉统一）。
// 未知/缺失键一律退化为通用链接图标；键值绝不直接注入 markup（Owner 配置但属于用户输入）。
const GITHUB = '<path d="M15 22v-4a4.8 4.8 0 0 0-1-3.5c3 0 6-2 6-5.5.08-1.25-.27-2.48-1-3.5.28-1.15.28-2.35 0-3.5 0 0-1 0-3 1.5-2.64-.5-5.36-.5-8 0C6 2 5 2 5 2c-.3 1.15-.3 2.35 0 3.5A5.403 5.403 0 0 0 4 9c0 3.5 3 5.5 6 5.5-.39.49-.68 1.05-.85 1.65-.17.6-.22 1.23-.15 1.85v4"/><path d="M9 18c-4.51 2-5-2-7-2"/>'
const GIT_BRANCH = '<path d="M6 3v12"/><circle cx="18" cy="6" r="3"/><circle cx="6" cy="18" r="3"/><path d="M18 9a9 9 0 0 1-9 9"/>'
const AT_SIGN = '<circle cx="12" cy="12" r="4"/><path d="M16 8v5a3 3 0 0 0 6 0v-1a10 10 0 1 0-4 8"/>'
const CIRCLE_HELP = '<circle cx="12" cy="12" r="10"/><path d="M9.09 9a3 3 0 0 1 5.83 1c0 2-3 3-3 3"/><path d="M12 17h.01"/>'
const TWITTER = '<path d="M22 4s-.7 2.1-2 3.4c1.6 10-9.4 17.3-18 11.6 2.2.1 4.4-.6 6-2C3 15.5.5 9.6 3 5c2.2 2.6 5.6 4.1 9 4-.9-4.2 4-6.6 7-3.8 1.1 0 3-1.2 3-1.2z"/>'
const TV = '<rect width="20" height="14" x="2" y="7" rx="2" ry="2"/><path d="M17 2v5M7 2v5"/>'
const MESSAGE_CIRCLE = '<path d="M7.9 20A9 9 0 1 0 4 16.1L2 22Z"/>'
const MESSAGE_SQUARE = '<path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"/>'
const SEND = '<path d="m22 2-7 20-4-9-9-4Z"/><path d="M22 2 11 13"/>'
const LINK = '<path d="M10 13a5 5 0 0 0 7.54.54l3-3a5 5 0 0 0-7.07-7.07l-1.72 1.71"/><path d="M14 11a5 5 0 0 0-7.54-.54l-3 3a5 5 0 0 0 7.07 7.07l1.71-1.71"/>'

// 与 Admin「社交链接」页的图标选项一一对应；新增键时同步两处。
const ICONS: Record<string, string> = {
  github: GITHUB,
  gitee: GIT_BRANCH,
  gitlab: GIT_BRANCH,
  weibo: AT_SIGN,
  zhihu: CIRCLE_HELP,
  x: TWITTER,
  bilibili: TV,
  wechat: MESSAGE_CIRCLE,
  qq: MESSAGE_SQUARE,
  telegram: SEND,
  link: LINK,
}

export const SOCIAL_ICON_KEYS = Object.keys(ICONS)

/** 取图标键对应的内联 SVG 子节点；未知键退化为通用链接图标。 */
export function socialIconMarkup(icon: string | null | undefined): string {
  const markup = icon ? ICONS[icon] : undefined
  return markup ?? LINK
}
