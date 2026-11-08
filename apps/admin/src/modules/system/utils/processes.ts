import type { ProcessMetrics } from '@/modules/system/api/system'

/** 按名称聚合的进程组（任务管理器风格，避免多进程程序刷屏）。 */
export interface ProcessGroup {
  name: string
  /** 同名进程实例数。 */
  count: number
  /** 实例 PID 列表（保留原始顺序）。 */
  pids: number[]
  /** CPU 合计（单核口径：100 = 打满一个核）。 */
  cpu_percent: number
  /**
   * 组内最大 RSS（近似物理占用）：同名进程多为 fork 共享基线内存，
   * 取最大避免共享页被重复计数（sum 会把 19 个 469 MB 算成 8.9 GB）。
   * 注意：对 postgres 这类共享缓冲区大的进程组会偏保守（低估私有部分之和）。
   */
  memory_bytes: number
}

/** 进程列表 → 按名称聚合的进程组（count/cpu 求和，内存取组内最大 RSS）。 */
export function groupProcesses(processes: ProcessMetrics[]): ProcessGroup[] {
  const groups = new Map<string, ProcessGroup>()
  for (const process of processes) {
    const existing = groups.get(process.name)
    if (existing) {
      existing.count += 1
      existing.pids.push(process.pid)
      existing.cpu_percent += process.cpu_percent
      // 内存取最大而非求和：fork 子进程共享基线页面，求和会重复计数共享内存。
      existing.memory_bytes = Math.max(existing.memory_bytes, process.memory_bytes)
    }
    else {
      groups.set(process.name, {
        name: process.name,
        count: 1,
        pids: [process.pid],
        cpu_percent: process.cpu_percent,
        memory_bytes: process.memory_bytes,
      })
    }
  }
  return [...groups.values()]
}

/** 进程组排序：按维度（合计值）与方向排序。 */
export function sortProcessGroups(
  groups: ProcessGroup[],
  key: 'cpu' | 'memory',
  descending: boolean,
): ProcessGroup[] {
  const direction = descending ? -1 : 1
  return [...groups].sort((a, b) => {
    const left = key === 'cpu' ? a.cpu_percent : a.memory_bytes
    const right = key === 'cpu' ? b.cpu_percent : b.memory_bytes
    return (left - right) * direction
  })
}
