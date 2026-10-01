// 旧版 Aries 的分页路由 /p/:page，301 跳转到新版 /?page=N
export default defineEventHandler((event) => {
  const raw = getRouterParam(event, 'page') ?? ''
  const page = Number(raw)
  const target = Number.isInteger(page) && page > 1 ? `/?page=${page}` : '/'
  return sendRedirect(event, target, 301)
})
