import { afterEach, describe, expect, it, vi } from 'vitest'
import { AxiosError, type AxiosResponse, type InternalAxiosRequestConfig } from 'axios'
import { api, safeInternalPath, setUnauthorizedHandler } from './client'

// 用自定义 Adapter 捕获 transformRequest 之后的最终请求体，验证序列化行为。
function captureAdapter(captured: { data?: unknown }) {
  return async (config: InternalAxiosRequestConfig): Promise<AxiosResponse> => {
    captured.data = config.data
    return { data: null, status: 200, statusText: 'OK', headers: {}, config }
  }
}

// 返回固定状态码的 Adapter，用于验证响应拦截行为。
function statusAdapter(status: number) {
  return async (config: InternalAxiosRequestConfig): Promise<AxiosResponse> => {
    const response: AxiosResponse = {
      data: { error: { code: 'TEST', message: 'test' } },
      status,
      statusText: 'ERR',
      headers: {},
      config,
    }
    if (status >= 400) throw new AxiosError('test', undefined, config, undefined, response)
    return response
  }
}

describe('api client', () => {
  it('keeps FormData payloads as multipart instead of JSON-stringifying them', async () => {
    const captured: { data?: unknown } = {}
    const form = new FormData()
    form.append('file[]', new File([new Uint8Array([1, 2, 3])], 'a.png'))

    await api.post('/api/admin/media', form, { adapter: captureAdapter(captured) })

    // 回归：实例若预设 Content-Type: application/json，Axios 会把 FormData 序列化成
    // JSON 字符串，Backend 因缺少 multipart boundary 返回 400，前端只能显示“文件上传失败”。
    expect(captured.data).toBeInstanceOf(FormData)
  })

  it('still serializes plain object payloads as JSON', async () => {
    const captured: { data?: unknown } = {}

    await api.post(
      '/api/admin/auth/login',
      { login: 'admin', password: 'secret' },
      { adapter: captureAdapter(captured) },
    )

    expect(typeof captured.data).toBe('string')
    expect(JSON.parse(captured.data as string)).toEqual({ login: 'admin', password: 'secret' })
  })
})

describe('unauthorized interceptor', () => {
  afterEach(() => {
    setUnauthorizedHandler(null)
  })

  it('invokes the registered handler when a response is 401', async () => {
    const handler = vi.fn()
    setUnauthorizedHandler(handler)

    await expect(
      api.get('/api/admin/session', { adapter: statusAdapter(401) }),
    ).rejects.toBeInstanceOf(AxiosError)
    expect(handler).toHaveBeenCalledTimes(1)
  })

  it('does not invoke the handler for non-401 errors', async () => {
    const handler = vi.fn()
    setUnauthorizedHandler(handler)

    await expect(
      api.get('/api/admin/session', { adapter: statusAdapter(500) }),
    ).rejects.toBeInstanceOf(AxiosError)
    await expect(
      api.get('/api/admin/session', { adapter: statusAdapter(403) }),
    ).rejects.toBeInstanceOf(AxiosError)
    expect(handler).not.toHaveBeenCalled()
  })

  it('does not invoke any handler after it is unregistered', async () => {
    setUnauthorizedHandler(() => {})
    setUnauthorizedHandler(null)

    await expect(
      api.get('/api/admin/session', { adapter: statusAdapter(401) }),
    ).rejects.toBeInstanceOf(AxiosError)
  })
})

describe('safeInternalPath', () => {
  it('accepts internal paths and rejects external ones', () => {
    expect(safeInternalPath('/articles?draft=1')).toBe('/articles?draft=1')
    expect(safeInternalPath('/')).toBe('/')
    // 协议相对地址与绝对地址都是开放重定向风险，回退到首页
    expect(safeInternalPath('//evil.example.com')).toBe('/')
    expect(safeInternalPath('https://evil.example.com')).toBe('/')
    expect(safeInternalPath('')).toBe('/')
  })
})
