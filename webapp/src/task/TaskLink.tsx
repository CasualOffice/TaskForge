/**
 * The one way a task is opened from a list, a row or a card.
 *
 * # The failure this module prevents
 *
 * A tracker whose tasks cannot be opened in a tab. Every surface opened the peek
 * from a `<button>`, so ⌘-click, middle-click, "Open in new tab" and dragging a
 * link into a chat window all did nothing — and those are the gestures people
 * use to work on three tasks at once. `design/LAYOUT-AND-INTERACTION-GUIDELINES.md`
 * §4 makes the full route "what direct links, new tabs and narrow screens get",
 * which is only true if there is a real `href` to reach it with.
 *
 * # Why one element serves both
 *
 * A plain click should preserve the reader's place — that is what the peek is
 * for, and losing a scrolled board to read one card is the reason people stop
 * opening tasks. A modified click should do what the browser has always done.
 * Rendering two controls to express that would put two tab stops on every row
 * for one destination.
 *
 * So this is an anchor with a real `href`, and the plain-click case is the only
 * one intercepted. Middle-click is not handled at all: it does not fire `click`,
 * so the browser's own behaviour survives by not being touched.
 */
import type { MouseEvent, ReactElement, ReactNode } from 'react'
import { Link } from '@tanstack/react-router'

export function TaskLink({
  taskId,
  className,
  title,
  onPeek,
  children,
}: {
  taskId: string
  className?: string
  title?: string
  /** Opens the peek. Called only for an unmodified primary click. */
  onPeek: (taskId: string) => void
  children: ReactNode
}): ReactElement {
  function onClick(event: MouseEvent<HTMLAnchorElement>): void {
    // Anything the user asked the browser for — a new tab, a new window, a
    // download, a selection — is theirs. Only the plain case is ours.
    if (event.metaKey || event.ctrlKey || event.shiftKey || event.altKey) return
    event.preventDefault()
    onPeek(taskId)
  }

  return (
    <Link
      to="/tasks/$taskId"
      params={{ taskId }}
      className={className}
      {...(title === undefined ? {} : { title })}
      onClick={onClick}
    >
      {children}
    </Link>
  )
}
