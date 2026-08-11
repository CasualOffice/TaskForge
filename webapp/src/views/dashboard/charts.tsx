/**
 * The chart set — closed, and drawn by hand.
 *
 * # Why there is no charting library
 *
 * ADR-024 budgets 200 KiB gzip for the initial shell and `docs/42` makes the
 * charting library lazy *when there is one*. Recharts is ~95 KiB gzip and
 * D3-scale plus a renderer is not much better; against 40 KiB of headroom that
 * is not a lazy chunk away from being a problem, it is a second bundle every
 * dashboard viewer downloads to draw four rectangles and a polyline.
 *
 * `docs/38` closes the visualization set precisely so this is possible: six
 * shapes, no chart builder, no arbitrary config. A closed set is a set you can
 * draw yourself. What a library would buy — axis tick placement, path
 * generation, a legend — is under 200 lines here, and what it would cost is the
 * budget plus a rendering model that fights every accessibility requirement
 * below.
 *
 * # Why every chart is also a table
 *
 * `docs/47`'s accessibility contract: a chart is an image of data, and an image
 * of data is unreadable to a screen reader, unreadable at 200% zoom on a phone,
 * and invisible to anyone whose CSS failed to load. So each chart renders the
 * SVG `aria-hidden` beside a visually-hidden `<table>` carrying the same
 * numbers. The table is not a fallback — it is the content, and the drawing is
 * the decoration. That is also why no number here is encoded by colour alone.
 */
import type { ReactElement } from 'react'

/** One plotted value. `label` is already presentation-ready — charts never map ids. */
export interface Point {
  readonly label: string
  readonly value: number
  /** What the value means in words, for the row a screen reader reads. */
  readonly formatted: string
}

/**
 * The data table every chart carries.
 *
 * `.visually-hidden` rather than `aria-label` on the SVG: a label can say "a
 * bar chart of open tasks by assignee" but it cannot say what the numbers are,
 * and the numbers are the entire point of the tile.
 */
function DataTable({
  caption,
  dimension,
  measure,
  points,
}: {
  caption: string
  dimension: string
  measure: string
  points: readonly Point[]
}): ReactElement {
  // The wrapper is load-bearing, not tidiness. `.visually-hidden` clips with
  // `width: 1px; overflow: hidden`, and neither constrains `display: table` —
  // a table sizes to its content whatever its width says, and tables do not
  // clip. A bare `<table class="visually-hidden">` therefore stayed invisible
  // while still pushing the *document* 60 px wider than the viewport, which is
  // a horizontal scrollbar caused by content nobody can see. Clipping happens
  // on the block wrapper, where `overflow` means what it says.
  return (
    <div className="visually-hidden">
      <table>
        <caption>{caption}</caption>
        <thead>
          <tr>
            <th scope="col">{dimension}</th>
            <th scope="col">{measure}</th>
          </tr>
        </thead>
        <tbody>
          {points.map((point) => (
            <tr key={point.label}>
              <th scope="row">{point.label}</th>
              <td>{point.formatted}</td>
            </tr>
          ))}
        </tbody>
      </table>
    </div>
  )
}

/**
 * A single number, and what it counts.
 *
 * The one "chart" with no axes, and the one that carries the most weight: an
 * unlabelled number on a dashboard is a number people misquote.
 */
export function NumberChart({ value, unit }: { value: string; unit: string }): ReactElement {
  return (
    <p className="chart__number">
      <span className="chart__value">{value}</span>
      <span className="chart__unit">{unit}</span>
    </p>
  )
}

/**
 * Horizontal bars, one per slice.
 *
 * Horizontal rather than vertical because the labels are names — assignees,
 * statuses, projects — and a vertical bar chart either truncates them or turns
 * them 45°, which is a readability cost paid for nothing. The value sits at the
 * end of each bar rather than inside it: inside, it disappears whenever the bar
 * is shorter than the text.
 */
export function BarChart({
  points,
  caption,
  dimension,
  measure,
}: {
  points: readonly Point[]
  caption: string
  dimension: string
  measure: string
}): ReactElement {
  // Against the largest value, not the total: these are magnitudes, not parts
  // of a whole, and scaling them to a total makes every bar shrink whenever an
  // unrelated slice grows.
  const peak = Math.max(...points.map((p) => p.value), 1)
  return (
    <div className="chart">
      <ul className="chart__bars" aria-hidden="true">
        {points.map((point) => (
          <li className="chart__bar" key={point.label}>
            <span className="chart__barlabel" title={point.label}>
              {point.label}
            </span>
            <span className="chart__track">
              <span className="chart__fill" style={{ width: `${(point.value / peak) * 100}%` }} />
            </span>
            <span className="chart__barvalue">{point.formatted}</span>
          </li>
        ))}
      </ul>
      <DataTable caption={caption} dimension={dimension} measure={measure} points={points} />
    </div>
  )
}

/** The drawing box. Fixed, with `preserveAspectRatio` off, so CSS sizes it. */
const W = 320
const H = 120
const PAD = { top: 8, right: 8, bottom: 18, left: 8 }

/**
 * A time series.
 *
 * Points are evenly spaced by *position*, not by date, because the server
 * returns one row per bucket and the buckets are already uniform. Spacing them
 * by timestamp would only matter for gaps, and a gap in a `date_trunc` series
 * is a bucket with no rows — which this fills as zero rather than skipping,
 * because a line that jumps a missing week is a line that lies about the trend.
 */
export function LineChart({
  points,
  caption,
  dimension,
  measure,
}: {
  points: readonly Point[]
  caption: string
  dimension: string
  measure: string
}): ReactElement {
  const peak = Math.max(...points.map((p) => p.value), 1)
  const span = Math.max(points.length - 1, 1)
  const x = (i: number): number => PAD.left + (i / span) * (W - PAD.left - PAD.right)
  const y = (v: number): number => H - PAD.bottom - (v / peak) * (H - PAD.top - PAD.bottom)

  const line = points.map((p, i) => `${i === 0 ? 'M' : 'L'}${x(i)},${y(p.value)}`).join(' ')
  // Closed back along the baseline, so the fill has an area rather than a
  // stroke with a fill rule nobody can predict.
  const area =
    points.length === 0
      ? ''
      : `${line} L${x(points.length - 1)},${H - PAD.bottom} L${x(0)},${H - PAD.bottom} Z`

  return (
    <div className="chart">
      <svg
        className="chart__svg"
        viewBox={`0 0 ${W} ${H}`}
        preserveAspectRatio="none"
        aria-hidden="true"
        focusable="false"
      >
        <line
          className="chart__axis"
          x1={PAD.left}
          y1={H - PAD.bottom}
          x2={W - PAD.right}
          y2={H - PAD.bottom}
        />
        <path className="chart__area" d={area} />
        <path className="chart__line" d={line} />
        {points.map((point, i) => (
          <circle className="chart__dot" key={point.label} cx={x(i)} cy={y(point.value)} r={2.5} />
        ))}
      </svg>
      <p className="chart__span" aria-hidden="true">
        <span>{points[0]?.label ?? ''}</span>
        <span>{points.at(-1)?.label ?? ''}</span>
      </p>
      <DataTable caption={caption} dimension={dimension} measure={measure} points={points} />
    </div>
  )
}

/**
 * One bar, divided into its parts.
 *
 * Used where the slices sum to something meaningful — every open task is in
 * exactly one state — which is the case a plain bar chart reads badly: five
 * separate bars invite comparing them to each other rather than to the whole.
 *
 * Each segment carries its own label rather than relying on a colour key,
 * because a legend that maps colour to meaning fails for the ~4% of readers who
 * cannot distinguish the colours, and fails for everyone in print.
 */
export function StackedBarChart({
  points,
  caption,
  dimension,
  measure,
}: {
  points: readonly Point[]
  caption: string
  dimension: string
  measure: string
}): ReactElement {
  const total = points.reduce((sum, p) => sum + p.value, 0)
  const shown = points.filter((p) => p.value > 0)
  return (
    <div className="chart">
      <div className="chart__stack" aria-hidden="true">
        {shown.map((point, i) => (
          <span
            className="chart__segment"
            key={point.label}
            style={{
              width: `${(point.value / Math.max(total, 1)) * 100}%`,
              // A ramp rather than a palette: the segments of one stack are
              // ordered (a workflow's states are), so their shading should be
              // too. Distinct hues would imply they are unrelated categories.
              opacity: 1 - i * (0.6 / Math.max(shown.length, 1)),
            }}
          />
        ))}
      </div>
      <ul className="chart__key" aria-hidden="true">
        {shown.map((point, i) => (
          <li key={point.label}>
            <span
              className="chart__swatch"
              style={{ opacity: 1 - i * (0.6 / Math.max(shown.length, 1)) }}
            />
            {point.label}
            <span className="chart__keyvalue">{point.formatted}</span>
          </li>
        ))}
      </ul>
      <DataTable caption={caption} dimension={dimension} measure={measure} points={points} />
    </div>
  )
}

/**
 * The numbers, plainly.
 *
 * Not a lesser chart: for a handful of precise values — p50 and p90 beside each
 * other — a table is what a reader actually wants, and `docs/38` lists it in the
 * closed set for that reason. Visible here, rather than the hidden twin the
 * drawn charts carry.
 */
export function TableChart({
  points,
  caption,
  dimension,
  measure,
}: {
  points: readonly Point[]
  caption: string
  dimension: string
  measure: string
}): ReactElement {
  return (
    <table className="chart__table">
      <caption className="visually-hidden">{caption}</caption>
      <thead>
        <tr>
          <th scope="col">{dimension}</th>
          <th scope="col">{measure}</th>
        </tr>
      </thead>
      <tbody>
        {points.map((point) => (
          <tr key={point.label}>
            <th scope="row">{point.label}</th>
            <td>{point.formatted}</td>
          </tr>
        ))}
      </tbody>
    </table>
  )
}
