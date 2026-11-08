import type { App } from 'vue'

import { createI18n } from 'vue-i18n'

import type { Language } from '.'

import { DEFAULT_LOCALE } from '.'
import zh from './zh.json'

/** 创建 vue-i18n 实例（仅简体中文，见包约定）并挂载到应用。 */
export function setupI18n(app: App) {
  const i18n = createI18n({
    legacy: false,
    locale: DEFAULT_LOCALE,
    fallbackLocale: DEFAULT_LOCALE,
    messages: <Record<Language, Record<string, any>>>{
      zh,
    },
  })
  app.use(i18n)
}
