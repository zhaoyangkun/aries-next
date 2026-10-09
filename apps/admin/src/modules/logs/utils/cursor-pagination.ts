/**
 * 运行日志 keyset 游标栈导航：第 1 页无 cursor；「下一页」把当前响应的
 * next_cursor 压栈，「上一页」弹栈。栈顶即当前页请求所用的 cursor，
 * 当前页码 = 栈长 + 1。筛选条件变化时清空栈（回到第 1 页）。
 */

/** 当前页请求应携带的 cursor；第 1 页（栈空）为 undefined。 */
export function cursorForPage(stack: readonly string[]): string | undefined {
  return stack.length > 0 ? stack[stack.length - 1] : undefined
}

/** 当前页码（仅展示用）：栈长 + 1。 */
export function currentPage(stack: readonly string[]): number {
  return stack.length + 1
}

/** 下一页：next_cursor 为 null（已是末页）时不动作，返回是否翻页。 */
export function goToNextPage(stack: string[], nextCursor: string | null): boolean {
  if (!nextCursor)
    return false
  stack.push(nextCursor)
  return true
}

/** 上一页：已在第 1 页（栈空）时不动作，返回是否翻页。 */
export function goToPreviousPage(stack: string[]): boolean {
  if (stack.length === 0)
    return false
  stack.pop()
  return true
}
