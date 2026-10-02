// 全局搜索弹层（Ctrl/⌘+K）的开关状态：布局挂载 SearchPalette，
// 浮动工具栏与布局的快捷键监听都要能打开它，因此状态放在共享 useState 中。
export function useSearchPalette() {
  const open = useState('search-palette-open', () => false)

  function show() {
    open.value = true
  }

  function hide() {
    open.value = false
  }

  return { open, show, hide }
}
