// 日志详情页的轻量语法高亮：自写 tokenizer，输出转义后的安全 HTML，
// 供 v-html 渲染。日志内容来自用户输入，所有 token 必须经过 escapeHtml。

function escapeHtml(text: string): string {
  return text
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;')
}

function span(className: string, text: string): string {
  return `<span class="${className}">${escapeHtml(text)}</span>`
}

const JSON_KEY_CLASS = 'text-blue-700 dark:text-blue-400'
const JSON_STRING_CLASS = 'text-emerald-700 dark:text-emerald-400'
const JSON_NUMBER_CLASS = 'text-orange-600 dark:text-orange-400'
const JSON_LITERAL_CLASS = 'text-fuchsia-600 dark:text-fuchsia-400'

// JSON 语法（stringify 产物）只有四类 token：字符串（含 key）、数字、true/false/null、标点。
const JSON_TOKEN_RE =
  /("(?:\\u[a-fA-F0-9]{4}|\\[^u]|[^\\"])*")(\s*:)?|\b(?:true|false|null)\b|-?\d+(?:\.\d+)?(?:[eE][+-]?\d+)?/g

/** fields 对象的 JSON 高亮：key 蓝 / 字符串绿 / 数字橙 / 布尔与 null 紫。 */
export function highlightJson(value: unknown): string {
  const source = JSON.stringify(value, null, 2) ?? String(value)
  let result = ''
  let lastIndex = 0
  for (const match of source.matchAll(JSON_TOKEN_RE)) {
    result += escapeHtml(source.slice(lastIndex, match.index))
    const [token, quoted, colon] = match
    if (quoted !== undefined) {
      // 带冒号的是 key，否则是字符串值。
      if (colon !== undefined) {
        result += span(JSON_KEY_CLASS, quoted) + escapeHtml(colon)
      } else {
        result += span(JSON_STRING_CLASS, quoted)
      }
    } else if (token === 'true' || token === 'false' || token === 'null') {
      result += span(JSON_LITERAL_CLASS, token)
    } else {
      result += span(JSON_NUMBER_CLASS, token)
    }
    lastIndex = match.index + token.length
  }
  result += escapeHtml(source.slice(lastIndex))
  return result
}

const SQL_KEYWORDS = new Set([
  'SELECT', 'INSERT', 'UPDATE', 'DELETE', 'FROM', 'WHERE', 'INTO', 'VALUES', 'SET',
  'JOIN', 'LEFT', 'RIGHT', 'INNER', 'OUTER', 'FULL', 'CROSS', 'ON', 'AND', 'OR', 'NOT',
  'NULL', 'AS', 'ORDER', 'BY', 'GROUP', 'HAVING', 'LIMIT', 'OFFSET', 'RETURNING',
  'DISTINCT', 'UNION', 'ALL', 'EXISTS', 'IN', 'LIKE', 'ILIKE', 'BETWEEN', 'IS',
  'CASE', 'WHEN', 'THEN', 'ELSE', 'END', 'ASC', 'DESC', 'TRUE', 'FALSE', 'DEFAULT',
  'WITH', 'RECURSIVE', 'CAST', 'CONFLICT', 'DO', 'NOTHING', 'CREATE', 'ALTER', 'DROP',
  'TABLE', 'INDEX', 'PRIMARY', 'KEY', 'REFERENCES', 'UNIQUE', 'CHECK',
])

const SQL_KEYWORD_CLASS = 'font-medium text-blue-700 dark:text-blue-400'
const SQL_STRING_CLASS = 'text-emerald-700 dark:text-emerald-400'
const SQL_NUMBER_CLASS = 'text-orange-600 dark:text-orange-400'
const SQL_COMMENT_CLASS = 'italic text-muted-foreground'
const SQL_PLACEHOLDER_CLASS = 'text-fuchsia-600 dark:text-fuchsia-400'
const SQL_TYPE_CLASS = 'text-cyan-700 dark:text-cyan-400'
const SQL_FUNCTION_CLASS = 'text-violet-700 dark:text-violet-400'

// 顺序即优先级：注释 > 字符串 > 类型转换 ::type > 占位符 $1 > 数字 > 单词（关键字/函数/标识符）。
const SQL_TOKEN_RE =
  /(--[^\n]*|\/\*[\s\S]*?\*\/)|('(?:[^']|'')*')|(::\s*[a-zA-Z_][\w]*(?:\[\])*)|(\$\d+)|\b(\d+(?:\.\d+)?)\b|([a-zA-Z_][\w]*)/g

/** SQL 语句高亮：关键字蓝 / 字符串绿 / 数字橙 / 注释灰 / $n 占位符紫 / ::类型青 / 函数堇。 */
export function highlightSql(sql: string): string {
  let result = ''
  let lastIndex = 0
  for (const match of sql.matchAll(SQL_TOKEN_RE)) {
    result += escapeHtml(sql.slice(lastIndex, match.index))
    const [token, comment, quoted, cast, placeholder, number, word] = match
    if (comment !== undefined) {
      result += span(SQL_COMMENT_CLASS, comment)
    } else if (quoted !== undefined) {
      result += span(SQL_STRING_CLASS, quoted)
    } else if (cast !== undefined) {
      result += span(SQL_TYPE_CLASS, cast)
    } else if (placeholder !== undefined) {
      result += span(SQL_PLACEHOLDER_CLASS, placeholder)
    } else if (number !== undefined) {
      result += span(SQL_NUMBER_CLASS, number)
    } else if (word !== undefined) {
      if (SQL_KEYWORDS.has(word.toUpperCase())) {
        result += span(SQL_KEYWORD_CLASS, word)
      } else if (sql[match.index + token.length] === '(') {
        result += span(SQL_FUNCTION_CLASS, word)
      } else {
        result += escapeHtml(word)
      }
    }
    lastIndex = match.index + token.length
  }
  result += escapeHtml(sql.slice(lastIndex))
  return result
}
