import { describe, expect, it } from 'vitest'

import { resolveErrorPage } from '../data/error-pages'

describe('resolveErrorPage', () => {
  it.each([
    ['401', '未登录'],
    ['403', '无权访问'],
    ['404', '页面不存在'],
    ['500', '服务器错误'],
    ['503', '服务暂不可用'],
  ])('resolves supported error code %s', (code, subtitle) => {
    expect(resolveErrorPage(code)).toMatchObject({
      code: Number(code),
      subtitle,
    })
  })

  it.each(['999', 'unexpected', ''])('falls back to 404 for unsupported code %j', (code) => {
    expect(resolveErrorPage(code)).toMatchObject({
      code: 404,
      subtitle: '页面不存在',
    })
  })
})
