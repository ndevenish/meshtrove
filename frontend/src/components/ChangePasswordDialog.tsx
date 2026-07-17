import { useState } from 'react'
import {
  Alert,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Stack,
  TextField,
} from '@mui/material'

import { api } from '../api'

/// Self-service password change: confirm the current password and set a new one.
/// The session cookie is unaffected, so the user stays logged in.
export default function ChangePasswordDialog({
  open,
  onClose,
}: {
  open: boolean
  onClose: () => void
}) {
  const [current, setCurrent] = useState('')
  const [next, setNext] = useState('')
  const [confirm, setConfirm] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  // Reset each time it opens.
  const [wasOpen, setWasOpen] = useState(false)
  if (open && !wasOpen) {
    setWasOpen(true)
    setCurrent('')
    setNext('')
    setConfirm('')
    setError('')
  }
  if (!open && wasOpen) setWasOpen(false)

  const tooShort = next.length > 0 && next.length < 8
  const mismatch = confirm.length > 0 && next !== confirm
  const canSubmit = !!current && next.length >= 8 && next === confirm && !busy

  const submit = async () => {
    setBusy(true)
    setError('')
    try {
      await api.changePassword(current, next)
      onClose()
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onClose={onClose} maxWidth="xs" fullWidth>
      <DialogTitle>Change password</DialogTitle>
      <DialogContent>
        <Stack spacing={2} sx={{ mt: 1 }}>
          {error && <Alert severity="error">{error}</Alert>}
          <TextField
            label="Current password"
            type="password"
            autoFocus
            autoComplete="current-password"
            value={current}
            onChange={(e) => setCurrent(e.target.value)}
          />
          <TextField
            label="New password"
            type="password"
            autoComplete="new-password"
            value={next}
            onChange={(e) => setNext(e.target.value)}
            error={tooShort}
            helperText={tooShort ? 'At least 8 characters.' : ' '}
          />
          <TextField
            label="Confirm new password"
            type="password"
            autoComplete="new-password"
            value={confirm}
            onChange={(e) => setConfirm(e.target.value)}
            error={mismatch}
            helperText={mismatch ? "Passwords don't match." : ' '}
            onKeyDown={(e) => {
              if (e.key === 'Enter' && canSubmit) void submit()
            }}
          />
        </Stack>
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="contained" onClick={submit} disabled={!canSubmit}>
          Change password
        </Button>
      </DialogActions>
    </Dialog>
  )
}
