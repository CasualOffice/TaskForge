/**
 * Where a popover surface lands.
 *
 * # The two failures this pins
 *
 * Both were arithmetic, and neither was visible until someone opened the
 * control on a particular screen — which is why they were reported as "New task
 * is unusable" rather than as a layout bug.
 *
 * A 360 px panel right-aligned to a trigger near the *left* edge extends
 * leftwards, off the screen. The toolbar wraps "New task" to the left at
 * desktop widths, so that is where it was.
 *
 * And a panel opening downwards with no room below opens into nothing.
 */
import { describe, expect, it } from 'vitest'

import { place } from './Popover'

const box = (left: number, top: number, width = 90, height = 34) => ({
  left,
  top,
  right: left + width,
  bottom: top + height,
  width,
  height,
})

const screen = { width: 1440, height: 900 }
const panel = { width: 360, height: 300 }

describe('placing a popover surface', () => {
  it('keeps a right-aligned panel on screen when its trigger is near the left edge', () => {
    // The reported failure: right-aligning to a trigger at x=40 wants
    // 40 + 90 - 360 = -230, which is off the screen by 230 px.
    const { left } = place(box(40, 100), panel, 'end', screen)
    expect(left).toBeGreaterThanOrEqual(8)
    expect(left + panel.width).toBeLessThanOrEqual(screen.width - 8)
  })

  it('keeps a left-aligned panel on screen when its trigger is near the right edge', () => {
    const { left } = place(box(1380, 100), panel, 'start', screen)
    expect(left + panel.width).toBeLessThanOrEqual(screen.width - 8)
  })

  it('right-aligns to the trigger when there is room', () => {
    const anchor = box(900, 100)
    const { left } = place(anchor, panel, 'end', screen)
    expect(left).toBe(anchor.right - panel.width)
  })

  it('opens above when there is no room below', () => {
    const anchor = box(400, 800)
    const { top } = place(anchor, panel, 'start', screen)
    expect(top).toBeLessThan(anchor.top)
    expect(top).toBeGreaterThanOrEqual(8)
  })

  it('opens below when there is room', () => {
    const anchor = box(400, 100)
    const { top } = place(anchor, panel, 'start', screen)
    expect(top).toBe(anchor.bottom + 4)
  })

  it('starts at the margin when the panel is wider than the screen', () => {
    // A phone narrower than the panel. Off the left edge is the one answer that
    // is never right, so the clamp's outermost bound is the left margin.
    const { left } = place(box(20, 100), { width: 500, height: 200 }, 'end', {
      width: 390,
      height: 844,
    })
    expect(left).toBe(8)
  })
})
