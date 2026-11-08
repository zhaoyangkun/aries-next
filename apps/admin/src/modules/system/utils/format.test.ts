import { describe, expect, it } from 'vitest'

import { formatBitrate, formatBytes, formatDuration, formatPercent, formatRate, usageTone } from './format'

describe('formatBytes', () => {
  it('formats bytes below one KiB as B', () => {
    expect(formatBytes(0)).toBe('0 B')
    expect(formatBytes(512)).toBe('512 B')
    expect(formatBytes(1023)).toBe('1023 B')
  })

  it('formats KiB and above with binary units', () => {
    expect(formatBytes(1024)).toBe('1.0 KB')
    expect(formatBytes(2048)).toBe('2.0 KB')
    expect(formatBytes(5 * 1024 * 1024)).toBe('5.0 MB')
    expect(formatBytes(3 * 1024 ** 3)).toBe('3.0 GB')
    expect(formatBytes(1024 ** 4)).toBe('1.0 TB')
  })

  it('rounds values at or above 100 to integers', () => {
    expect(formatBytes(123456 * 1024)).toBe('121 MB')
    expect(formatBytes(1024 * 1206)).toBe('1.2 MB')
  })

  it('guards invalid input', () => {
    expect(formatBytes(Number.NaN)).toBe('-')
    expect(formatBytes(-1)).toBe('-')
  })
})

describe('formatRate', () => {
  it('appends per-second suffix', () => {
    expect(formatRate(0)).toBe('0 B/s')
    expect(formatRate(2048)).toBe('2.0 KB/s')
  })
})

describe('formatDuration', () => {
  it('formats seconds and minutes', () => {
    expect(formatDuration(0)).toBe('0 秒')
    expect(formatDuration(45)).toBe('45 秒')
    expect(formatDuration(60)).toBe('1 分')
    expect(formatDuration(65)).toBe('1 分 5 秒')
  })

  it('formats hours and days', () => {
    expect(formatDuration(3600)).toBe('1 小时')
    expect(formatDuration(3660)).toBe('1 小时 1 分')
    expect(formatDuration(86400)).toBe('1 天')
    expect(formatDuration(90000)).toBe('1 天 1 小时')
  })

  it('guards invalid input', () => {
    expect(formatDuration(Number.NaN)).toBe('-')
    expect(formatDuration(-5)).toBe('-')
  })
})

describe('formatPercent', () => {
  it('formats with one decimal', () => {
    expect(formatPercent(0)).toBe('0.0%')
    expect(formatPercent(42.56)).toBe('42.6%')
  })

  it('clamps out-of-range values', () => {
    expect(formatPercent(120)).toBe('100.0%')
    expect(formatPercent(-3)).toBe('0.0%')
  })
})

describe('formatBitrate', () => {
  it('converts bytes per second to bit-rate units', () => {
    expect(formatBitrate(0)).toBe('0 Kbps')
    expect(formatBitrate(125_000)).toBe('1.0 Mbps')
    expect(formatBitrate(125_000 * 100)).toBe('100.0 Mbps')
    expect(formatBitrate(125_000_000)).toBe('1.00 Gbps')
  })

  it('guards invalid input', () => {
    expect(formatBitrate(Number.NaN)).toBe('-')
    expect(formatBitrate(-1)).toBe('-')
  })
})

describe('usageTone', () => {
  it('maps percent to threshold tiers', () => {
    expect(usageTone(0)).toBe('ok')
    expect(usageTone(69.9)).toBe('ok')
    expect(usageTone(70)).toBe('warning')
    expect(usageTone(89.9)).toBe('warning')
    expect(usageTone(90)).toBe('critical')
    expect(usageTone(100)).toBe('critical')
  })

  it('treats invalid input as ok', () => {
    expect(usageTone(Number.NaN)).toBe('ok')
  })
})
