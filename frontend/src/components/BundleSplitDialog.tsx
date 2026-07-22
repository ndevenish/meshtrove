import { useEffect, useState } from 'react'
import {
  Alert,
  Box,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  TextField,
  Typography,
} from '@mui/material'

import { api, type BundleDetail, type ModelSummary } from '../api'

/// Name and confirm a split: the picked members leave this bundle for a new one.
///
/// The picking happens on the page behind (checkboxes on the member cards), so
/// all this asks for is the name — and says plainly what travels with them,
/// since "split" could as easily have meant a copy.
export default function BundleSplitDialog({
  open,
  onClose,
  bundle,
  models,
  onSplit,
}: {
  open: boolean
  onClose: () => void
  bundle: BundleDetail
  /** The selected members, in the order they are shown. */
  models: ModelSummary[]
  onSplit: (created: BundleDetail) => void
}) {
  const [name, setName] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  // Each split is its own thing: reopening starts from a blank name rather than
  // the last one, which would quietly name two different halves the same.
  useEffect(() => {
    if (open) {
      setName('')
      setError('')
    }
  }, [open])

  const confirm = async () => {
    if (!name.trim()) return setError('Give the new bundle a name')
    setBusy(true)
    setError('')
    try {
      onSplit(
        await api.splitBundle(
          bundle.id,
          name.trim(),
          models.map((m) => m.id),
        ),
      )
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onClose={busy ? undefined : onClose} maxWidth="sm" fullWidth>
      <DialogTitle>Split into a new bundle</DialogTitle>
      <DialogContent>
        {error && (
          <Alert severity="error" sx={{ mb: 2 }}>
            {error}
          </Alert>
        )}
        <Typography sx={{ mb: 2 }}>
          Move {models.length} model{models.length === 1 ? '' : 's'} out of{' '}
          <strong>{bundle.name}</strong> into a bundle of their own.
        </Typography>
        <TextField
          autoFocus
          fullWidth
          label="New bundle name"
          value={name}
          disabled={busy}
          onChange={(e) => setName(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === 'Enter') void confirm()
          }}
          sx={{ mb: 2 }}
        />
        <Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>
          The new bundle inherits this one’s creator, source, tags and custom fields — the half
          being lifted out was bought on the same terms as the half staying behind. Its description
          and pictures stay here; the models themselves are untouched.
        </Typography>
        <Box
          sx={{
            maxHeight: 180,
            overflowY: 'auto',
            border: (t) => `1px solid ${t.palette.divider}`,
            borderRadius: 1,
            p: 1,
          }}
        >
          {models.map((model) => (
            <Typography key={model.id} variant="body2">
              {model.name}
            </Typography>
          ))}
        </Box>
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose} disabled={busy}>
          Cancel
        </Button>
        <Button variant="contained" onClick={confirm} disabled={busy}>
          Split out {models.length} model{models.length === 1 ? '' : 's'}
        </Button>
      </DialogActions>
    </Dialog>
  )
}
