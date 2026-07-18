import { useEffect, useState } from 'react'
import {
  Dialog,
  DialogTitle,
  DialogContent,
  DialogActions,
  Button,
  Typography,
  RadioGroup,
  FormControlLabel,
  Radio,
  Alert,
  Box,
} from '@mui/material'

import { api, type BundleDetail, type BundleMemberDisposition } from '../api'

/// Confirm deleting a bundle, and choose what happens to its member models. A
/// member is a standalone model that may also live in other bundles, so the
/// choice is three-way: keep them all (just unlink), delete only the ones unique
/// to this bundle, or delete every member. The backend does whichever
/// all-or-nothing over the members the caller may edit.
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
  const [members, setMembers] = useState<BundleMemberDisposition>('keep')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  // Reopen resets to the safe default: deleting models is chosen fresh each time,
  // never carried over from a previous delete.
  useEffect(() => {
    if (open) {
      setMembers('keep')
      setError('')
    }
  }, [open])

  const memberCount = bundle.models.length

  const confirm = async () => {
    setBusy(true)
    setError('')
    try {
      await api.deleteBundle(bundle.id, members)
      onDeleted()
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
      setBusy(false)
    }
  }

  const buttonLabel =
    members === 'keep' || memberCount === 0
      ? 'Delete bundle'
      : members === 'delete'
        ? `Delete bundle & ${memberCount} model${memberCount === 1 ? '' : 's'}`
        : 'Delete bundle & unique models'

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
          <RadioGroup
            value={members}
            onChange={(e) => setMembers(e.target.value as BundleMemberDisposition)}
          >
            <Option
              value="keep"
              title={`Keep all ${memberCount} model${memberCount === 1 ? '' : 's'}`}
              caption="The members stay in your library; they only leave this bundle."
            />
            <Option
              value="delete_exclusive"
              title="Delete models, but keep any in another bundle"
              caption="Members unique to this bundle are deleted; any that also belong to another bundle stay (they just leave this one)."
            />
            <Option
              value="delete"
              title="Delete all member models"
              caption="Every member is deleted, including any that also belong to another bundle."
            />
          </RadioGroup>
        )}
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose} disabled={busy}>
          Cancel
        </Button>
        <Button color="error" variant="contained" onClick={confirm} disabled={busy}>
          {buttonLabel}
        </Button>
      </DialogActions>
    </Dialog>
  )
}

/// A radio row with a title and an explanatory caption beneath it.
function Option({ value, title, caption }: { value: string; title: string; caption: string }) {
  return (
    <FormControlLabel
      value={value}
      control={<Radio sx={{ alignSelf: 'flex-start', pt: 0.5 }} />}
      sx={{ alignItems: 'flex-start', mb: 1 }}
      label={
        <Box sx={{ py: 0.5 }}>
          <Typography variant="body2">{title}</Typography>
          <Typography variant="caption" color="text.secondary">
            {caption}
          </Typography>
        </Box>
      }
    />
  )
}
