import { useEffect, useState } from 'react'
import {
  Dialog,
  DialogTitle,
  DialogContent,
  DialogActions,
  Button,
  Typography,
  Alert,
} from '@mui/material'

import { api } from '../api'

/// Confirm deleting a model. Unlike a bundle it has no members to reason about,
/// so this is a plain are-you-sure: the model, its variants, files and images
/// all go, and it leaves any bundles it belonged to.
export default function ModelDeleteDialog({
  open,
  onClose,
  model,
  onDeleted,
}: {
  open: boolean
  onClose: () => void
  model: { id: string; name: string }
  onDeleted: () => void
}) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  useEffect(() => {
    if (open) setError('')
  }, [open])

  const confirm = async () => {
    setBusy(true)
    setError('')
    try {
      await api.deleteModel(model.id)
      onDeleted()
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onClose={busy ? undefined : onClose} maxWidth="sm" fullWidth>
      <DialogTitle>Delete model?</DialogTitle>
      <DialogContent>
        {error && (
          <Alert severity="error" sx={{ mb: 2 }}>
            {error}
          </Alert>
        )}
        <Typography>
          Delete <strong>{model.name}</strong>? Its variants, files and images go with it, and it
          leaves any bundles it belonged to. This can’t be undone.
        </Typography>
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose} disabled={busy}>
          Cancel
        </Button>
        <Button color="error" variant="contained" onClick={confirm} disabled={busy}>
          Delete model
        </Button>
      </DialogActions>
    </Dialog>
  )
}
