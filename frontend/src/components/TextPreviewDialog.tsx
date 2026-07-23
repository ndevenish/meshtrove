import { useEffect, useState } from 'react'
import {
  Dialog,
  DialogTitle,
  DialogContent,
  Box,
  Typography,
  IconButton,
  Alert,
  CircularProgress,
} from '@mui/material'
import CloseIcon from '@mui/icons-material/Close'

import { downloadUrl } from '../api'

// A big text file is rare in a fileset, but a stray log or dump shouldn't lock
// the tab up: read only the first slice and say so. The download endpoint
// streams, so a Range request costs nothing when the server honours it, and a
// client-side slice covers it when it doesn't.
const TEXT_PREVIEW_LIMIT = 512 * 1024

/// A plain reader for a text file sitting inside a fileset — a readme, a licence,
/// a slicer's print settings. Like the STL and image previews, this is the only
/// way to look at one short of downloading it.
///
/// The bytes come straight from the file's download endpoint (same-origin, so
/// the session cookie rides along — staged import files stay gated to editors).
/// The `attachment` disposition that endpoint sets doesn't stop `fetch` from
/// reading the body.
export default function TextPreviewDialog({
  open,
  fileId,
  filename,
  size,
  onClose,
}: {
  open: boolean
  fileId: string | null
  filename: string
  /** File size in bytes; drives whether we only fetch (and flag) a first slice. */
  size: number
  onClose: () => void
}) {
  const [text, setText] = useState('')
  const [truncated, setTruncated] = useState(false)
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState(false)

  useEffect(() => {
    if (!open || !fileId) return
    setLoading(true)
    setError(false)
    setText('')
    setTruncated(false)

    const controller = new AbortController()
    const large = size > TEXT_PREVIEW_LIMIT
    fetch(downloadUrl(fileId), {
      signal: controller.signal,
      headers: large ? { Range: `bytes=0-${TEXT_PREVIEW_LIMIT - 1}` } : {},
    })
      .then((res) => {
        if (!res.ok && res.status !== 206) throw new Error(`HTTP ${res.status}`)
        return res.text()
      })
      .then((body) => {
        // Honoured Range → already a slice; ignored it → slice here. Either way
        // the file is longer than what's shown whenever it's over the limit.
        setText(large ? body.slice(0, TEXT_PREVIEW_LIMIT) : body)
        setTruncated(large)
        setLoading(false)
      })
      .catch((e) => {
        if (controller.signal.aborted) return
        setError(true)
        setLoading(false)
        console.error(e)
      })

    return () => controller.abort()
  }, [open, fileId, size])

  return (
    <Dialog open={open} onClose={onClose} maxWidth="md" fullWidth>
      <DialogTitle sx={{ pr: 6 }}>
        <Typography component="span" noWrap sx={{ display: 'block', fontWeight: 600 }}>
          {filename}
        </Typography>
        <IconButton
          onClick={onClose}
          sx={{ position: 'absolute', right: 8, top: 8 }}
          aria-label="Close"
        >
          <CloseIcon />
        </IconButton>
      </DialogTitle>
      <DialogContent>
        {error ? (
          <Alert severity="error">Could not load this file.</Alert>
        ) : loading ? (
          <Box
            sx={{
              minHeight: { xs: 240, sm: 320 },
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
            }}
          >
            <CircularProgress aria-label="Loading file" />
          </Box>
        ) : (
          <>
            {truncated && (
              <Alert severity="info" sx={{ mb: 1 }}>
                Large file — showing the first {Math.round(TEXT_PREVIEW_LIMIT / 1024)} KB. Download
                it to read the rest.
              </Alert>
            )}
            <Box
              component="pre"
              sx={{
                m: 0,
                p: 1.5,
                maxHeight: { xs: 320, sm: 560 },
                overflow: 'auto',
                borderRadius: 1,
                bgcolor: 'background.default',
                fontFamily: 'monospace',
                fontSize: 13,
                lineHeight: 1.5,
                whiteSpace: 'pre-wrap',
                wordBreak: 'break-word',
              }}
            >
              {text}
            </Box>
          </>
        )}
      </DialogContent>
    </Dialog>
  )
}
