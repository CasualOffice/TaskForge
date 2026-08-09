/**
 * The task, in full. `TaskDetail` — the default detail surface.
 *
 * # Why this is a route and not a drawer
 *
 * `design/LAYOUT-AND-INTERACTION-GUIDELINES.md` §4, rewritten after the drawer
 * was tried and failed: "a 420–560 px column cannot show what a task *is*, who
 * owns it, what it blocks, and what people have said about it without scrolling,
 * and everything below the fold is effectively invisible. A reader had to scroll
 * to learn the assignee."
 *
 * # The layout is the argument
 *
 * `design/DESIGN-FOUNDATION.md` §1.8: "Scrolling is a cost, not a layout tool. A
 * surface that scrolls to reveal a field has hidden that field." So the page
 * itself does not scroll — it is a fixed two-column grid inside the shell's main
 * area. The left column carries the narrative (title, description, relations,
 * conversation); the right column is metadata, short by construction. **Exactly
 * one region scrolls**, the comment thread, and it scrolls inside itself. §11
 * forbids the alternative by name: "nested scroll regions — a scrolling panel
 * inside a scrolling page".
 *
 * The consequence worth stating: a very long description gets its own bounded
 * region rather than pushing the conversation off the screen. A description is a
 * genuine tail; the fields around it are not.
 *
 * # It shares its parts with the peek
 *
 * §9 of the foundation names `TaskDetail` and `TaskPeek` and requires "one
 * information architecture and one component set, so a field cannot exist in one
 * and be forgotten in the other". Both compose `TaskTitle`, `TaskDescription`,
 * `TaskMeta` and `StatusControl`; the peek shows fewer of them, and cannot show
 * a *different* version of any of them.
 */
import { type ReactElement } from 'react'
import { Link } from '@tanstack/react-router'
import { useQuery } from '@tanstack/react-query'

import { keys } from '../api/keys'
import { listProjects } from '../api/projects'
import { readTask } from '../api/tasks'
import { Activity } from './Activity'
import { CommentThread } from './CommentThread'
import { Custody } from './Custody'
import { RelationsPanel } from './Relations'
import { Subtasks } from './Subtasks'
import { EmptyState } from '../shell/EmptyState'
import { GapNotice, ErrorNotice } from '../shell/notice'
import { useAuthority } from '../shell/permissions'
import { useWorkspaceId } from '../shell/session'
import { TypeBadge } from '../tasks/TaskCard'
import { TaskDescription, TaskTitle } from './TaskNarrative'
import { TaskMeta } from './TaskMeta'
import { unbuiltSentence } from './unbuilt'

export function TaskDetail({ taskId }: { taskId: string }): ReactElement {
  const workspaceId = useWorkspaceId()

  const task = useQuery({
    queryKey: keys.task(workspaceId, taskId),
    queryFn: ({ signal }) => readTask(workspaceId, taskId, signal),
    enabled: workspaceId !== '',
  })

  const projects = useQuery({
    queryKey: keys.projects(workspaceId),
    queryFn: ({ signal }) => listProjects(workspaceId, signal),
    enabled: workspaceId !== '',
    staleTime: 60_000,
  })

  // Project-scoped, not workspace-scoped: a constrained grant reaches a task
  // through its project, and asking at workspace scope would understate the
  // answer for anyone whose authority comes from a project role.
  const authority = useAuthority(task.data?.project_id)

  if (task.error != null) {
    return (
      <section className="view">
        <div className="detail__notice">
          <ErrorNotice error={task.error} />
        </div>
        <EmptyState
          title="That task could not be opened."
          actions={
            <Link to="/" className="linkbutton">
              Back to tasks
            </Link>
          }
        />
      </section>
    )
  }

  if (task.data === undefined) {
    return (
      <section className="view">
        <p className="empty" role="status">
          Loading task…
        </p>
      </section>
    )
  }

  const current = task.data
  const project = (projects.data?.data ?? []).find((entry) => entry.id === current.project_id)
  const missing = unbuiltSentence()

  return (
    <section className="detail" aria-label={`${current.key} ${current.title}`}>
      <header className="detail__bar">
        {/* A real link, scoped to the project, so it works from a pasted URL in
            a fresh tab where there is no history to go back through. */}
        <Link
          to="/"
          search={{ project: current.project_id }}
          className="linkbutton linkbutton--quiet"
        >
          ← {project?.key ?? 'Tasks'}
        </Link>
        <span className="key">{current.key}</span>
        <TypeBadge type={current.type} />
      </header>

      <div className="detail__cols">
        <div className="detail__main">
          <TaskTitle task={current} authority={authority} />

          <div className="detail__scrollable">
            <TaskDescription task={current} authority={authority} />

            {/* §12: blockers and subtasks are two lists, never one. A blocker
                gates this task's transitions; a subtask is part of its scope and
                gates nothing. */}
            {/* Custody first among the panels: the moment someone opens a task
                is usually the moment they are about to pass it on. */}
            <Custody task={current} authority={authority} />
            <Subtasks taskId={current.id} />
            <RelationsPanel taskId={current.id} taskKey={current.key} authority={authority} />
            <Activity taskId={current.id} authority={authority} />

            {/* Whatever the registry declares and nothing serves. One line, not
                one box each. */}
            {missing === undefined ? null : <GapNotice what={missing} />}

            {/* Inside the scroll region, not below it. A conversation is the
                end of the page, and a second scroller pinned under the first
                only ever cut the section above it in half. */}
            <section className="detail__comments" aria-labelledby="comments-heading">
              <h2 id="comments-heading" className="narr__heading">
                Comments
              </h2>
              <CommentThread taskId={current.id} authority={authority} />
            </section>
          </div>
        </div>

        <aside className="detail__side" aria-label="Task metadata">
          <TaskMeta task={current} authority={authority} projectName={project?.name} />
        </aside>
      </div>
    </section>
  )
}
