import { useState } from 'react'
import {
  Alert,
  Button,
  Dialog,
  DialogActions,
  DialogContent,
  DialogContentText,
  DialogTitle,
  IconButton,
  MenuItem,
  Paper,
  Select,
  Stack,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableRow,
  TextField,
  Tooltip,
  Typography,
} from '@mui/material'
import KeyIcon from '@mui/icons-material/Key'
import DeleteIcon from '@mui/icons-material/Delete'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'

import { api, type Role, type UserAccount } from '../api'
import { useAuth } from '../main'

const ROLES: Role[] = ['admin', 'editor', 'viewer']

/// Admin-only: list accounts, change their roles, reset their passwords, and
/// delete them. Your own row is locked — the backend refuses a self-role-change
/// or self-delete so an admin can't lock itself out of its own settings.
export default function UsersPanel() {
  const { user } = useAuth()
  const queryClient = useQueryClient()
  const [error, setError] = useState('')
  const [resetting, setResetting] = useState<UserAccount | null>(null)
  const [deleting, setDeleting] = useState<UserAccount | null>(null)

  const { data: users } = useQuery({
    queryKey: ['users'],
    queryFn: () => api.users(),
    enabled: user?.role === 'admin',
  })

  const setRole = useMutation({
    mutationFn: ({ id, role }: { id: string; role: Role }) => api.setUserRole(id, role),
    onSuccess: (updated) => {
      queryClient.setQueryData<UserAccount[]>(['users'], (prev) =>
        prev?.map((u) => (u.id === updated.id ? updated : u)),
      )
      setError('')
    },
    onError: (e) => setError(e instanceof Error ? e.message : String(e)),
  })

  return (
    <Paper variant="outlined" sx={{ p: 3, mt: 3 }}>
      <Typography variant="h6" sx={{ mb: 0.5 }}>
        Users
      </Typography>
      <Typography variant="body2" color="text.secondary" sx={{ mb: 2 }}>
        Viewers read and browse; editors create and edit what they own; admins edit everything and
        manage users. New sign-ups start as viewers.
      </Typography>
      {error && (
        <Alert severity="error" sx={{ mb: 2 }} onClose={() => setError('')}>
          {error}
        </Alert>
      )}
      <Table size="small">
        <TableHead>
          <TableRow>
            <TableCell>User</TableCell>
            <TableCell sx={{ width: 160 }}>Role</TableCell>
            <TableCell>Joined</TableCell>
            <TableCell sx={{ width: 96 }} align="right">
              Actions
            </TableCell>
          </TableRow>
        </TableHead>
        <TableBody>
          {(users ?? []).map((u) => {
            const isSelf = u.id === user?.id
            return (
              <TableRow key={u.id}>
                <TableCell>
                  {u.username}
                  {isSelf && (
                    <Typography component="span" color="text.secondary" sx={{ ml: 1 }}>
                      (you)
                    </Typography>
                  )}
                </TableCell>
                <TableCell>
                  <Select
                    size="small"
                    fullWidth
                    value={u.role}
                    disabled={isSelf || setRole.isPending}
                    onChange={(e) => setRole.mutate({ id: u.id, role: e.target.value as Role })}
                  >
                    {ROLES.map((r) => (
                      <MenuItem key={r} value={r}>
                        {r}
                      </MenuItem>
                    ))}
                  </Select>
                </TableCell>
                <TableCell>{new Date(u.created_at).toLocaleDateString()}</TableCell>
                <TableCell align="right">
                  {/* Your own account uses the self-service "Change password"; it
                      can't be reset or deleted from here. */}
                  {!isSelf && (
                    <Stack direction="row" spacing={0.5} sx={{ justifyContent: 'flex-end' }}>
                      <Tooltip title="Reset password">
                        <IconButton size="small" onClick={() => setResetting(u)}>
                          <KeyIcon sx={{ fontSize: 18 }} />
                        </IconButton>
                      </Tooltip>
                      <Tooltip title="Delete user">
                        <IconButton size="small" color="error" onClick={() => setDeleting(u)}>
                          <DeleteIcon sx={{ fontSize: 18 }} />
                        </IconButton>
                      </Tooltip>
                    </Stack>
                  )}
                </TableCell>
              </TableRow>
            )
          })}
          {users?.length === 0 && (
            <TableRow>
              <TableCell colSpan={4}>
                <Typography color="text.secondary">No registered users yet.</Typography>
              </TableCell>
            </TableRow>
          )}
        </TableBody>
      </Table>

      <ResetPasswordDialog
        target={resetting}
        onClose={() => setResetting(null)}
        onDone={() => setResetting(null)}
      />
      <DeleteUserDialog
        target={deleting}
        onClose={() => setDeleting(null)}
        onDone={() => {
          setDeleting(null)
          void queryClient.invalidateQueries({ queryKey: ['users'] })
        }}
      />
    </Paper>
  )
}

/// Admin sets a user's password outright — no old-password check.
function ResetPasswordDialog({
  target,
  onClose,
  onDone,
}: {
  target: UserAccount | null
  onClose: () => void
  onDone: () => void
}) {
  const [next, setNext] = useState('')
  const [confirm, setConfirm] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  const [lastId, setLastId] = useState<string | null>(null)
  if (target && target.id !== lastId) {
    setLastId(target.id)
    setNext('')
    setConfirm('')
    setError('')
  }

  const tooShort = next.length > 0 && next.length < 8
  const mismatch = confirm.length > 0 && next !== confirm
  const canSubmit = next.length >= 8 && next === confirm && !busy

  const submit = async () => {
    if (!target) return
    setBusy(true)
    setError('')
    try {
      await api.resetUserPassword(target.id, next)
      onDone()
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={!!target} onClose={onClose} maxWidth="xs" fullWidth>
      <DialogTitle>Reset password{target ? ` — ${target.username}` : ''}</DialogTitle>
      <DialogContent>
        <Stack spacing={2} sx={{ mt: 1 }}>
          {error && <Alert severity="error">{error}</Alert>}
          <Typography variant="body2" color="text.secondary">
            Sets a new password for this account directly. Tell the user their new password — there
            is no email.
          </Typography>
          <TextField
            label="New password"
            type="password"
            autoFocus
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
          Reset password
        </Button>
      </DialogActions>
    </Dialog>
  )
}

/// Delete a user, with a confirm. Their models, bundles, and other content are
/// reassigned to you (the acting admin) so nothing is lost.
function DeleteUserDialog({
  target,
  onClose,
  onDone,
}: {
  target: UserAccount | null
  onClose: () => void
  onDone: () => void
}) {
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  const [lastId, setLastId] = useState<string | null>(null)
  if (target && target.id !== lastId) {
    setLastId(target.id)
    setError('')
  }

  const submit = async () => {
    if (!target) return
    setBusy(true)
    setError('')
    try {
      await api.deleteUser(target.id)
      onDone()
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={!!target} onClose={onClose} maxWidth="xs" fullWidth>
      <DialogTitle>Delete user?</DialogTitle>
      <DialogContent>
        {error && (
          <Alert severity="error" sx={{ mb: 2 }}>
            {error}
          </Alert>
        )}
        <DialogContentText>
          Delete <strong>{target?.username}</strong>? Their models, bundles, and any other content
          will be reassigned to you. This can't be undone.
        </DialogContentText>
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="contained" color="error" onClick={submit} disabled={busy}>
          Delete user
        </Button>
      </DialogActions>
    </Dialog>
  )
}
