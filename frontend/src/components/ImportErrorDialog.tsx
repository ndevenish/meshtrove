import {
  Alert,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  DialogContentText,
  DialogTitle,
} from '@mui/material'

/// A failed import is a modal, not a toast: an upload can run for minutes, by
/// which time a snackbar would have come and gone unseen — leaving the user
/// staring at a drop target that silently swallowed their files.
export default function ImportErrorDialog({
  error,
  onClose,
}: {
  /** Empty string when there is nothing to report. */
  error: string
  onClose: () => void
}) {
  return (
    <Dialog open={!!error} onClose={onClose} maxWidth="sm" fullWidth>
      <DialogTitle>Import failed</DialogTitle>
      <DialogContent>
        <Alert severity="error" sx={{ mb: 2 }}>
          {error}
        </Alert>
        <DialogContentText variant="body2">
          Nothing was imported — the staged import was discarded. You can drop the files again once
          the problem is fixed.
        </DialogContentText>
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose}>Close</Button>
      </DialogActions>
    </Dialog>
  )
}
