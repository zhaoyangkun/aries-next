import { afterEach, describe, expect, it, vi } from 'vitest'
import { api } from '@/shared/api/client'
import { articlesApi, type AdminArticle, type ArticleRevision } from './articles'

const article: AdminArticle = {
  id: 1,
  author_id: 1,
  category_id: null,
  status: 'draft',
  slug: 'first-post',
  title: 'First post',
  summary: '',
  cover_url: null,
  markdown_source: '# First post',
  rendered_html: '<h1>First post</h1>',
  seo_keywords: [],
  tag_ids: [],
  password_protected: false,
  allow_comments: true,
  is_pinned: false,
  version: 1,
  published_at: null,
  created_at: '2026-08-05T12:00:00Z',
  updated_at: '2026-08-05T12:00:00Z',
}

const revision: ArticleRevision = {
  revision_no: 2,
  title: 'First post',
  slug: 'first-post',
  summary: '',
  category_id: null,
  cover_url: null,
  seo_keywords: [],
  tag_ids: [],
  password_protected: false,
  allow_comments: true,
  is_pinned: false,
  markdown_source: '# First post',
  operator_id: 1,
  created_at: '2026-08-05T12:00:00Z',
}

afterEach(() => {
  vi.restoreAllMocks()
})

describe('articlesApi', () => {
  it('loads the server article list with filters', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({
      data: { items: [article], total: 1, page: 1, page_size: 20 },
    })

    const result = await articlesApi.list({ page: 1, page_size: 20, status: 'draft' })

    expect(result.items).toEqual([article])
    expect(get).toHaveBeenCalledWith('/api/admin/articles', {
      params: { page: 1, page_size: 20, status: 'draft' },
    })
  })

  it('creates a server draft instead of writing browser storage', async () => {
    const post = vi.spyOn(api, 'post').mockResolvedValue({ data: article })
    const payload = {
      title: 'First post',
      summary: '',
      markdown_source: '# First post',
    }

    await expect(articlesApi.create(payload)).resolves.toEqual(article)
    expect(post).toHaveBeenCalledWith('/api/admin/articles', payload)
  })

  it('requests sanitized preview html from the backend', async () => {
    const post = vi.spyOn(api, 'post').mockResolvedValue({
      data: { rendered_html: '<h1>Preview</h1>' },
    })

    await expect(articlesApi.preview('# Preview')).resolves.toEqual({
      rendered_html: '<h1>Preview</h1>',
    })
    expect(post).toHaveBeenCalledWith('/api/admin/articles/preview', {
      markdown_source: '# Preview',
    })
  })

  it('loads and updates an existing draft by id and version', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({ data: article })
    const put = vi.spyOn(api, 'put').mockResolvedValue({ data: { ...article, version: 2 } })

    await expect(articlesApi.get(1)).resolves.toEqual(article)
    await expect(
      articlesApi.update(1, {
        title: 'First post revised',
        slug: 'first-post',
        summary: '',
        markdown_source: '# First post revised',
        expected_version: 1,
      }),
    ).resolves.toMatchObject({ version: 2 })
    expect(get).toHaveBeenCalledWith('/api/admin/articles/1')
    expect(put).toHaveBeenCalledWith('/api/admin/articles/1', {
      title: 'First post revised',
      slug: 'first-post',
      summary: '',
      markdown_source: '# First post revised',
      expected_version: 1,
    })
  })

  it('sends explicit status commands with the expected version', async () => {
    const patch = vi.spyOn(api, 'patch').mockResolvedValue({
      data: { ...article, status: 'published', version: 2 },
    })

    await expect(articlesApi.changeStatus(1, 'publish', 1)).resolves.toMatchObject({
      status: 'published',
      version: 2,
    })
    expect(patch).toHaveBeenCalledWith('/api/admin/articles/1/status', {
      command: 'publish',
      expected_version: 1,
    })
  })

  it('loads and creates article taxonomy through the server API', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValueOnce({
      data: [{ id: 1, parent_id: null, name: 'Rust', slug: 'rust', description: '' }],
    }).mockResolvedValueOnce({
      data: [{ id: 2, name: 'Vue 3', slug: 'vue-3' }],
    })
    const post = vi.spyOn(api, 'post').mockResolvedValueOnce({
      data: { id: 3, parent_id: null, name: 'AI', slug: 'ai', description: '' },
    }).mockResolvedValueOnce({
      data: { id: 4, name: 'LLM', slug: 'llm' },
    })

    await expect(articlesApi.listCategories()).resolves.toHaveLength(1)
    await expect(articlesApi.listTags()).resolves.toHaveLength(1)
    await expect(articlesApi.createCategory('AI')).resolves.toMatchObject({ name: 'AI' })
    await expect(articlesApi.createTag('LLM')).resolves.toMatchObject({ name: 'LLM' })
    expect(get).toHaveBeenNthCalledWith(1, '/api/admin/categories')
    expect(get).toHaveBeenNthCalledWith(2, '/api/admin/tags')
    expect(post).toHaveBeenNthCalledWith(1, '/api/admin/categories', { name: 'AI' })
    expect(post).toHaveBeenNthCalledWith(2, '/api/admin/tags', { name: 'LLM' })
  })

  it('serializes sorting and taxonomy filters for the article list', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({
      data: { items: [], total: 0, page: 2, page_size: 10 },
    })

    await articlesApi.list({
      page: 2,
      page_size: 10,
      keyword: 'rust',
      status: 'published',
      category_id: 3,
      tag_id: 7,
      sort: 'title',
      order: 'asc',
    })

    expect(get).toHaveBeenCalledWith('/api/admin/articles', {
      params: {
        page: 2,
        page_size: 10,
        keyword: 'rust',
        status: 'published',
        category_id: 3,
        tag_id: 7,
        sort: 'title',
        order: 'asc',
      },
    })
  })

  it('omits access_password from the payload when it is not provided', async () => {
    const post = vi.spyOn(api, 'post').mockResolvedValue({ data: article })
    const payload = {
      title: 'First post',
      summary: '',
      markdown_source: '# First post',
      cover_url: null,
      seo_keywords: ['rust'],
      allow_comments: false,
      is_pinned: true,
    }

    await articlesApi.create(payload)
    const sentBody = post.mock.calls[0]?.[1] as Record<string, unknown>
    expect(sentBody).not.toHaveProperty('access_password')
    expect(post).toHaveBeenCalledWith('/api/admin/articles', payload)
  })

  it('serializes access_password set and clear semantics on update', async () => {
    const put = vi.spyOn(api, 'put').mockResolvedValue({ data: article })
    const base = {
      title: 'First post',
      summary: '',
      markdown_source: '# First post',
      expected_version: 1,
    }

    await articlesApi.update(1, { ...base, access_password: 'secret123' })
    expect(put).toHaveBeenLastCalledWith('/api/admin/articles/1', {
      ...base,
      access_password: 'secret123',
    })

    await articlesApi.update(1, { ...base, access_password: null })
    expect(put).toHaveBeenLastCalledWith('/api/admin/articles/1', {
      ...base,
      access_password: null,
    })
  })

  it('loads article revisions sorted by the backend', async () => {
    const get = vi.spyOn(api, 'get').mockResolvedValue({ data: [revision] })

    await expect(articlesApi.listRevisions(1)).resolves.toEqual([revision])
    expect(get).toHaveBeenCalledWith('/api/admin/articles/1/revisions')
  })

  it('restores a revision with the expected version body', async () => {
    const post = vi.spyOn(api, 'post').mockResolvedValue({
      data: { ...article, version: 3 },
    })

    await expect(articlesApi.restoreRevision(1, 2, 2)).resolves.toMatchObject({ version: 3 })
    expect(post).toHaveBeenCalledWith('/api/admin/articles/1/revisions/2/restore', {
      expected_version: 2,
    })
  })

  it('deletes a recycled article through the server API', async () => {
    const del = vi.spyOn(api, 'delete').mockResolvedValue({ data: undefined })

    await expect(articlesApi.remove(1)).resolves.toBeUndefined()
    expect(del).toHaveBeenCalledWith('/api/admin/articles/1')
  })

  it('updates and deletes taxonomy entries through the server API', async () => {
    const put = vi.spyOn(api, 'put')
      .mockResolvedValueOnce({ data: { id: 1, parent_id: null, name: 'Rust 2', slug: 'rust', description: '' } })
      .mockResolvedValueOnce({ data: { id: 2, name: 'Vue 4', slug: 'vue' } })
    const del = vi.spyOn(api, 'delete').mockResolvedValue({ data: undefined })

    await expect(articlesApi.updateCategory(1, { name: 'Rust 2' })).resolves.toMatchObject({ name: 'Rust 2' })
    await expect(articlesApi.updateTag(2, { name: 'Vue 4', slug: 'vue' })).resolves.toMatchObject({ name: 'Vue 4' })
    await expect(articlesApi.deleteCategory(1)).resolves.toBeUndefined()
    await expect(articlesApi.deleteTag(2)).resolves.toBeUndefined()
    expect(put).toHaveBeenNthCalledWith(1, '/api/admin/categories/1', { name: 'Rust 2' })
    expect(put).toHaveBeenNthCalledWith(2, '/api/admin/tags/2', { name: 'Vue 4', slug: 'vue' })
    expect(del).toHaveBeenNthCalledWith(1, '/api/admin/categories/1')
    expect(del).toHaveBeenNthCalledWith(2, '/api/admin/tags/2')
  })
})
