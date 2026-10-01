import { api } from '@/shared/api/client'

export type CommentPolicySetting = 'closed' | 'moderated' | 'auto_approve'

export interface SiteSettings {
  site_name: string
  site_description: string
  site_url: string
  logo_url: string
  icp_text: string
  default_cover_url: string
  page_size_index: number
  page_size_archive: number
  page_size_search: number
  comment_policy: CommentPolicySetting
  comments_per_page: number
  updated_at: string
}

export type UpdateSiteSettingsPayload = Omit<SiteSettings, 'updated_at'>

// 站点基础信息沿用独立的 /api/admin/site-settings 端点（不在 setting_groups 三组之内）。
export const siteSettingsApi = {
  async get() {
    const { data } = await api.get<SiteSettings>('/api/admin/site-settings')
    return data
  },

  async update(payload: UpdateSiteSettingsPayload) {
    const { data } = await api.put<SiteSettings>('/api/admin/site-settings', payload)
    return data
  },
}

export type SettingGroupName = 'appearance' | 'email' | 'integrations' | 'ai'

export type ColorSchemePreference = 'system' | 'light' | 'dark'
export type ListDensity = 'comfortable' | 'compact'

// 与 backend/core/src/settings.rs 的结构化字段一一对应；Optional 字段允许 jsonb 缺省。
export interface AppearanceSettings {
  logo_url: string | null
  favicon_url: string | null
  color_scheme: ColorSchemePreference | null
  list_density: ListDensity | null
}

// 邮件设置的读取视图：smtp_password 永不回读，只回是否已设置。
export interface EmailSettingsView {
  enabled: boolean
  smtp_host: string | null
  smtp_port: number | null
  smtp_username: string | null
  smtp_password_set: boolean
  from_address: string | null
  from_name: string | null
}

// 邮件设置更新入参：smtp_password 三态——缺省/null 保持不变，空串清除，非空更新。
export interface EmailSettingsUpdate {
  enabled: boolean
  smtp_host: string | null
  smtp_port: number | null
  smtp_username: string | null
  smtp_password?: string | null
  from_address: string | null
  from_name: string | null
}

export interface IntegrationSettings {
  analytics_id: string | null
  site_verification_token: string | null
}

// AI Provider 协议：anthropic 时 base_url 可省略（Backend 默认 https://api.anthropic.com）。
export type AiProtocol = 'openai' | 'anthropic'

// AI 功能开关：editor_assist 控制 rewrite/summary/metadata，comment_moderation 控制评论自动审核。
export interface AiFeatureToggles {
  editor_assist: boolean
  comment_moderation: boolean
}

// AI 设置的读取视图：api_key 永不回读，只回是否已设置。
export interface AiSettingsView {
  enabled: boolean
  protocol: AiProtocol
  base_url: string | null
  model: string | null
  api_key_set: boolean
  features: AiFeatureToggles
}

// AI 设置更新入参：api_key 三态——缺省/null 保持不变，空串清除，非空更新。
export interface AiSettingsUpdate {
  enabled: boolean
  protocol: AiProtocol
  base_url: string | null
  model: string | null
  api_key?: string | null
  features: AiFeatureToggles
}

export interface SettingGroupResponse<T> {
  group: SettingGroupName
  version: number
  updated_at: string
  settings: T
}

// 分组设置（appearance/email/integrations/ai，仅 owner 可写）：
// 更新必须携带 expected_version 做乐观锁，过期提交 Backend 返回 409。
export const settingsGroupApi = {
  async getAppearance() {
    const { data } = await api.get<SettingGroupResponse<AppearanceSettings>>('/api/admin/settings/appearance')
    return data
  },

  async updateAppearance(expectedVersion: number, settings: AppearanceSettings) {
    const { data } = await api.put<SettingGroupResponse<AppearanceSettings>>('/api/admin/settings/appearance', {
      expected_version: expectedVersion,
      settings,
    })
    return data
  },

  async getEmail() {
    const { data } = await api.get<SettingGroupResponse<EmailSettingsView>>('/api/admin/settings/email')
    return data
  },

  async updateEmail(expectedVersion: number, settings: EmailSettingsUpdate) {
    const { data } = await api.put<SettingGroupResponse<EmailSettingsView>>('/api/admin/settings/email', {
      expected_version: expectedVersion,
      settings,
    })
    return data
  },

  async getIntegrations() {
    const { data } = await api.get<SettingGroupResponse<IntegrationSettings>>('/api/admin/settings/integrations')
    return data
  },

  async updateIntegrations(expectedVersion: number, settings: IntegrationSettings) {
    const { data } = await api.put<SettingGroupResponse<IntegrationSettings>>('/api/admin/settings/integrations', {
      expected_version: expectedVersion,
      settings,
    })
    return data
  },

  async getAi() {
    const { data } = await api.get<SettingGroupResponse<AiSettingsView>>('/api/admin/settings/ai')
    return data
  },

  async updateAi(expectedVersion: number, settings: AiSettingsUpdate) {
    const { data } = await api.put<SettingGroupResponse<AiSettingsView>>('/api/admin/settings/ai', {
      expected_version: expectedVersion,
      settings,
    })
    return data
  },
}
