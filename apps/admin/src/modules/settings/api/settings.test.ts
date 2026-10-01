import { afterEach, describe, expect, it, vi } from 'vitest'
import { api } from '@/shared/api/client'
import {
  settingsGroupApi,
  siteSettingsApi,
  type AppearanceSettings,
  type EmailSettingsView,
  type SettingGroupResponse,
} from './settings'

const siteSettings = {
  site_name: 'Aries',
  site_description: '',
  site_url: '',
  logo_url: '',
  icp_text: '',
  default_cover_url: '',
  page_size_index: 10,
  page_size_archive: 20,
  page_size_search: 20,
  comment_policy: 'moderated' as const,
  comments_per_page: 20,
  updated_at: '2026-08-05T12:00:00Z',
}

const emailResponse: SettingGroupResponse<EmailSettingsView> = {
  group: 'email',
  version: 3,
  updated_at: '2026-08-05T12:00:00Z',
  settings: {
    enabled: true,
    smtp_host: 'smtp.example.com',
    smtp_port: 465,
    smtp_username: 'noreply@example.com',
    smtp_password_set: true,
    from_address: 'noreply@example.com',
    from_name: 'Aries',
  },
}

afterEach(() => {
  vi.restoreAllMocks()
})

describe('siteSettingsApi', () => {
  it('reads and replaces site settings', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({ data: siteSettings })
    const put = vi.spyOn(api, 'put').mockResolvedValue({ data: siteSettings })
    const { updated_at: _, ...payload } = siteSettings

    await expect(siteSettingsApi.get()).resolves.toEqual(siteSettings)
    await expect(siteSettingsApi.update(payload)).resolves.toEqual(siteSettings)
    expect(get).toHaveBeenCalledWith('/api/admin/site-settings')
    expect(put).toHaveBeenCalledWith('/api/admin/site-settings', payload)
  })
})

describe('settingsGroupApi', () => {
  it('reads and updates the appearance group with optimistic locking', async () => {
    const appearance: SettingGroupResponse<AppearanceSettings> = {
      group: 'appearance',
      version: 2,
      updated_at: '2026-08-05T12:00:00Z',
      settings: { logo_url: '/logo.png', favicon_url: null, color_scheme: 'dark', list_density: 'compact' },
    }
    const get = vi.spyOn(api, 'get').mockResolvedValue({ data: appearance })
    const put = vi.spyOn(api, 'put').mockResolvedValue({ data: { ...appearance, version: 3 } })

    await expect(settingsGroupApi.getAppearance()).resolves.toEqual(appearance)
    await settingsGroupApi.updateAppearance(2, appearance.settings)
    expect(get).toHaveBeenCalledWith('/api/admin/settings/appearance')
    expect(put).toHaveBeenCalledWith('/api/admin/settings/appearance', {
      expected_version: 2,
      settings: appearance.settings,
    })
  })

  it('serializes email secret three-state semantics', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({ data: emailResponse })
    const put = vi.spyOn(api, 'put').mockResolvedValue({ data: emailResponse })
    const base = {
      enabled: true,
      smtp_host: 'smtp.example.com',
      smtp_port: 465,
      smtp_username: 'u',
      from_address: 'a@example.com',
      from_name: 'Aries',
    }

    await expect(settingsGroupApi.getEmail()).resolves.toEqual(emailResponse)
    expect(get).toHaveBeenCalledWith('/api/admin/settings/email')

    // 缺省密码字段表示保持不变。
    await settingsGroupApi.updateEmail(3, base)
    const keepBody = put.mock.calls[0]?.[1] as { settings: Record<string, unknown> }
    expect(keepBody.settings).not.toHaveProperty('smtp_password')

    // 空串表示清除，非空表示更新。
    await settingsGroupApi.updateEmail(3, { ...base, smtp_password: '' })
    expect(put).toHaveBeenLastCalledWith('/api/admin/settings/email', {
      expected_version: 3,
      settings: { ...base, smtp_password: '' },
    })
    await settingsGroupApi.updateEmail(3, { ...base, smtp_password: 'new-secret' })
    expect(put).toHaveBeenLastCalledWith('/api/admin/settings/email', {
      expected_version: 3,
      settings: { ...base, smtp_password: 'new-secret' },
    })
  })

  it('reads and updates the integrations group', async () => {
    const integrations = {
      group: 'integrations',
      version: 1,
      updated_at: '2026-08-05T12:00:00Z',
      settings: { analytics_id: 'G-XXXX', site_verification_token: null },
    }
    const get = vi.spyOn(api, 'get').mockResolvedValue({ data: integrations })
    const put = vi.spyOn(api, 'put').mockResolvedValue({ data: { ...integrations, version: 2 } })

    await expect(settingsGroupApi.getIntegrations()).resolves.toEqual(integrations)
    await settingsGroupApi.updateIntegrations(1, integrations.settings)
    expect(get).toHaveBeenCalledWith('/api/admin/settings/integrations')
    expect(put).toHaveBeenCalledWith('/api/admin/settings/integrations', {
      expected_version: 1,
      settings: integrations.settings,
    })
  })

  it('reads and updates the ai group with api_key three-state semantics', async () => {
    const aiResponse = {
      group: 'ai',
      version: 4,
      updated_at: '2026-08-05T12:00:00Z',
      settings: {
        enabled: true,
        protocol: 'anthropic',
        base_url: 'https://api.example.com/v1',
        model: 'gpt-test',
        api_key_set: true,
        features: { editor_assist: true, comment_moderation: false },
      },
    }
    const get = vi.spyOn(api, 'get').mockResolvedValue({ data: aiResponse })
    const put = vi.spyOn(api, 'put').mockResolvedValue({ data: aiResponse })
    const base = {
      enabled: true,
      protocol: 'anthropic' as const,
      base_url: 'https://api.example.com/v1',
      model: 'gpt-test',
      features: { editor_assist: true, comment_moderation: false },
    }

    await expect(settingsGroupApi.getAi()).resolves.toEqual(aiResponse)
    expect(get).toHaveBeenCalledWith('/api/admin/settings/ai')

    // protocol 随常规字段读写；缺省 api_key 表示保持不变。
    await settingsGroupApi.updateAi(4, base)
    const keepBody = put.mock.calls[0]?.[1] as { settings: Record<string, unknown> }
    expect(keepBody.settings).not.toHaveProperty('api_key')
    expect(keepBody.settings.protocol).toBe('anthropic')

    // 空串清除，非空更新。
    await settingsGroupApi.updateAi(4, { ...base, api_key: '' })
    expect(put).toHaveBeenLastCalledWith('/api/admin/settings/ai', {
      expected_version: 4,
      settings: { ...base, api_key: '' },
    })
    await settingsGroupApi.updateAi(4, { ...base, api_key: 'sk-new' })
    expect(put).toHaveBeenLastCalledWith('/api/admin/settings/ai', {
      expected_version: 4,
      settings: { ...base, api_key: 'sk-new' },
    })
  })
})
