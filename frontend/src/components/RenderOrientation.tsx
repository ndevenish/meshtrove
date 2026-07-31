import { useEffect, useRef, useState } from 'react'
import {
  Box,
  Button,
  CircularProgress,
  Divider,
  IconButton,
  Popover,
  Stack,
  ToggleButton,
  ToggleButtonGroup,
  Tooltip,
  Typography,
} from '@mui/material'
import ThreeDRotationIcon from '@mui/icons-material/ThreeDRotation'
import RotateLeftIcon from '@mui/icons-material/RotateLeft'
import RotateRightIcon from '@mui/icons-material/RotateRight'
import SwapVertIcon from '@mui/icons-material/SwapVert'

import {
  api,
  waitForJob,
  type ImageRecord,
  type OrientableRenders,
  type RenderOverrides,
} from '../api'

/// The turntable turns in eighths. Finer than that is a slider, and a slider is
/// a lot of renders to find the angle you could have clicked to.
const STEP = 45

/// Whether these controls apply at all: only a *rendered* picture can be
/// rendered again, and only while the file it came from is still there
/// (`source_file_id` is cleared when the model file goes).
export function canReorient(image: ImageRecord): boolean {
  return image.kind === 'rendered' && !!image.source_file_id
}

/// All the button needs of a picture: which one to re-render, and where it
/// currently stands. A gallery's `ImageRecord` is one — so is a bundle member's
/// preview, which that page has as a `MemberPreview` and never as a full record
/// (the server already vouched for it being re-renderable, so there is nothing
/// for `canReorient` to check).
export type Reorientable = Pick<ImageRecord, 'id' | 'render_overrides'>

/// Split `+Z` into the axis and its sign, so the UI can offer three axes and a
/// flip rather than six near-identical buttons.
function splitAxis(up: string | null | undefined): { axis: string; negative: boolean } {
  if (!up) return { axis: '', negative: false }
  return { axis: up.slice(1), negative: up.startsWith('-') }
}

// --- the axis gizmo -------------------------------------------------------
//
// Three buttons labelled X, Y and Z say nothing about which way those axes
// actually point, so picking one is a guess followed by a render. The gizmo
// answers it: it stands the axes up in the same camera the renderer will use,
// so you can read off which one is currently up and which way a turn will take
// you before spending a render on it.

type Vec3 = [number, number, number]

/// Must match the backend's `DEFAULT_CAMERA_DIRECTION` (services/renderer.rs)
/// and the `--camera-direction` in the shipped renderer config. An admin who
/// edits that setting moves the real camera and not this drawing of it — the
/// gizmo would then be a stale sketch of where the camera used to stand.
const BASE_DIRECTION: Vec3 = [-1, -0.6, -1]

const AXES: { name: string; vector: Vec3; colour: string }[] = [
  // Readable on both themes, which f3d's own axis colours (a very light green
  // in particular) are not against a white popover.
  { name: 'X', vector: [1, 0, 0], colour: '#e5484d' },
  { name: 'Y', vector: [0, 1, 0], colour: '#2f9e5f' },
  { name: 'Z', vector: [0, 0, 1], colour: '#3e63dd' },
]

const dot = (a: Vec3, b: Vec3) => a[0] * b[0] + a[1] * b[1] + a[2] * b[2]
const cross = (a: Vec3, b: Vec3): Vec3 => [
  a[1] * b[2] - a[2] * b[1],
  a[2] * b[0] - a[0] * b[2],
  a[0] * b[1] - a[1] * b[0],
]
const norm = (v: Vec3): Vec3 => {
  const length = Math.hypot(...v) || 1
  return [v[0] / length, v[1] / length, v[2] / length]
}

function axisVector(axis: string): Vec3 {
  const sign = axis.startsWith('-') ? -1 : 1
  return axis.endsWith('X') ? [sign, 0, 0] : axis.endsWith('Z') ? [0, 0, sign] : [0, sign, 0]
}

/// Rodrigues, the same turn the backend makes to the camera direction.
function rotateAbout(v: Vec3, axis: Vec3, degrees: number): Vec3 {
  const t = (degrees * Math.PI) / 180
  const [c, s] = [Math.cos(t), Math.sin(t)]
  const k = cross(axis, v)
  const d = dot(axis, v)
  return [0, 1, 2].map((i) => v[i] * c + k[i] * s + axis[i] * d * (1 - c)) as Vec3
}

/// The shortest rotation taking `from` onto `to`, applied to `v`. Mirrors
/// `rotate_between` in services/renderer.rs — this is the camera standing up
/// with the model, and the gizmo is only honest if it stands up the same way.
function rotateBetween(v: Vec3, from: Vec3, to: Vec3): Vec3 {
  const axis = cross(from, to)
  const sin = Math.hypot(...axis)
  if (sin < 1e-9) {
    if (dot(from, to) > 0) return v
    const perpendicular: Vec3 = Math.abs(from[0]) < 0.9 ? [1, 0, 0] : [0, 1, 0]
    return rotateAbout(v, norm(cross(from, perpendicular)), 180)
  }
  return rotateAbout(v, norm(axis), (Math.atan2(sin, dot(from, to)) * 180) / Math.PI)
}

/// f3d's own default, and so the axis the shipped camera direction was aimed
/// with. The gizmo draws the world before any override, so this is where it
/// starts from — matching `was_up` in the backend's `args_for`.
const DEFAULT_UP: Vec3 = [0, 1, 0]

/// The three world axes as the current camera sees them, projected flat.
/// `depth` is how far each one points away from the viewer, so the ones going
/// into the screen can be drawn faint and underneath.
function project(up: string, turntable: number) {
  const upVector = axisVector(up)
  const stoodUp = rotateBetween(BASE_DIRECTION, DEFAULT_UP, upVector)
  const forward = norm(rotateAbout(stoodUp, upVector, turntable))
  // A camera looking straight down its own up axis has no sideways: nothing
  // sensible to draw, and a zero-length cross product to draw it with.
  let right = cross(forward, upVector)
  if (Math.hypot(...right) < 1e-6) right = [1, 0, 0]
  right = norm(right)
  const screenUp = cross(right, forward)

  return AXES.map((axis) => ({
    ...axis,
    x: dot(axis.vector, right),
    y: dot(axis.vector, screenUp),
    depth: dot(axis.vector, forward),
  }))
}

/// Which way the axes point in the picture this is about to make.
function AxisGizmo({ up, turntable }: { up: string; turntable: number }) {
  const size = 84
  const centre = size / 2
  const reach = 26
  // Back to front, so an axis pointing away is overdrawn by the ones in front.
  const axes = project(up, turntable).sort((a, b) => b.depth - a.depth)

  return (
    // Centred by a flex parent rather than `mx: auto` on the svg: an svg's own
    // box does not behave like a block's, and auto margins slide off it.
    <Box sx={{ display: 'flex', justifyContent: 'center' }}>
      <Box
        component="svg"
        viewBox={`0 0 ${size} ${size}`}
        sx={{ width: size, height: size, display: 'block' }}
        aria-label={`Axes as seen in the render: up is ${up}, turned ${turntable}°`}
      >
        <circle cx={centre} cy={centre} r={2} fill="currentColor" opacity={0.35} />
        {axes.map((axis) => {
          // Screen y grows downward; the projection's does not.
          const x = centre + axis.x * reach
          const y = centre - axis.y * reach
          const away = axis.depth > 0
          return (
            <g key={axis.name} opacity={away ? 0.4 : 1}>
              <line
                x1={centre}
                y1={centre}
                x2={x}
                y2={y}
                stroke={axis.colour}
                strokeWidth={2}
                strokeLinecap="round"
              />
              <text
                x={centre + axis.x * (reach + 9)}
                y={centre - axis.y * (reach + 9)}
                fill={axis.colour}
                fontSize={11}
                fontWeight={700}
                textAnchor="middle"
                dominantBaseline="central"
              >
                {axis.name}
              </text>
            </g>
          )
        })}
      </Box>
    </Box>
  )
}

/// "No orientation set", as one stable object: the seed of these controls is a
/// hook dependency, and a fresh `{}` every render would re-seed them forever.
const NO_OVERRIDES: RenderOverrides = {}

/// The panel itself — the gizmo, the axis picker, the turntable and the footer.
/// The per-image button and the whole-gallery one open this same thing; all that
/// differs between them is what an apply writes to, and what the footer says it
/// covers.
function OrientationControls({
  value,
  onChange,
  busy,
  covers,
  error,
}: {
  value: RenderOverrides
  onChange: (next: RenderOverrides) => void
  busy: boolean
  /** what this apply will re-render, for the footer */
  covers: string
  error: string | null
}) {
  const { axis, negative } = splitAxis(value.up)
  const turntable = value.turntable ?? 0

  const setAxis = (next: string) =>
    onChange({ ...value, up: next ? `${negative ? '-' : '+'}${next}` : null })
  const flip = () => onChange({ ...value, up: axis ? `${negative ? '+' : '-'}${axis}` : null })
  const turn = (delta: number) => onChange({ ...value, turntable: turntable + delta })

  return (
    <Stack spacing={1.5} sx={{ p: 2, width: 260 }}>
      {/* The axes as this render will see them. `up` falls back to f3d's
          own default, which is what an unset override leaves in force. */}
      <AxisGizmo up={value.up || '+Y'} turntable={turntable} />
      <Box>
        <Typography variant="caption" color="text.secondary">
          Which way is up
        </Typography>
        <Stack direction="row" spacing={1} sx={{ mt: 0.5, alignItems: 'center' }}>
          <ToggleButtonGroup
            exclusive
            size="small"
            value={axis}
            onChange={(_, next) => setAxis(next ?? '')}
            sx={{ flexGrow: 1 }}
          >
            {['X', 'Y', 'Z'].map((a) => (
              <ToggleButton key={a} value={a} sx={{ flexGrow: 1, py: 0.25 }}>
                {a}
              </ToggleButton>
            ))}
          </ToggleButtonGroup>
          <Tooltip title={axis ? `Stand it on its head (−${axis})` : 'Pick an axis first'}>
            {/* A disabled child cannot fire the events a Tooltip listens
                for, hence the span: without it the hint disappears exactly
                when it is most needed. */}
            <span>
              <ToggleButton
                size="small"
                value="flip"
                disabled={!axis}
                selected={negative}
                onChange={flip}
                sx={{ py: 0.25 }}
              >
                <SwapVertIcon sx={{ fontSize: 18 }} />
              </ToggleButton>
            </span>
          </Tooltip>
        </Stack>
      </Box>

      <Box>
        <Typography variant="caption" color="text.secondary">
          Turntable
        </Typography>
        <Stack direction="row" spacing={1} sx={{ mt: 0.5, alignItems: 'center' }}>
          <Tooltip title={`Turn ${STEP}° left`}>
            <IconButton size="small" onClick={() => turn(-STEP)}>
              <RotateLeftIcon />
            </IconButton>
          </Tooltip>
          <Typography variant="body2" sx={{ flexGrow: 1, textAlign: 'center' }}>
            {turntable}°
          </Typography>
          <Tooltip title={`Turn ${STEP}° right`}>
            <IconButton size="small" onClick={() => turn(STEP)}>
              <RotateRightIcon />
            </IconButton>
          </Tooltip>
        </Stack>
      </Box>

      <Divider />
      <Stack direction="row" spacing={1} sx={{ alignItems: 'center' }}>
        <Typography variant="caption" color="text.secondary" sx={{ flexGrow: 1 }}>
          {busy ? 'Rendering…' : covers}
        </Typography>
        <Button
          size="small"
          disabled={!value.up && !turntable}
          onClick={() => onChange(NO_OVERRIDES)}
        >
          Reset
        </Button>
      </Stack>
      {error && (
        <Typography variant="caption" color="error">
          {error}
        </Typography>
      )}
    </Stack>
  )
}

/// What the controls show, and the deferred apply behind them.
///
/// Changes are applied on a short delay rather than per click: stepping the
/// turntable round three notches should cost one render, not three. The timer is
/// cleared on unmount — a popover closed mid-debounce has said what it wants, but
/// a component that is gone has no business refetching for it.
///
/// `seed` is what the controls open on, re-read whenever it changes while the
/// popover is shut (so a picture swapped underneath brings its own orientation).
/// It has to be referentially stable — memoise it, or take it straight off a
/// query — since a new object every render would re-seed on every render.
function useOrientation(
  seed: RenderOverrides,
  open: boolean,
  run: (next: RenderOverrides) => Promise<void>,
) {
  const [pending, setPending] = useState<RenderOverrides>(seed)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)
  useEffect(() => {
    if (!open) setPending(seed)
  }, [seed, open])

  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)
  useEffect(() => {
    return () => {
      if (timer.current) clearTimeout(timer.current)
    }
  }, [])
  // Read at fire time, not capture time: the caller passes a fresh closure every
  // render, and the one that should run is the latest.
  const runRef = useRef(run)
  runRef.current = run

  const apply = (next: RenderOverrides) => {
    setPending(next)
    setError(null)
    if (timer.current) clearTimeout(timer.current)
    timer.current = setTimeout(async () => {
      setBusy(true)
      try {
        await runRef.current(next)
      } catch (e) {
        setError(e instanceof Error ? e.message : 'Re-render failed.')
      } finally {
        setBusy(false)
      }
    }, 500)
  }

  return { pending, apply, busy, error }
}

/// Correct a render that came out on its side, or facing the wrong way.
///
/// f3d assumes +Y up; print files are authored by whoever made them, so a Z-up
/// STL renders lying down. The axis is a property of the *file*, and that is
/// where this writes it (`POST /api/images/{id}/rerender`) — so the fix outlives
/// the picture it was made on, and a bulk re-render inherits it.
///
/// The image row is rewritten in place, so the id this is anchored to stays valid
/// and the picture simply changes underneath.
export default function RenderOrientation({
  image,
  onRendered,
  edge = false,
}: {
  image: Reorientable
  /** called once a render has finished, to refetch whatever holds the picture */
  onRendered: () => void
  /** sit on top of the image (the preview pane) rather than beside it */
  edge?: boolean
}) {
  const [anchor, setAnchor] = useState<HTMLElement | null>(null)
  const { pending, apply, busy, error } = useOrientation(
    image.render_overrides ?? NO_OVERRIDES,
    !!anchor,
    async (next) => {
      const { job_id } = await api.rerenderImage(image.id, next)
      await waitForJob(job_id)
      onRendered()
    },
  )

  return (
    <>
      <Tooltip title="Fix which way up this renders">
        <IconButton
          size="small"
          onClick={(e) => setAnchor(e.currentTarget)}
          sx={
            edge
              ? {
                  position: 'absolute',
                  top: 8,
                  right: 8,
                  bgcolor: 'background.paper',
                  boxShadow: 1,
                  '&:hover': { bgcolor: 'background.paper' },
                }
              : { p: 0.25, bgcolor: 'background.paper' }
          }
        >
          {busy ? (
            <CircularProgress size={edge ? 20 : 16} />
          ) : (
            <ThreeDRotationIcon sx={{ fontSize: edge ? 20 : 16 }} />
          )}
        </IconButton>
      </Tooltip>

      <Popover
        open={!!anchor}
        anchorEl={anchor}
        onClose={() => setAnchor(null)}
        anchorOrigin={{ vertical: 'bottom', horizontal: 'right' }}
        transformOrigin={{ vertical: 'top', horizontal: 'right' }}
      >
        <OrientationControls
          value={pending}
          onChange={apply}
          busy={busy}
          covers="Applies to every render of this file."
          error={error}
        />
      </Popover>
    </>
  )
}

/// One orientation for everything a bundle shows.
///
/// A bundle is one purchase, from files that were authored together, so they are
/// wrong together: a Z-up publisher lays every one of them on its side, and the
/// per-image popover then means the same answer typed once per model. This is the
/// control above with the whole crate as its subject — chiefly the *member
/// models' previews*, which is what a bundle page is a wall of, plus any render
/// in the bundle's own gallery. The axis still lands on each source file, so the
/// fix outlives these pictures.
///
/// What it covers is counted by the server (`bundle.orientable`), not from the
/// gallery below: the members' previews are not in this page's `images`, and a
/// bundle rarely owns files of its own. Nothing is drawn unless there are at
/// least two pictures to fix, since one of them is what a per-image button is
/// for.
export function BundleRenderOrientation({
  bundleId,
  orientable,
  onRendered,
}: {
  bundleId: string
  /** what the fix would cover, straight off the bundle query so the seed it
      carries stays referentially stable between renders */
  orientable: OrientableRenders
  onRendered: () => void
}) {
  const [anchor, setAnchor] = useState<HTMLElement | null>(null)
  const { pending, apply, busy, error } = useOrientation(
    orientable.shared_overrides ?? NO_OVERRIDES,
    !!anchor,
    async (next) => {
      const { job_ids } = await api.rerenderBundleImages(bundleId, next)
      // Every picture, not just the first: the page is only worth refetching once
      // the last of them has been redrawn.
      await Promise.all(job_ids.map((id) => waitForJob(id)))
      onRendered()
    },
  )

  if (orientable.count < 2) return null

  return (
    <>
      {/* `describeChild`, unlike the icon-only buttons above: a Tooltip's title
          otherwise becomes the child's `aria-label` and *replaces* its visible
          text as the accessible name, so this button would announce as the hint
          and answer to nothing a user can see. */}
      <Tooltip
        describeChild
        title="Fix which way up this bundle renders — every model’s preview at once"
      >
        <Button
          size="small"
          startIcon={
            busy ? <CircularProgress size={16} /> : <ThreeDRotationIcon sx={{ fontSize: 18 }} />
          }
          onClick={(e) => setAnchor(e.currentTarget)}
        >
          Orient all {orientable.count}
        </Button>
      </Tooltip>

      <Popover
        open={!!anchor}
        anchorEl={anchor}
        onClose={() => setAnchor(null)}
        anchorOrigin={{ vertical: 'bottom', horizontal: 'left' }}
        transformOrigin={{ vertical: 'top', horizontal: 'left' }}
      >
        <OrientationControls
          value={pending}
          onChange={apply}
          busy={busy}
          covers={`Applies to all ${orientable.count} pictures this bundle shows, the member models’ previews included.`}
          error={error}
        />
      </Popover>
    </>
  )
}
