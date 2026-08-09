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
 *
 * The command *contents* are not here: they come from the extension point
 * registry through `palette/registry.ts`. See that file for why.
 */
import { useCallback, useEffect, useMemo, useRef, useState, type ReactElement } from 'react'
import { useNavigate } from '@tanstack/react-router'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'

import { keys } from '../api/keys'
import { PERMISSIONS } from '../api/permissions'
import { listProjects } from '../api/projects'
import { createTask, listTasks } from '../api/tasks'
import { useAnnounce } from '../shell/announce'
import { useFocusTrap } from '../shell/focusTrap'
import { useAppSearch, useOpenTask, useUpdateSearch } from '../shell/navigation'
import { useAuthority } from '../shell/permissions'
import { useSession, useWorkspaceId } from '../shell/session'
import { rank } from './commands'
import { buildCommands } from './registry'

/** Long enough not to query per keystroke; the command half stays instant regardless. */
const DEBOUNCE_MS = 200

export function CommandPalette(): ReactElement | null {
  const [open, setOpen] = useState(false)

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

  if (!open) return null
  return <PaletteOverlay close={() => setOpen(false)} />
}

function PaletteOverlay({ close }: { close: () => void }): ReactElement {
  const panel = useRef<HTMLDivElement>(null)
  const navigate = useNavigate()
  const client = useQueryClient()
  const announce = useAnnounce()
  const openTask = useOpenTask()
  const update = useUpdateSearch()
  const search = useAppSearch()
  const workspaceId = useWorkspaceId()
  const { workspaces, chooseWorkspace } = useSession()
  const authority = useAuthority(search.project)

  const [term, setTerm] = useState('')
  const [highlighted, setHighlighted] = useState(0)
  const [debounced, setDebounced] = useState('')

  useFocusTrap(panel, close)

  useEffect(() => {
    const timer = setTimeout(() => setDebounced(term), DEBOUNCE_MS)
    return () => clearTimeout(timer)
  }, [term])

  const projects = useQuery({
    queryKey: keys.projects(workspaceId),
    queryFn: ({ signal }) => listProjects(workspaceId, signal),
    enabled: workspaceId !== '',
    staleTime: 60_000,
  })

  const create = useMutation({
    mutationFn: ({ projectId, title }: { projectId: string; title: string }) =>
      createTask(workspaceId, projectId, { title }),
    onSuccess: (task) => {
      announce(`Created ${task.key}`)
      void client.invalidateQueries({ queryKey: keys.taskLists(workspaceId) })
      openTask(task.id)
      close()
    },
  })

  const go = useCallback(
    (to: string) => {
      void navigate({ to, search: ((current: unknown) => current) as never })
      close()
    },
    [navigate, close],
  )

  const commands = useMemo(
    () =>
      buildCommands({
        term,
        projects: (projects.data?.data ?? []).map((p) => ({ id: p.id, key: p.key, name: p.name })),
        scopedProject: search.project,
        workspaces: workspaces.map((w) => ({ id: w.id, name: w.name, slug: w.slug })),
        mayCreateTask: authority.can(PERMISSIONS.taskCreate),
        go,
        setProject: (id) => {
          update({ project: id })
          close()
        },
        chooseWorkspace: (id) => {
          chooseWorkspace(id)
          close()
        },
        createTask: (projectId, title) => create.mutate({ projectId, title }),
        clearFilters: () => {
          update({ project: undefined, q: undefined })
          close()
        },
      }),
    [term, projects.data, search.project, workspaces, authority, go, update, close, chooseWorkspace, create],
  )

  const matches = useMemo(() => rank(commands, term).slice(0, 8), [commands, term])

  const tasks = useQuery({
    queryKey: [...keys.taskLists(workspaceId), 'palette', debounced],
    queryFn: ({ signal }) => listTasks(workspaceId, { filter: { q: debounced }, limit: 8 }, signal),
    enabled: workspaceId !== '' && debounced.trim().length >= 2,
    staleTime: 30_000,
  })

  const taskRows = tasks.data?.data ?? []
  const total = matches.length + taskRows.length

  useEffect(() => setHighlighted(0), [term, taskRows.length])

  function activate(index: number): void {
    const command = matches[index]
    if (command !== undefined) {
      if (command.unavailable !== undefined) return
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
              aria-disabled={command.unavailable !== undefined}
              className={`palette__option${highlighted === index ? ' palette__option--on' : ''}${
                command.unavailable === undefined ? '' : ' palette__option--off'
              }`}
              onMouseEnter={() => setHighlighted(index)}
              onClick={() => activate(index)}
            >
              <span className="palette__group">{command.group}</span>
              <span className="palette__title">{command.title}</span>
              {command.unavailable === undefined ? (
                command.hint === undefined ? null : (
                  <kbd>{command.hint}</kbd>
                )
              ) : (
                <span className="palette__why">{command.unavailable}</span>
              )}
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
