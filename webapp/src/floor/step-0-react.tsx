// Step 0 — react + react-dom only. Everything above this line in the ladder is
// measured as a marginal delta against it (see scripts/measure-deps.mjs).
import { StrictMode, useState, type ReactElement } from 'react'
import { createRoot } from 'react-dom/client'

function App(): ReactElement {
  const [n, setN] = useState(0)
  return (
    <button type="button" onClick={() => setN((v) => v + 1)}>
      {n}
    </button>
  )
}

const host = document.getElementById('root')
if (host === null) throw new Error('#root missing')
createRoot(host).render(
  <StrictMode>
    <App />
  </StrictMode>,
)
