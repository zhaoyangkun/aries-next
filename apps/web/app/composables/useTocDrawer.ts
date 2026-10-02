// 移动端目录抽屉的开关状态 + 当前文章是否有目录：
// 抽屉由文章详情页挂载，浮动工具栏在任意页面都可能需要唤出它，状态共享在 useState 中。
// hasToc 由文章页的目录组件（ArticleToc）在提取到标题后置位，离开文章页时复位。
export function useTocDrawer() {
  const open = useState('toc-drawer-open', () => false)
  const hasToc = useState('article-has-toc', () => false)

  function show() {
    open.value = true
  }

  function hide() {
    open.value = false
  }

  return { open, hasToc, show, hide }
}
