import { useState } from 'react'
import { Box, Typography, LinearProgress, alpha } from '@mui/material'
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
  progress,
  onDrop,
}: {
  label: string
  hint?: string
  accept?: string
  multiple?: boolean
  busy?: boolean
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
        textAlign: 'center',
        borderRadius: 2,
        border: '2px dashed',
        borderColor: over ? 'primary.main' : alpha(theme.palette.text.primary, 0.25),
        backgroundColor: over ? alpha(theme.palette.primary.main, 0.06) : 'transparent',
        px: 3,
        py: 3,
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
      <UploadFileIcon sx={{ fontSize: 32, opacity: 0.6 }} />
      <Typography sx={{ fontWeight: 600, mt: 0.5 }}>{label}</Typography>
      {hint && (
        <Typography variant="body2" color="text.secondary">
          {hint}
        </Typography>
      )}
      {busy &&
        (progress === undefined ? (
          <LinearProgress sx={{ mt: 1.5 }} />
        ) : (
          <LinearProgress variant="determinate" value={progress} sx={{ mt: 1.5 }} />
        ))}
    </Box>
  )
}
