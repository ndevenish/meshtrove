import { useState } from 'react'
import {
  Alert,
  MenuItem,
  Paper,
  Select,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableRow,
  Typography,
} from '@mui/material'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'

import { api, type Role, type UserAccount } from '../api'
import { useAuth } from '../main'

const ROLES: Role[] = ['admin', 'editor', 'viewer']

/// Admin-only: list accounts and change their roles. Your own row is locked —
/// the backend refuses a self-role-change so an admin can't demote the last
/// admin out of its own settings.
export default function UsersPanel() {
  const { user } = useAuth()
  const queryClient = useQueryClient()
  const [error, setError] = useState('')

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
              </TableRow>
            )
          })}
          {users?.length === 0 && (
            <TableRow>
              <TableCell colSpan={3}>
                <Typography color="text.secondary">No registered users yet.</Typography>
              </TableCell>
            </TableRow>
          )}
        </TableBody>
      </Table>
    </Paper>
  )
}
