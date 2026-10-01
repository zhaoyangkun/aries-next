// datetime-local 值（YYYY-MM-DDTHH:mm）→ RFC 3339（按 UTC 解析，秒补 00）。
export function toRfc3339(value: string) {
  if (!value)
    return undefined
  const seconds = value.length === 16 ? ':00' : ''
  return `${value}${seconds}Z`
}
