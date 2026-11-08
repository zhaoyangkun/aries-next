import dayjs from 'dayjs'
import relativeTime from 'dayjs/plugin/relativeTime'
import 'dayjs/locale/zh-cn'

/** dayjs 全局配置：中文 locale 与 relativeTime 插件。 */
export function setupDayjs() {
  dayjs.locale('zh-cn')
  dayjs.extend(relativeTime)
}
