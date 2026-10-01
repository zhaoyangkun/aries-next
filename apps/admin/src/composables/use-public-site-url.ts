import { ref } from 'vue'

import { siteSettingsApi } from '@/modules/settings/api/settings'

// header「打开公开站」入口的跳转地址：运行期从站点设置 site_url 读取（Admin 为纯 SPA，
// 无 SSR，setup 阶段直接发起请求即可），构建产物不含任何域名——换域名只需在
// Admin「站点设置」修改 site_url，无需重新构建；未配置或后端不可用时保持空串，由调用方隐藏入口。
export function usePublicSiteUrl() {
  const publicSiteUrl = ref('')
  siteSettingsApi
    .get()
    .then((settings) => {
      publicSiteUrl.value = settings.site_url ?? ''
    })
    .catch(() => {
      // 保持空串：隐藏入口
    })
  return publicSiteUrl
}
