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

export { TableChart } from './TableChart'

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

/**
 * Composition, as a share of the whole.
 *
 * The chart a stacked bar cannot replace, which is why it is here rather than
 * being called a duplicate of one: a stacked bar is read as a *length* and
 * invites comparing segments to each other, while a ring is read as a
 * *proportion* and answers "how much of the work is this" directly. "Half our
 * open work is bugs" is a sentence a donut says and a bar does not.
 *
 * A donut rather than a pie: the hole carries the total, which is the number
 * people actually want beside the shares, and it removes the centre — the part
 * of a pie where every slice converges and none is readable.
 *
 * Drawn with `stroke-dasharray` on one circle per slice rather than as arc
 * paths. Same picture, no trigonometry, and each slice is one element that can
 * carry its own title — an arc path generator is the part of a charting library
 * this would otherwise have been imported for.
 */
export function DonutChart({
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
  // A circle of radius 50 has a circumference of 2πr; using that as the
  // dash-array total means a slice's dash length *is* its percentage.
  const RADIUS = 50
  const CIRCUMFERENCE = 2 * Math.PI * RADIUS

  let consumed = 0
  const slices = shown.map((point, i) => {
    const share = point.value / Math.max(total, 1)
    const slice = {
      key: point.label,
      length: share * CIRCUMFERENCE,
      // Negative, because SVG strokes run clockwise from 3 o'clock and the
      // offset counts backwards. Rotated -90° below so the first slice starts
      // at the top, where a reader expects it.
      offset: -consumed * CIRCUMFERENCE,
      shade: 1 - i * (0.62 / Math.max(shown.length, 1)),
      formatted: point.formatted,
      percent: Math.round(share * 100),
    }
    consumed += share
    return slice
  })

  return (
    <div className="chart chart--donut">
      <div className="chart__ring">
        <svg viewBox="0 0 120 120" aria-hidden="true" focusable="false">
          <g transform="rotate(-90 60 60)">
            <circle className="chart__ringtrack" cx="60" cy="60" r={RADIUS} />
            {slices.map((slice) => (
              <circle
                className="chart__slice"
                key={slice.key}
                cx="60"
                cy="60"
                r={RADIUS}
                strokeDasharray={`${slice.length} ${CIRCUMFERENCE - slice.length}`}
                strokeDashoffset={slice.offset}
                style={{ opacity: slice.shade }}
              />
            ))}
          </g>
        </svg>
        {/* The total, in the hole. A ring without it makes a reader estimate
            the whole from the parts, which is the one thing a proportion chart
            is bad at. */}
        <p className="chart__ringtotal" aria-hidden="true">
          <span className="chart__ringvalue">{total}</span>
          <span className="chart__ringunit">total</span>
        </p>
      </div>

      <ul className="chart__key" aria-hidden="true">
        {slices.map((slice) => (
          <li key={slice.key}>
            <span className="chart__swatch" style={{ opacity: slice.shade }} />
            {slice.key}
            <span className="chart__keyvalue">
              {slice.formatted} · {slice.percent}%
            </span>
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
const PAD = { top: 10, right: 8, bottom: 18, left: 30 }

/**
 * An axis label, short enough for 30 px of gutter.
 *
 * Halves can land on `.5` for an odd peak, and a gridline reading "3.5 tasks"
 * is noise — the midline exists to help you place a point, not to be quoted.
 */
function formatTick(value: number): string {
  if (value >= 1000) return `${Math.round(value / 100) / 10}k`
  return String(Math.round(value))
}

/**
 * A time series.
 *
 * Points are evenly spaced by *position*, not by date, because the server
 * returns one row per bucket and the buckets are already uniform. Spacing them
 * by timestamp would only matter for gaps, and a gap in a `date_trunc` series
 * is a bucket with no rows — which this fills as zero rather than skipping,
 * because a line that jumps a missing week is a line that lies about the trend.
 *
 * # Why it has a scale
 *
 * The first version drew the line and nothing else, which makes a trend
 * *shape* legible and its *size* unknowable: the same picture means "eight a
 * week" or "eighty a week" and a reader cannot tell which. Throughput and cycle
 * time are the two numbers on this page anyone judges performance by, so the
 * chart carries a zero baseline, a labelled peak, and a midline — three
 * gridlines, which is enough to read a value off and few enough not to become
 * the loudest thing in the tile.
 *
 * The baseline is always zero, never the minimum. A trend chart cropped to its
 * own range turns a rise from 40 to 44 into a cliff, and that is the most
 * common way a dashboard misleads without containing a single wrong number.
 */
export function LineChart({
  points,
  series,
  caption,
  dimension,
  measure,
}: {
  /** One series. Ignored when `series` is given. */
  points?: readonly Point[]
  /**
   * Two or more named series sharing one axis.
   *
   * `created_vs_completed` is the reason this exists: the whole message of that
   * chart is where the lines *cross*, and two tiles side by side cannot show a
   * crossing. Drawn on one scale — the peak of *all* series — because two lines
   * with independent axes can be made to cross anywhere.
   */
  series?: readonly { name: string; points: readonly Point[] }[]
  caption: string
  dimension: string
  measure: string
}): ReactElement {
  const lines = series ?? [{ name: measure, points: points ?? [] }]
  const peak = Math.max(...lines.flatMap((line) => line.points.map((p) => p.value)), 1)
  const longest = Math.max(...lines.map((line) => line.points.length), 1)
  const span = Math.max(longest - 1, 1)
  // A single bucket has no span to spread across, and dividing by one would pin
  // it to the left edge looking like a series that failed to load. It is not a
  // trend yet — it is one week — so it sits in the middle and says so below.
  const x = (i: number): number =>
    longest === 1
      ? (PAD.left + (W - PAD.right)) / 2
      : PAD.left + (i / span) * (W - PAD.left - PAD.right)
  const y = (v: number): number => H - PAD.bottom - (v / peak) * (H - PAD.top - PAD.bottom)

  const path = (of: readonly Point[]): string =>
    of.map((p, i) => `${i === 0 ? 'M' : 'L'}${x(i)},${y(p.value)}`).join(' ')

  const axis = lines[0]?.points ?? []

  return (
    <div className="chart">
      <svg
        className="chart__svg"
        viewBox={`0 0 ${W} ${H}`}
        preserveAspectRatio="none"
        aria-hidden="true"
        focusable="false"
      >
        {/* Zero, half and peak. Drawn before the series so the data sits on top
            of its own scale rather than under it. */}
        {[0, peak / 2, peak].map((value) => (
          <g key={value}>
            <line
              className={value === 0 ? 'chart__axis' : 'chart__grid'}
              x1={PAD.left}
              y1={y(value)}
              x2={W - PAD.right}
              y2={y(value)}
            />
            <text className="chart__tick" x={PAD.left - 5} y={y(value) + 3} textAnchor="end">
              {formatTick(value)}
            </text>
          </g>
        ))}

        {lines.map((line, index) => {
          const d = path(line.points)
          // Only the first series gets an area. Two filled regions overlapping
          // read as a third colour where they cross, which is exactly the point
          // of the chart and exactly where it must stay legible.
          const area =
            index === 0 && line.points.length > 0
              ? `${d} L${x(line.points.length - 1)},${H - PAD.bottom} L${x(0)},${H - PAD.bottom} Z`
              : undefined
          return (
            <g key={line.name} className={`chart__series chart__series--${index}`}>
              {area === undefined ? null : <path className="chart__area" d={area} />}
              <path className="chart__line" d={d} />
              {line.points.map((point, i) => (
                <circle
                  className="chart__dot"
                  key={point.label}
                  cx={x(i)}
                  cy={y(point.value)}
                  r={line.points.length === 1 ? 4 : 2.5}
                />
              ))}
            </g>
          )
        })}
      </svg>

      {/* Named, not colour-coded alone: two lines distinguished only by hue are
          two lines nobody with a colour vision deficiency can tell apart. Each
          key entry carries the series name beside its swatch. */}
      {series === undefined ? null : (
        <ul className="chart__key" aria-hidden="true">
          {lines.map((line, index) => (
            <li key={line.name}>
              <span className={`chart__swatch chart__swatch--${index}`} />
              {line.name}
            </li>
          ))}
        </ul>
      )}

      {/* First and last bucket. One bucket names itself once — repeating the
          same date at both ends reads as a range that lasted no time. */}
      <p className="chart__span" aria-hidden="true">
        {longest === 1 ? (
          <span className="chart__single">
            {axis[0]?.label} — one week so far, not yet a trend
          </span>
        ) : (
          <>
            <span>{axis[0]?.label ?? ''}</span>
            <span>{axis.at(-1)?.label ?? ''}</span>
          </>
        )}
      </p>

      {lines.map((line) => (
        <DataTable
          key={line.name}
          caption={series === undefined ? caption : `${caption} — ${line.name}`}
          dimension={dimension}
          measure={measure}
          points={line.points}
        />
      ))}
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
