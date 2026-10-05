import { expect, test } from '@playwright/test'

// 跨端场景：在 Admin 改排序，到公开站验证。公开站地址与 admin project 的 baseURL 不同，
// 这里直接读环境变量（与 playwright.config.ts 中 web project 的默认值保持一致）。
const webBaseURL = process.env.E2E_WEB_BASE_URL ?? 'http://127.0.0.1:3000'

interface ListRow {
  title: string
  pinned: boolean
  published: boolean
}

// 核心回归场景：文章手动排序（reorder / move 接口）曾出现 Admin 调整顺序后公开站
// 不同步的问题。公开列表固定按 is_pinned DESC, sort_order ASC 排序
// （backend/infra/src/content.rs list_public_articles），因此交换两篇相邻、
// 未置顶且已发布文章的 sort_order，必然改变公开站首页的相对顺序。
test('排序模式下移动文章后，公开站首页顺序同步变化', async ({ page }) => {
  const username = process.env.E2E_ADMIN_USERNAME
  const password = process.env.E2E_ADMIN_PASSWORD
  test.skip(
    !username || !password,
    '未设置 E2E_ADMIN_USERNAME / E2E_ADMIN_PASSWORD（凭据只走环境变量，禁止硬编码）',
  )

  // 1. 登录 Admin（登录页无 data-testid，使用 label 关联的 id 与按钮 Role）。
  await page.goto('/auth/sign-in')
  await page.locator('#login').fill(username!)
  await page.locator('#password').fill(password!)
  await page.getByRole('button', { name: '登录', exact: true }).click()
  // 登录成功后跳离登录页（默认到 Dashboard）。
  await page.waitForURL(url => !url.pathname.startsWith('/auth/sign-in'), { timeout: 15_000 })

  // 2. 打开文章列表，切到「按排序（可拖拽）」（sort_order 升序）。
  await page.goto('/articles')
  await page.locator('select[aria-label="排序字段"]').selectOption('sort_order')

  // 排序模式下每行渲染「上移《标题》/ 下移《标题》」箭头按钮；以按钮为锚点定位行。
  const downButtons = page.getByRole('button', { name: /^下移《.+》$/ })

  async function waitForListReady() {
    // 加载完成 = 出现数据行箭头按钮，或出现空状态文案。
    await expect(
      downButtons
        .first()
        .or(page.getByText('还没有文章'))
        .or(page.getByText('没有匹配的文章')),
    ).toBeVisible()
  }

  async function listRows(): Promise<ListRow[]> {
    const count = await downButtons.count()
    const rows: ListRow[] = []
    for (let index = 0; index < count; index += 1) {
      const button = downButtons.nth(index)
      const label = (await button.getAttribute('aria-label')) ?? ''
      const title = label.replace(/^下移《|》$/g, '')
      const rowText = await page.locator('tr', { has: button }).innerText()
      rows.push({
        title,
        pinned: rowText.includes('置顶'),
        published: rowText.includes('已发布'),
      })
    }
    return rows
  }

  await waitForListReady()
  const rowsBefore = await listRows()

  // 断言宽松化：找一对相邻、均未置顶、均已发布的文章；没有数据（或没有可用对）则跳过。
  let pairIndex = -1
  for (let index = 0; index + 1 < rowsBefore.length; index += 1) {
    const upper = rowsBefore[index]
    const lower = rowsBefore[index + 1]
    if (!upper.pinned && !lower.pinned && upper.published && lower.published) {
      pairIndex = index
      break
    }
  }
  test.skip(pairIndex === -1, '文章列表中没有一对相邻、未置顶且已发布的文章，跳过重排场景')

  const upperTitle = rowsBefore[pairIndex].title
  const lowerTitle = rowsBefore[pairIndex + 1].title

  let swapped = false
  try {
    // 3. 用上方的「下移」箭头交换两篇（页内交换走 reorder 接口）。
    await page.getByRole('button', { name: `下移《${upperTitle}》` }).click()
    await expect
      .poll(async () => {
        const rows = await listRows()
        return [rows[pairIndex]?.title, rows[pairIndex + 1]?.title]
      })
      .toEqual([lowerTitle, upperTitle])
    swapped = true

    // 4. 打开公开站首页，验证两篇文章的相对顺序已经反转。
    await page.goto(webBaseURL)
    const publicTitles = (await page.locator('#article-list .post-title a').allInnerTexts()).map(
      title => title.trim(),
    )
    const upperPublicIndex = publicTitles.indexOf(upperTitle)
    const lowerPublicIndex = publicTitles.indexOf(lowerTitle)
    test.skip(
      upperPublicIndex === -1 || lowerPublicIndex === -1,
      '被交换的文章不在公开站首页第一页（可能被分页截断），无法验证顺序',
    )
    expect(
      lowerPublicIndex,
      `公开站首页《${lowerTitle}》应排在《${upperTitle}》之前`,
    ).toBeLessThan(upperPublicIndex)
  } finally {
    // 5. 恢复原顺序，保证用例幂等可重复执行。
    if (swapped) {
      await page.goto('/articles')
      await page.locator('select[aria-label="排序字段"]').selectOption('sort_order')
      await waitForListReady()
      await page.getByRole('button', { name: `上移《${upperTitle}》` }).click()
      await expect
        .poll(async () => {
          const rows = await listRows()
          return [rows[pairIndex]?.title, rows[pairIndex + 1]?.title]
        })
        .toEqual([upperTitle, lowerTitle])
    }
  }
})
