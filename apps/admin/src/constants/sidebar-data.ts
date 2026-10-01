import {
  FileIcon,
  FileTextIcon,
  FolderTreeIcon,
  ImageIcon,
  ImagesIcon,
  LayoutDashboardIcon,
  Link2Icon,
  MailIcon,
  MessageSquareIcon,
  MonitorCogIcon,
  NavigationIcon,
  NotebookPenIcon,
  PaletteIcon,
  PlugIcon,
  ScrollTextIcon,
  SettingsIcon,
  SparklesIcon,
  SquareTerminalIcon,
  TagIcon,
  UserIcon,
} from '@lucide/vue'

import type { NavGroup, NavItem } from '@/components/app-sidebar/types'

type SidebarLink = Extract<NavItem, { url: string }>

// 设置子导航：settings 壳（settings-aside）与侧边栏共用。
export const settingsNavItems: SidebarLink[] = [
  { title: '个人资料', url: '/settings/', icon: UserIcon, permission: 'profile:manage' },
  { title: '外观', url: '/settings/appearance', icon: PaletteIcon, permission: 'profile:manage' },
  { title: '站点设置', url: '/settings/site', icon: MonitorCogIcon, permission: 'settings:manage' },
  { title: '邮件设置', url: '/settings/email', icon: MailIcon, permission: 'settings:manage' },
  { title: '第三方集成', url: '/settings/integrations', icon: PlugIcon, permission: 'settings:manage' },
  { title: 'AI 设置', url: '/settings/ai', icon: SparklesIcon, permission: 'settings:manage' },
]

// 主导航：permission 控制可见性（后端仍以 Permission Guard 为准）。
// 分组参考旧版 Aries 后台（d2-admin）侧边栏：文章 / 外观 / 用户(互动) / 系统。
export const navData: NavGroup[] = [
  {
    title: '工作台',
    items: [
      { title: '概览', url: '/dashboard', icon: LayoutDashboardIcon, permission: 'dashboard:view' },
    ],
  },
  {
    title: '内容',
    items: [
      {
        title: '文章',
        icon: FileTextIcon,
        permission: 'content:manage',
        items: [
          { title: '文章', url: '/articles', icon: FileTextIcon, permission: 'content:manage' },
          { title: '文章分类', url: '/categories', icon: FolderTreeIcon, permission: 'content:manage' },
          { title: '标签', url: '/tags', icon: TagIcon, permission: 'content:manage' },
        ],
      },
      {
        title: '外观',
        icon: PaletteIcon,
        items: [
          { title: '导航菜单', url: '/navigation', icon: NavigationIcon, permission: 'content:manage' },
          { title: '图库', url: '/galleries', icon: ImagesIcon, permission: 'content:manage' },
          { title: '外观设置', url: '/settings/appearance', icon: PaletteIcon, permission: 'profile:manage' },
        ],
      },
      {
        title: '互动',
        icon: MessageSquareIcon,
        items: [
          { title: '评论', url: '/comments', icon: MessageSquareIcon, permission: 'comments:moderate' },
          { title: '页面', url: '/pages', icon: FileIcon, permission: 'content:manage' },
          { title: '日志', url: '/journals', icon: NotebookPenIcon, permission: 'content:manage' },
          { title: '友情链接', url: '/links', icon: Link2Icon, permission: 'content:manage' },
        ],
      },
    ],
  },
  {
    title: '系统',
    items: [
      { title: '媒体库', url: '/media', icon: ImageIcon, permission: 'content:manage' },
      { title: '审计日志', url: '/audit-logs', icon: ScrollTextIcon, permission: 'settings:manage' },
      { title: '运行日志', url: '/logs', icon: SquareTerminalIcon, permission: 'settings:manage' },
      { title: '设置', items: settingsNavItems, icon: SettingsIcon, permission: 'profile:manage' },
    ],
  },
]
