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

/// A plain viewer for an image sitting inside a fileset — a render, a painted
/// reference, a card back. Unlike a model's gallery images these were never
/// promoted to the `images` table, so the only way to see one used to be to
/// download it.
///
/// The bytes come straight from the file's download endpoint (same-origin, so
/// the session cookie rides along — staged import files stay gated to editors).
/// The `attachment` disposition that endpoint sets doesn't stop an `<img>` from
/// rendering it.
export default function ImagePreviewDialog({
  open,
  fileId,
  filename,
  onClose,
}: {
  open: boolean
  fileId: string | null
  filename: string
  onClose: () => void
}) {
  const [loading, setLoading] = useState(true)
  const [error, setError] = useState(false)
  useEffect(() => {
    setLoading(true)
    setError(false)
  }, [fileId])

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
          <Alert severity="error">Could not load this image.</Alert>
        ) : (
          <Box
            sx={{
              position: 'relative',
              width: '100%',
              minHeight: { xs: 240, sm: 320 },
              borderRadius: 1,
              overflow: 'hidden',
              bgcolor: 'background.default',
              display: 'flex',
              alignItems: 'center',
              justifyContent: 'center',
            }}
          >
            {loading && <CircularProgress aria-label="Loading image" />}
            {fileId && (
              <Box
                component="img"
                src={downloadUrl(fileId)}
                alt={filename}
                onLoad={() => setLoading(false)}
                onError={() => {
                  setLoading(false)
                  setError(true)
                }}
                sx={{
                  maxWidth: '100%',
                  maxHeight: { xs: 320, sm: 560 },
                  objectFit: 'contain',
                  display: loading ? 'none' : 'block',
                }}
              />
            )}
          </Box>
        )}
      </DialogContent>
    </Dialog>
  )
}
