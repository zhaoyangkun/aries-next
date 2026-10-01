export function validateRequired(value: string, fieldName: string) {
  return value.trim() ? '' : `请输入${fieldName}`
}

export function validateUsername(value: string) {
  const username = value.trim()
  if (!username) return '请输入用户名'
  if (!/^[A-Za-z0-9_-]{3,30}$/.test(username)) {
    return '使用 3 至 30 个字母、数字、短横线或下划线'
  }
  return ''
}

export function validateEmail(value: string) {
  const email = value.trim()
  if (!email) return '请输入邮箱'
  const [local, domain, ...rest] = email.split('@')
  if (
    rest.length > 0 ||
    !local ||
    !domain ||
    !domain.includes('.') ||
    email.length > 254 ||
    /\s/.test(email)
  ) {
    return '请输入有效的邮箱地址'
  }
  return ''
}

export function validateDisplayName(value: string) {
  const length = Array.from(value.trim()).length
  if (length === 0) return '请输入显示名称'
  if (length > 60) return '显示名称不能超过 60 个字符'
  return ''
}

export function validatePassword(value: string) {
  const length = Array.from(value).length
  if (length < 10 || length > 128) return '密码长度需要为 10 至 128 个字符'
  if (!/\p{L}/u.test(value) || !/\d/.test(value)) {
    return '密码需要同时包含字母和数字'
  }
  return ''
}

export function validateAvatarUrl(value: string) {
  const avatarUrl = value.trim()
  if (!avatarUrl) return ''
  if (avatarUrl.length > 2048) return '头像 URL 不能超过 2048 个字符'
  try {
    const url = new URL(avatarUrl)
    return url.protocol === 'http:' || url.protocol === 'https:'
      ? ''
      : '头像 URL 需要使用 HTTP 或 HTTPS'
  } catch {
    return '请输入有效的头像 URL'
  }
}
