interface ErrorPage {
  code: number
  subtitle: string
  error: string
}

type ErrorCode = '401' | '403' | '404' | '500' | '503'
type ErrorPageContent = Omit<ErrorPage, 'code'>

const FALLBACK_ERROR_CODE: ErrorCode = '404'

const ERROR_PAGE_MAP: Record<ErrorCode, ErrorPageContent> = {
  401: {
    subtitle: '未登录',
    error: '请先登录后再访问该页面。',
  },
  403: {
    subtitle: '无权访问',
    error: '当前账号没有访问该页面的权限。',
  },
  404: {
    subtitle: '页面不存在',
    error: '你要找的页面可能已被移除、更名或暂时不可用。',
  },
  500: {
    subtitle: '服务器错误',
    error: '服务器遇到了意外情况，请稍后重试。',
  },
  503: {
    subtitle: '服务暂不可用',
    error: '后端服务暂时不可用，请确认服务已启动后重试。',
  },
}

function isErrorCode(code: string): code is ErrorCode {
  return Object.hasOwn(ERROR_PAGE_MAP, code)
}

export function resolveErrorPage(code: string): ErrorPage {
  const resolvedCode = isErrorCode(code) ? code : FALLBACK_ERROR_CODE

  return {
    code: Number(resolvedCode),
    ...ERROR_PAGE_MAP[resolvedCode],
  }
}
