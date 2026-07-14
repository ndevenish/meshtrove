import { useState } from 'react'
import { Box, Typography, LinearProgress, alpha } from '@mui/material'
import UploadFileIcon from '@mui/icons-material/UploadFile'

/// A dashed drop target with a hidden file-input fallback (click to browse).
export default function Dropzone({
  label,
  hint,
  accept,
  multiple = false,
  busy = false,
  progress,
  onFiles,
}: {
  label: string
  hint?: string
  accept?: string
  multiple?: boolean
  busy?: boolean
  /** 0-100 for a determinate bar; omit for indeterminate */
  progress?: number
  onFiles: (files: File[]) => void
}) {
  const [over, setOver] = useState(false)

  const take = (list: FileList | null) => {
    if (list && list.length) onFiles(Array.from(list))
  }

  return (
    <Box
      component="label"
      onDragOver={(e) => {
        e.preventDefault()
        setOver(true)
      }}
      onDragLeave={() => setOver(false)}
      onDrop={(e) => {
        e.preventDefault()
        setOver(false)
        take(e.dataTransfer.files)
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
          take(e.target.files)
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
