import { describe, expect, it } from 'vitest'
import {
  currentPage,
  cursorForPage,
  goToNextPage,
  goToPreviousPage,
} from './cursor-pagination'

describe('cursor-pagination', () => {
  it('starts at page 1 without a cursor', () => {
    const stack: string[] = []
    expect(currentPage(stack)).toBe(1)
    expect(cursorForPage(stack)).toBeUndefined()
  })

  it('pushes next_cursor when going forward and pops when going back', () => {
    const stack: string[] = []

    expect(goToNextPage(stack, 'cursor-page-2')).toBe(true)
    expect(currentPage(stack)).toBe(2)
    expect(cursorForPage(stack)).toBe('cursor-page-2')

    expect(goToNextPage(stack, 'cursor-page-3')).toBe(true)
    expect(currentPage(stack)).toBe(3)
    expect(cursorForPage(stack)).toBe('cursor-page-3')

    expect(goToPreviousPage(stack)).toBe(true)
    expect(currentPage(stack)).toBe(2)
    expect(cursorForPage(stack)).toBe('cursor-page-2')

    expect(goToPreviousPage(stack)).toBe(true)
    expect(currentPage(stack)).toBe(1)
    expect(cursorForPage(stack)).toBeUndefined()
  })

  it('does not go forward without a next_cursor (last page)', () => {
    const stack = ['cursor-page-2']
    expect(goToNextPage(stack, null)).toBe(false)
    expect(stack).toEqual(['cursor-page-2'])
    expect(currentPage(stack)).toBe(2)
  })

  it('does not go back from the first page (empty stack)', () => {
    const stack: string[] = []
    expect(goToPreviousPage(stack)).toBe(false)
    expect(stack).toEqual([])
  })
})
