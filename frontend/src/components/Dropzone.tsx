import { useState } from 'react'
import { Box, Stack, Typography, LinearProgress, alpha } from '@mui/material'
import UploadFileIcon from '@mui/icons-material/UploadFile'

import { readDrop, readFileList, type Drop } from '../upload'

/// A dashed drop target with a hidden file-input fallback (click to browse).
/// Accepts a folder as readily as a file: the drop is resolved through
/// `readDrop`, which walks a directory into its files rather than handing the
/// directory itself on as if it were one.
export default function Dropzone({
  label,
  hint,
  accept,
  multiple = false,
  busy = false,
  dense = false,
  progress,
  onDrop,
}: {
  label: string
  hint?: string
  accept?: string
  multiple?: boolean
  busy?: boolean
  /** A one-line-tall strip — icon beside the text rather than above it — for a
      target that is a field on a form rather than the point of the page. Two
      lines with a hint, and never more: the label and hint each stay on one, and
      the progress bar rides the bottom border instead of adding a row. */
  dense?: boolean
  /** 0-100 for a determinate bar; omit for indeterminate */
  progress?: number
  onDrop: (drop: Drop) => void
}) {
  const [over, setOver] = useState(false)

  return (
    <Box
      component="label"
      // Marks this as an explicit file target: an image dropped in here is a
      // file, not a picture, so the page-wide image drop (see imageGestures.ts)
      // leaves anything landing inside alone.
      data-file-drop=""
      onDragOver={(e) => {
        e.preventDefault()
        setOver(true)
      }}
      onDragLeave={() => setOver(false)}
      onDrop={(e) => {
        e.preventDefault()
        setOver(false)
        void readDrop(e.dataTransfer).then((drop) => {
          if (drop.files.length) onDrop(drop)
        })
      }}
      sx={(theme) => ({
        display: 'block',
        cursor: 'pointer',
        textAlign: dense ? 'left' : 'center',
        borderRadius: 2,
        border: '2px dashed',
        borderColor: over ? 'primary.main' : alpha(theme.palette.text.primary, 0.25),
        backgroundColor: over ? alpha(theme.palette.primary.main, 0.06) : 'transparent',
        px: dense ? 1.5 : 3,
        py: dense ? 1 : 3,
        ...(dense && { position: 'relative', overflow: 'hidden' }),
        transition: 'border-color 120ms, background-color 120ms',
      })}
    >
      <input
        hidden
        type="file"
        accept={accept}
        multiple={multiple}
        onChange={(e) => {
          if (e.target.files?.length) onDrop(readFileList(e.target.files))
          e.target.value = ''
        }}
      />
      {dense ? (
        <Stack direction="row" spacing={1.5} sx={{ alignItems: 'center' }}>
          <UploadFileIcon sx={{ fontSize: 24, opacity: 0.6, flexShrink: 0 }} />
          {/* minWidth: 0 so a long filename truncates rather than stretching the
              strip past its column. */}
          <Box sx={{ minWidth: 0 }}>
            <Typography noWrap sx={{ fontWeight: 600 }}>
              {label}
            </Typography>
            {hint && (
              <Typography noWrap variant="body2" color="text.secondary">
                {hint}
              </Typography>
            )}
          </Box>
        </Stack>
      ) : (
        <>
          <UploadFileIcon sx={{ fontSize: 32, opacity: 0.6 }} />
          <Typography sx={{ fontWeight: 600, mt: 0.5 }}>{label}</Typography>
          {hint && (
            <Typography variant="body2" color="text.secondary">
              {hint}
            </Typography>
          )}
        </>
      )}
      {busy &&
        (dense ? (
          // Along the bottom edge, out of the flow: a dense strip has a height to
          // keep, and a bar that pushes it taller mid-upload makes the form jump.
          <LinearProgress
            variant={progress === undefined ? 'indeterminate' : 'determinate'}
            value={progress}
            sx={{ position: 'absolute', bottom: 0, left: 0, right: 0 }}
          />
        ) : progress === undefined ? (
          <LinearProgress sx={{ mt: 1.5 }} />
        ) : (
          <LinearProgress variant="determinate" value={progress} sx={{ mt: 1.5 }} />
        ))}
    </Box>
  )
}
