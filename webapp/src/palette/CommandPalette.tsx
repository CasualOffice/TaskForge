/**
 * ⌘K — the keyboard-first way to reach anything.
 *
 * # The failure this module prevents
 *
 * A palette that is a search box. docs/42 lists what it has to carry: "create,
 * navigate, assign, transition, search, and plugin-contributed commands". A box
 * that only navigates leaves every other capability needing a button, and the
 * seven-item navigation promise collapses within two features.
 *
 * # Why it searches tasks *and* commands
 *
 * A user typing `WR-125` means a task, and a user typing `board` means a
 * command; asking them which mode they are in is asking them to know the app's
 * internals. Both are matched, commands first because they are instantaneous and
 * the task query is a round trip — a list that reorders when the network answers
 * is a list you cannot select from.
 *
 * # The combobox contract
 *
 * `role="combobox"` on the input with `aria-activedescendant` pointing at the
 * highlighted option, and a `role="listbox"` beside it. Focus never leaves the
 * input, which is what lets arrow keys move the selection while typing keeps
 * working — the pattern WCAG 2.2 expects, and the one a `div` with `onKeyDown`
 * only approximates.
 */
import { useEffect, useMemo, useRef, useState, type ReactElement } from 'react'
import { useNavigate } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'

import { keys } from '../api/keys'
import { listTasks } from '../api/tasks'
import { useFocusTrap } from '../shell/focusTrap'
import { useOpenTask, useUpdateSearch } from '../shell/navigation'
import { useSession, useWorkspaceId } from '../shell/session'
import { rank, type Command } from './commands'

/** Long enough not to query per keystroke; the command half stays instant regardless. */
const DEBOUNCE_MS = 200

export function CommandPalette(): ReactElement | null {
  const [open, setOpen] = useState(false)
  const [term, setTerm] = useState('')

  useEffect(() => {
    function onKeyDown(event: KeyboardEvent): void {
      // `metaKey` for macOS, `ctrlKey` elsewhere. Both, because docs/18's
      // browser matrix is not an OS matrix and a Linux user pressing Ctrl+K
      // expects the same thing.
      if ((event.metaKey || event.ctrlKey) && event.key.toLowerCase() === 'k') {
        event.preventDefault()
        setOpen((current) => !current)
      }
    }
    window.addEventListener('keydown', onKeyDown)
    return () => window.removeEventListener('keydown', onKeyDown)
  }, [])

  useEffect(() => {
    if (!open) setTerm('')
  }, [open])

  if (!open) return null
  return <PaletteOverlay term={term} setTerm={setTerm} close={() => setOpen(false)} />
}

function PaletteOverlay({
  term,
  setTerm,
  close,
}: {
  term: string
  setTerm: (next: string) => void
  close: () => void
}): ReactElement {
  const panel = useRef<HTMLDivElement>(null)
  const navigate = useNavigate()
  const openTask = useOpenTask()
  const update = useUpdateSearch()
  const workspaceId = useWorkspaceId()
  const { workspaces, chooseWorkspace } = useSession()
  const [highlighted, setHighlighted] = useState(0)
  const [debounced, setDebounced] = useState(term)

  useFocusTrap(panel, close)

  useEffect(() => {
    const timer = setTimeout(() => setDebounced(term), DEBOUNCE_MS)
    return () => clearTimeout(timer)
  }, [term])

  const commands = useMemo<Command[]>(() => {
    const go = (to: string) => () => {
      void navigate({ to, search: (current: Record<string, unknown>) => current })
      close()
    }
    const built: Command[] = [
      { id: 'go-my-work', title: 'Go to My Work', group: 'Go', keywords: 'mine assigned', run: go('/my-work') },
      { id: 'go-list', title: 'Go to Tasks', group: 'Go', keywords: 'list table', run: go('/') },
      { id: 'go-board', title: 'Go to Board', group: 'Go', keywords: 'kanban columns', run: go('/board') },
      { id: 'go-reports', title: 'Go to Reports', group: 'Go', keywords: 'charts metrics', run: go('/reports') },
      {
        id: 'view-clear',
        title: 'Clear filters',
        group: 'View',
        keywords: 'reset all projects',
        run: () => {
          update({ project: undefined, q: undefined })
          close()
        },
      },
    ]
    for (const workspace of workspaces) {
      built.push({
        id: `ws-${workspace.id}`,
        title: `Switch to ${workspace.name}`,
        group: 'Go',
        keywords: `workspace ${workspace.slug}`,
        run: () => {
          chooseWorkspace(workspace.id)
          close()
        },
      })
    }
    return built
  }, [navigate, close, update, workspaces, chooseWorkspace])

  const matches = useMemo(() => rank(commands, term).slice(0, 8), [commands, term])

  const tasks = useQuery({
    queryKey: [...keys.taskLists(workspaceId), 'palette', debounced],
    queryFn: ({ signal }) =>
      listTasks(workspaceId, { filter: { q: debounced }, limit: 8 }, signal),
    enabled: workspaceId !== '' && debounced.trim().length >= 2,
    staleTime: 30_000,
  })

  const taskRows = tasks.data?.data ?? []
  const total = matches.length + taskRows.length

  useEffect(() => setHighlighted(0), [term, taskRows.length])

  function activate(index: number): void {
    const command = matches[index]
    if (command !== undefined) {
      command.run()
      return
    }
    const task = taskRows[index - matches.length]
    if (task !== undefined) {
      openTask(task.id)
      close()
    }
  }

  function onKeyDown(event: React.KeyboardEvent): void {
    if (event.key === 'ArrowDown') {
      event.preventDefault()
      setHighlighted((current) => (total === 0 ? 0 : (current + 1) % total))
    } else if (event.key === 'ArrowUp') {
      event.preventDefault()
      setHighlighted((current) => (total === 0 ? 0 : (current - 1 + total) % total))
    } else if (event.key === 'Enter') {
      event.preventDefault()
      activate(highlighted)
    }
  }

  const optionId = (index: number): string => `palette-option-${index}`

  return (
    <div className="palette">
      <div className="palette__scrim" onClick={close} aria-hidden="true" />
      <div className="palette__panel" ref={panel} role="dialog" aria-modal="true" aria-label="Commands">
        <input
          className="palette__input"
          type="text"
          role="combobox"
          aria-expanded="true"
          aria-controls="palette-list"
          aria-autocomplete="list"
          aria-activedescendant={total === 0 ? undefined : optionId(highlighted)}
          placeholder="Type a command or search tasks…"
          value={term}
          onChange={(event) => setTerm(event.target.value)}
          onKeyDown={onKeyDown}
        />

        <ul className="palette__list" id="palette-list" role="listbox" aria-label="Results">
          {matches.map((command, index) => (
            <li
              key={command.id}
              id={optionId(index)}
              role="option"
              aria-selected={highlighted === index}
              className={`palette__option${highlighted === index ? ' palette__option--on' : ''}`}
              onMouseEnter={() => setHighlighted(index)}
              onClick={() => activate(index)}
            >
              <span className="palette__group">{command.group}</span>
              <span className="palette__title">{command.title}</span>
              {command.hint === undefined ? null : <kbd>{command.hint}</kbd>}
            </li>
          ))}

          {taskRows.map((task, offset) => {
            const index = matches.length + offset
            return (
              <li
                key={task.id}
                id={optionId(index)}
                role="option"
                aria-selected={highlighted === index}
                className={`palette__option${highlighted === index ? ' palette__option--on' : ''}`}
                onMouseEnter={() => setHighlighted(index)}
                onClick={() => activate(index)}
              >
                <span className="palette__group key">{task.key}</span>
                <span className="palette__title">{task.title}</span>
              </li>
            )
          })}

          {total === 0 ? <li className="palette__empty">No matches.</li> : null}
        </ul>

        <footer className="palette__foot">
          <kbd>↑</kbd>
          <kbd>↓</kbd> to move · <kbd>↵</kbd> to choose · <kbd>esc</kbd> to close
        </footer>
      </div>
    </div>
  )
}
