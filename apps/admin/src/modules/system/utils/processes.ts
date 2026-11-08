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
  /** RSS 合计（注意：fork 共享页会在多个进程间重复计数，合计值会夸大）。 */
  memory_bytes: number
}

/** 进程列表 → 按名称聚合的进程组（求和 count/cpu/内存，保留全部 PID）。 */
export function groupProcesses(processes: ProcessMetrics[]): ProcessGroup[] {
  const groups = new Map<string, ProcessGroup>()
  for (const process of processes) {
    const existing = groups.get(process.name)
    if (existing) {
      existing.count += 1
      existing.pids.push(process.pid)
      existing.cpu_percent += process.cpu_percent
      existing.memory_bytes += process.memory_bytes
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
