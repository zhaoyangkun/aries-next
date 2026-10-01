import { describe, expect, it } from 'vitest'
import { highlightJson, highlightSql } from './highlight'

describe('highlightJson', () => {
  it('colors keys, strings, numbers and literals', () => {
    const html = highlightJson({ status: 500, ok: false, note: 'failed', extra: null })

    expect(html).toContain('text-blue-700')
    expect(html).toContain('&quot;status&quot;</span>:')
    expect(html).toContain('text-emerald-700')
    expect(html).toContain('&quot;failed&quot;')
    expect(html).toContain('text-orange-600')
    expect(html).toContain('>500<')
    expect(html).toContain('text-fuchsia-600')
    expect(html).toContain('>false</span>')
    expect(html).toContain('>null</span>')
  })

  it('escapes HTML inside values to keep v-html safe', () => {
    const html = highlightJson({ msg: '<script>alert("x")</script>' })

    expect(html).not.toContain('<script>')
    expect(html).toContain('&lt;script&gt;')
  })

  it('handles primitives at the top level', () => {
    expect(highlightJson('plain')).toContain('&quot;plain&quot;')
    expect(highlightJson(42)).toContain('>42</span>')
  })
})

describe('highlightSql', () => {
  it('colors keywords, placeholders and casts', () => {
    const html = highlightSql('SELECT * FROM articles WHERE id = $1::bigint LIMIT 10')

    expect(html).toContain('>SELECT</span>')
    expect(html).toContain('>FROM</span>')
    expect(html).toContain('>$1</span>')
    expect(html).toContain('::bigint</span>')
    expect(html).toContain('>10</span>')
    // 小写关键字同样命中，保留原始大小写输出。
    expect(highlightSql('select id from t')).toContain('>select</span>')
  })

  it('colors strings, comments and function calls', () => {
    const html = highlightSql("SELECT now() -- 当前时间\nWHERE name = 'it''s'")

    expect(html).toContain('>now</span>(')
    expect(html).toContain('italic')
    expect(html).toContain('&#39;it&#39;&#39;s&#39;')
  })

  it('escapes HTML metacharacters in statements', () => {
    const html = highlightSql("SELECT '<img onerror=alert(1)>'")

    expect(html).not.toContain('<img')
    expect(html).toContain('&lt;img')
  })
})
