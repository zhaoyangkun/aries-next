import { describe, expect, it } from 'vitest'

import { groupProcesses, sortProcessGroups } from './processes'

function process(pid: number, name: string, cpu: number, memory: number) {
  return { pid, name, cpu_percent: cpu, memory_bytes: memory }
}

describe('groupProcesses', () => {
  it('sums same-name processes into one group', () => {
    const groups = groupProcesses([
      process(1, 'worker', 10, 100),
      process(2, 'worker', 20, 200),
      process(3, 'db', 5, 500),
    ])
    expect(groups).toHaveLength(2)
    const worker = groups.find(group => group.name === 'worker')!
    expect(worker.count).toBe(2)
    expect(worker.pids).toEqual([1, 2])
    expect(worker.cpu_percent).toBeCloseTo(30)
    expect(worker.memory_bytes).toBe(300)
    const db = groups.find(group => group.name === 'db')!
    expect(db.count).toBe(1)
    expect(db.pids).toEqual([3])
  })

  it('treats empty names as a single unnamed group', () => {
    const groups = groupProcesses([
      process(1, '', 1, 10),
      process(2, '', 2, 20),
    ])
    expect(groups).toHaveLength(1)
    expect(groups[0].count).toBe(2)
  })

  it('returns empty array for empty input', () => {
    expect(groupProcesses([])).toEqual([])
  })
})

describe('sortProcessGroups', () => {
  const groups = groupProcesses([
    process(1, 'a', 50, 100),
    process(2, 'b', 10, 900),
    process(3, 'c', 30, 500),
  ])

  it('sorts by cpu descending by default', () => {
    const sorted = sortProcessGroups(groups, 'cpu', true)
    expect(sorted.map(group => group.name)).toEqual(['a', 'c', 'b'])
  })

  it('sorts ascending when asked', () => {
    const sorted = sortProcessGroups(groups, 'cpu', false)
    expect(sorted.map(group => group.name)).toEqual(['b', 'c', 'a'])
  })

  it('sorts by memory total', () => {
    const sorted = sortProcessGroups(groups, 'memory', true)
    expect(sorted.map(group => group.name)).toEqual(['b', 'c', 'a'])
  })

  it('does not mutate the input array', () => {
    const before = [...groups]
    sortProcessGroups(groups, 'memory', true)
    expect(groups).toEqual(before)
  })
})
