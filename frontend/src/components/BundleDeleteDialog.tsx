import { useEffect, useState } from 'react'
import {
  Dialog,
  DialogTitle,
  DialogContent,
  DialogActions,
  Button,
  Typography,
  FormControlLabel,
  Checkbox,
  Alert,
  Box,
} from '@mui/material'

import { api, type BundleDetail } from '../api'

/// Confirm deleting a bundle, and choose what happens to its member models.
/// The default (and the plain DELETE's behaviour) only unlinks the members — they
/// are standalone models that may live in other bundles. Ticking the box deletes
/// them too, which the backend does all-or-nothing over the ones you may edit.
export default function BundleDeleteDialog({
  open,
  onClose,
  bundle,
  onDeleted,
}: {
  open: boolean
  onClose: () => void
  bundle: BundleDetail
  onDeleted: () => void
}) {
  const [deleteModels, setDeleteModels] = useState(false)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  // Reopen resets the choice: deleting the models is opt-in every time, never a
  // sticky setting from a previous delete.
  useEffect(() => {
    if (open) {
      setDeleteModels(false)
      setError('')
    }
  }, [open])

  const memberCount = bundle.models.length

  const confirm = async () => {
    setBusy(true)
    setError('')
    try {
      await api.deleteBundle(bundle.id, deleteModels)
      onDeleted()
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onClose={busy ? undefined : onClose} maxWidth="sm" fullWidth>
      <DialogTitle>Delete bundle?</DialogTitle>
      <DialogContent>
        {error && (
          <Alert severity="error" sx={{ mb: 2 }}>
            {error}
          </Alert>
        )}
        <Typography sx={{ mb: memberCount ? 2 : 0 }}>
          Delete <strong>{bundle.name}</strong>? This can’t be undone.
        </Typography>
        {memberCount > 0 && (
          <>
            <FormControlLabel
              control={
                <Checkbox
                  checked={deleteModels}
                  onChange={(e) => setDeleteModels(e.target.checked)}
                />
              }
              label={`Also delete the ${memberCount} member model${memberCount === 1 ? '' : 's'}`}
            />
            <Box sx={{ pl: 4 }}>
              <Typography variant="caption" color="text.secondary">
                {deleteModels
                  ? 'The member models and their files are deleted too. A model that also belongs to another bundle is removed from it as well.'
                  : 'The member models are kept — they only leave this bundle. Delete the bundle alone.'}
              </Typography>
            </Box>
          </>
        )}
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose} disabled={busy}>
          Cancel
        </Button>
        <Button color="error" variant="contained" onClick={confirm} disabled={busy}>
          {deleteModels && memberCount > 0
            ? `Delete bundle & ${memberCount} model${memberCount === 1 ? '' : 's'}`
            : 'Delete bundle'}
        </Button>
      </DialogActions>
    </Dialog>
  )
}
