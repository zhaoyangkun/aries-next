import type { ClassValue } from 'clsx'

import { clsx } from 'clsx'
import { twMerge } from 'tailwind-merge'

/** 合并 Tailwind 类名：clsx 拼接后经 tailwind-merge 消解冲突类（shadcn 约定）。 */
export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs))
}
