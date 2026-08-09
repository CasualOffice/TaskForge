/**
 * The live region every optimistic outcome is announced through — and the toast
 * that shows the same sentence to people who are looking at the screen.
 *
 * # The failure this module prevents
 *
 * An interface that only *looks* like it worked. docs/42 §Accessibility: "live
 * regions announce optimistic outcomes and errors." A card that slides to
 * another column is feedback for people who can see it slide; for everyone else
 * a drag either announces its result or produces silence indistinguishable from
 * a dropped event.
 *
 * The inverse failure is the one this file used to have: the *only* output was
 * the visually-hidden region, so a sighted user who saved a description, moved a
 * task or posted a comment got no confirmation at all. Every call site already
 * wrote the right sentence; it simply had nowhere visible to go. It does now,
 * and the two outputs cannot drift because they are the same string.
 *
 * One region for the whole app, not one per view: two `aria-live` regions
 * updating in the same tick race, and screen readers resolve that race by
 * dropping one of them. The toasts are `aria-hidden` for the same reason — they
 * are the visual half of an announcement the region has already made, and
 * announcing both would say everything twice.
 */
import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useMemo,
  useRef,
  useState,
  type ReactElement,
  type ReactNode,
} from 'react'

/** `error` is for a refusal the user must notice; it does not auto-dismiss. */
export type Tone = 'info' | 'error'

type Announce = (message: string, tone?: Tone) => void

interface Toast {
  readonly id: number
  readonly message: string
  readonly tone: Tone
}

/** Long enough to read a sentence, short enough not to sit over the work. */
const DISMISS_MS = 5_000
/** Beyond this the stack is a wall; the oldest fall off. */
const MAX_VISIBLE = 3

const AnnounceContext = createContext<Announce>(() => undefined)

export function Announcer({ children }: { children: ReactNode }): ReactElement {
  const [message, setMessage] = useState('')
  const [toasts, setToasts] = useState<readonly Toast[]>([])
  const nextId = useRef(0)

  const announce = useCallback<Announce>((next, tone = 'info') => {
    // Cleared first so an identical consecutive message still counts as a
    // change. Announcing "Moved to Active" twice in a row is otherwise silent
    // the second time, which is exactly when a user is checking whether their
    // second attempt worked.
    setMessage('')
    requestAnimationFrame(() => setMessage(next))
    nextId.current += 1
    const toast: Toast = { id: nextId.current, message: next, tone }
    setToasts((current) => [...current, toast].slice(-MAX_VISIBLE))
  }, [])

  const dismiss = useCallback((id: number) => {
    setToasts((current) => current.filter((toast) => toast.id !== id))
  }, [])

  const value = useMemo(() => announce, [announce])

  return (
    <AnnounceContext.Provider value={value}>
      {children}
      {/* `polite`, not `assertive`: these outcomes should not interrupt a user
          mid-sentence. `atomic` so the whole sentence is read, not the diff. */}
      <div className="visually-hidden" role="status" aria-live="polite" aria-atomic="true">
        {message}
      </div>
      {toasts.length === 0 ? null : (
        <div className="toasts" aria-hidden="true">
          {toasts.map((toast) => (
            <ToastItem key={toast.id} toast={toast} onDismiss={dismiss} />
          ))}
        </div>
      )}
    </AnnounceContext.Provider>
  )
}

function ToastItem({
  toast,
  onDismiss,
}: {
  toast: Toast
  onDismiss: (id: number) => void
}): ReactElement {
  useEffect(() => {
    // A refusal stays until it is dismissed. An error that disappears while the
    // user is still reading it is a failure they will report as "nothing
    // happened", which is the opposite of what it said.
    if (toast.tone === 'error') return
    const timer = setTimeout(() => onDismiss(toast.id), DISMISS_MS)
    return () => clearTimeout(timer)
  }, [toast.id, toast.tone, onDismiss])

  return (
    <div className={`toast toast--${toast.tone}`}>
      <span className="toast__text">{toast.message}</span>
      {/* The whole stack is `aria-hidden`, so this must not be a tab stop
          announcing nothing: the toast dismisses itself on a timer and the
          sentence is already in the live region. */}
      <button
        type="button"
        className="toast__close"
        tabIndex={-1}
        onClick={() => onDismiss(toast.id)}
      >
        ✕
      </button>
    </div>
  )
}

export function useAnnounce(): Announce {
  return useContext(AnnounceContext)
}
