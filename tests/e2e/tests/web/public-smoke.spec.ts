import { expect, test } from '@playwright/test'

// 公开站冒烟：首页 SSR 可访问且文章列表区块渲染（无数据时显示空态，同样算通过）。
test('公开站首页可访问并渲染文章列表区块', async ({ page }) => {
  const response = await page.goto('/')
  expect(response, '首页应有响应').toBeTruthy()
  expect(response!.status(), '首页应返回成功状态码').toBeLessThan(400)
  await expect(page.locator('#article-list')).toBeVisible()
})
