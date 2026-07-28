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

import { api, waitForJob, type ImageRecord, type RenderOverrides } from '../api'

/// The turntable turns in eighths. Finer than that is a slider, and a slider is
/// a lot of renders to find the angle you could have clicked to.
const STEP = 45

/// Whether these controls apply at all: only a *rendered* picture can be
/// rendered again, and only while the file it came from is still there
/// (`source_file_id` is cleared when the model file goes).
export function canReorient(image: ImageRecord): boolean {
  return image.kind === 'rendered' && !!image.source_file_id
}

/// Split `+Z` into the axis and its sign, so the UI can offer three axes and a
/// flip rather than six near-identical buttons.
function splitAxis(up: string | null | undefined): { axis: string; negative: boolean } {
  if (!up) return { axis: '', negative: false }
  return { axis: up.slice(1), negative: up.startsWith('-') }
}

/// Correct a render that came out on its side, or facing the wrong way.
///
/// f3d assumes +Y up; print files are authored by whoever made them, so a Z-up
/// STL renders lying down. The axis is a property of the *file*, and that is
/// where this writes it (`POST /api/images/{id}/rerender`) — so the fix outlives
/// the picture it was made on, and a bulk re-render inherits it.
///
/// Changes are applied on a short delay rather than per click: stepping the
/// turntable round three notches should cost one render, not three. The image
/// row is rewritten in place, so the id this is anchored to stays valid and the
/// picture simply changes underneath.
export default function RenderOrientation({
  image,
  onRendered,
  edge = false,
}: {
  image: ImageRecord
  /** called once a render has finished, to refetch whatever holds the picture */
  onRendered: () => void
  /** sit on top of the image (the preview pane) rather than beside it */
  edge?: boolean
}) {
  const [anchor, setAnchor] = useState<HTMLElement | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState<string | null>(null)

  // What the controls show. Seeded from the render's own orientation so the
  // popover opens where the last one left off, and re-seeded if the picture is
  // swapped for another while the popover is shut.
  const [pending, setPending] = useState<RenderOverrides>(image.render_overrides ?? {})
  useEffect(() => {
    if (!anchor) setPending(image.render_overrides ?? {})
  }, [image.render_overrides, anchor])

  // The apply is deferred so a burst of clicks queues one render. The timer is
  // cleared on unmount: a popover closed mid-debounce has said what it wants,
  // but a component that is gone has no business refetching for it.
  const timer = useRef<ReturnType<typeof setTimeout> | null>(null)
  useEffect(() => {
    return () => {
      if (timer.current) clearTimeout(timer.current)
    }
  }, [])

  const apply = (next: RenderOverrides) => {
    setPending(next)
    setError(null)
    if (timer.current) clearTimeout(timer.current)
    timer.current = setTimeout(async () => {
      setBusy(true)
      try {
        const { job_id } = await api.rerenderImage(image.id, next)
        await waitForJob(job_id)
        onRendered()
      } catch (e) {
        setError(e instanceof Error ? e.message : 'Re-render failed.')
      } finally {
        setBusy(false)
      }
    }, 500)
  }

  const { axis, negative } = splitAxis(pending.up)
  const turntable = pending.turntable ?? 0

  const setAxis = (next: string) =>
    apply({ ...pending, up: next ? `${negative ? '-' : '+'}${next}` : null })
  const flip = () => apply({ ...pending, up: axis ? `${negative ? '+' : '-'}${axis}` : null })
  const turn = (delta: number) => apply({ ...pending, turntable: turntable + delta })

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
        <Stack spacing={1.5} sx={{ p: 2, width: 260 }}>
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
              {busy ? 'Rendering…' : 'Applies to every render of this file.'}
            </Typography>
            <Button size="small" disabled={!pending.up && !turntable} onClick={() => apply({})}>
              Reset
            </Button>
          </Stack>
          {error && (
            <Typography variant="caption" color="error">
              {error}
            </Typography>
          )}
        </Stack>
      </Popover>
    </>
  )
}
