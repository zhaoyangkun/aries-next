// 将 Vditor 运行期按需加载的静态资产（lute 引擎、highlight.js、KaTeX、icons、i18n、
// 预览主题 CSS、emoji 图片）从 node_modules 拷到 public/vditor/dist，配合编辑器
// 初始化时的 `cdn: '/vditor'` 选项完全本地化——默认 CDN（unpkg）在国内不稳定，
// 会导致编辑器卡在加载页。
// 由 package.json 的 postinstall 触发；跳过图表类等极重且极少用的库
// （mermaid / mathjax / echarts / flowchart 等），用到时回退 CDN 行为与修复前一致。

import { cpSync, existsSync, mkdirSync, rmSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const packageRoot = join(dirname(fileURLToPath(import.meta.url)), '..')
const distDir = join(packageRoot, 'node_modules', 'vditor', 'dist')
const targetDir = join(packageRoot, 'public', 'vditor', 'dist')

// 相对 vditor/dist 的资产清单
const ASSETS = [
  'js/lute',
  'js/highlight.js',
  'js/icons',
  'js/i18n',
  'js/katex',
  'css',
  'images',
]

if (!existsSync(distDir)) {
  console.warn('[copy-vditor-assets] vditor/dist 不存在，跳过（可能未安装依赖）')
  process.exit(0)
}

rmSync(targetDir, { recursive: true, force: true })
mkdirSync(targetDir, { recursive: true })

for (const asset of ASSETS) {
  cpSync(join(distDir, asset), join(targetDir, asset), { recursive: true })
}

console.log(`[copy-vditor-assets] ${ASSETS.length} 项资产已拷贝到 public/vditor/dist`)
