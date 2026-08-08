# TaskForge — 03 Lightweight Client Architecture

## 9. Lightweight Client Requirements
- The client must remain a thin interaction layer, not a second application server. Business rules, authorization, filtering, sorting and workflow validation belong on the backend.
- Initial authenticated shell target: approximately 150–200 KB compressed JavaScript where practical, excluding optional editor and plugin chunks. Exact budgets should be enforced in CI and adjusted only through architecture review.
- Route-level and feature-level code splitting is mandatory. Calendar, timeline, reports, admin screens, rich text, charts and plugin UI load only when opened.
- Use server-side cursor pagination, filtering, sorting and search. Never download an entire large project to render a board or list.
- Virtualize long lists, boards and activity streams. Render only visible rows/cards plus a small overscan window.
- Use SSE for most server-to-client updates; WebSocket is optional where bidirectional low-latency interaction is genuinely required. Batch and coalesce events.
- Use optimistic updates for common mutations, with version-aware rollback on conflict. Avoid broad client state stores; server state belongs in TanStack Query and local transient UI state remains local.
- No heavy office editor, canvas framework, embedded analytics runtime or large component suite in the core bundle. Rich text should be loaded only when the user edits content.
- Prefer native CSS, CSS variables and accessible headless primitives. Avoid shipping multiple icon libraries and duplicate date/time packages.
- Performance targets: first usable shell under 2.5 seconds on a mid-tier laptop and constrained broadband; p75 interaction latency under 100 ms for local UI actions; no long tasks over 200 ms during common board/list interaction; smooth 60 fps dragging where hardware permits.
- Offline-first is not required initially. A small draft cache and retry queue are acceptable, but do not ship a large synchronization engine until product need is proven.

## 8. UX and Information Architecture
- Permanent navigation: Home, My Work, Inbox, Search, Teams/Projects, Saved Views and Create. Administration appears only when authorized.
- Project views in core: Overview, Board, List and Activity. Calendar, Timeline, Workload, Releases and specialized reports can arrive later or through plugins.
- Clicking a task opens a side drawer while preserving board/list context. A dedicated full-page route remains available for deep links.
- Task creation initially asks only for title, project, status and optional assignee. Description, due date, tags, environment, milestone and custom fields are progressively disclosed.
- My Work aggregates assignments across accessible projects into Today, Overdue, Upcoming, Blocked and Recently Completed.
- Command palette handles create, navigate, assign, transition, search and plugin commands without bloating permanent navigation.
