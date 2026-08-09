/**
 * The one control height.
 *
 * The design system draws a button and a text input 34 px tall at `md` and a
 * select 32, so any row that mixes them is 2 px ragged — the kind of thing
 * nobody names but everybody sees. Every select the product places beside a
 * button carries this, and when design-book evens the two up there is a single
 * constant to delete rather than a hunt through call sites.
 */
export const CONTROL = 34

/**
 * The tint that says "this control is narrowing the list".
 *
 * It used to be a class (`.filter__select--on`). The design system's select
 * sets its border and background inline, and an inline style beats a class, so
 * expressing it as CSS now would mean a filter that is on and looks off.
 */
export function narrowing(on: boolean): import('react').CSSProperties | undefined {
  return on
    ? {
        borderColor: 'var(--color-accent)',
        background: 'var(--color-selected)',
        color: 'var(--color-accent)',
      }
    : undefined
}

/**
 * A subtle button used as a section's own control.
 *
 * The design system pads a button 15 px so its label never touches its edge,
 * which is right for a button and wrong for one whose whole surface is
 * transparent: sitting in a column of flush-left sections, its label alone
 * starts 15 px in and the column's edge visibly breaks. The pull-back is on
 * the box, so the padding still catches the pointer.
 */
export const FLUSH = { marginLeft: -15 } as const
