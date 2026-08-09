/**
 * A button that opens a small surface next to itself.
 *
 * # The failure this module prevents
 *
 * A toolbar, a drawer and a row each growing their own dropdown. Four
 * hand-rolled popovers means four chances to forget `Escape`, four
 * outside-click listeners bound on `click` instead of `mousedown` (which closes
 * the surface under the cursor on the first selection), and four different
 * answers to where focus goes on close. `design/LAYOUT-AND-INTERACTION-GUIDELINES.md`
 * §9 names popover and menu as distinct overlays with one job each; this is the
 * mechanism both of them use.
 *
 * # Why the trigger is a plain button and the content is a render prop
 *
 * The content needs to close the surface — picking a status, saving a form —
 * and the only way to hand it that without a context is to pass it. Everything
 * else about the content is the caller's: this file owns *when* it is on screen
 * and nothing about what it says.
 *
 * # This is deliberately shaped like the design system's `Popover`
 *
 * AGENTS.md makes `@schnsrw/design-system` a consumed dependency and its
 * primitives include `Popover` and `Menu`. The props below — a trigger, an
 * alignment, content, an open state — are that shape, so the swap replaces the
 * body of this file rather than every call site.
 */
import {
  useCallback,
  useEffect,
  useId,
  useRef,
  useState,
  type ReactElement,
  type ReactNode,
} from 'react'

export function Popover({
  label,
  ariaLabel,
  align = 'start',
  triggerClass = 'button',
  disabled = false,
  title,
  children,
}: {
  /** What the trigger shows. A node, so a status chip can be the trigger. */
  label: ReactNode
  /** Required when `label` is not readable text on its own. */
  ariaLabel?: string
  /** Which edge the surface lines up with. `end` keeps a right-hand control on screen. */
  align?: 'start' | 'end'
  triggerClass?: string
  disabled?: boolean
  /** Hover/focus explanation — §9: a tooltip explains, it does not act. */
  title?: string
  children: (close: () => void) => ReactNode
}): ReactElement {
  const [open, setOpen] = useState(false)
  const host = useRef<HTMLDivElement>(null)
  const trigger = useRef<HTMLButtonElement>(null)
  const id = useId()

  const close = useCallback(() => {
    setOpen(false)
    // Focus returns to the control that opened the surface. Without this, a
    // keyboard user who picks an option is left with focus on `<body>` and has
    // to tab from the top of the page to get back to where they were.
    trigger.current?.focus()
  }, [])

  useEffect(() => {
    if (!open) return
    function onDocument(event: MouseEvent): void {
      if (!host.current?.contains(event.target as Node)) setOpen(false)
    }
    function onKey(event: KeyboardEvent): void {
      if (event.key !== 'Escape') return
      // Stopped, so a popover inside the drawer does not also close the drawer.
      // One Escape, one dismissal — the innermost.
      event.stopPropagation()
      setOpen(false)
      trigger.current?.focus()
    }
    // `mousedown`, not `click`: a `click` listener fires after the content has
    // already re-rendered, and the surface closes under the cursor.
    document.addEventListener('mousedown', onDocument)
    document.addEventListener('keydown', onKey, true)
    return () => {
      document.removeEventListener('mousedown', onDocument)
      document.removeEventListener('keydown', onKey, true)
    }
  }, [open])

  return (
    <div className="pop" ref={host}>
      <button
        type="button"
        ref={trigger}
        className={triggerClass}
        aria-expanded={open}
        aria-haspopup="true"
        aria-controls={open ? id : undefined}
        {...(ariaLabel === undefined ? {} : { 'aria-label': ariaLabel })}
        {...(title === undefined ? {} : { title })}
        disabled={disabled}
        onClick={() => setOpen(!open)}
      >
        {label}
      </button>

      {open ? (
        <div className={`pop__surface pop__surface--${align}`} id={id}>
          {children(close)}
        </div>
      ) : null}
    </div>
  )
}

/**
 * One-of-N inside a popover.
 *
 * `role="menu"` is deliberately absent: a menu role imposes roving focus, and a
 * list of ordinary buttons already tabs, activates on Enter and Space, and
 * announces its own disabled state. The one thing it does not give for free is
 * telling the reader which entry is the current one, which is `aria-current`.
 */
export function ChoiceList({
  options,
  current,
  onChoose,
  close,
}: {
  options: readonly { value: string; label: ReactNode; disabled?: string }[]
  current?: string
  onChoose: (value: string) => void
  close: () => void
}): ReactElement {
  return (
    <ul className="pop__list">
      {options.map((option) => (
        <li key={option.value}>
          <button
            type="button"
            className="pop__item"
            aria-current={option.value === current ? 'true' : undefined}
            disabled={option.disabled !== undefined}
            {...(option.disabled === undefined ? {} : { title: option.disabled })}
            onClick={() => {
              onChoose(option.value)
              close()
            }}
          >
            {option.label}
            {option.disabled === undefined ? null : (
              <span className="pop__why">{option.disabled}</span>
            )}
          </button>
        </li>
      ))}
    </ul>
  )
}
