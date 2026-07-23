import { useState } from 'react'
import {
  Alert,
  Box,
  Button,
  IconButton,
  MenuItem,
  Paper,
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
import ContentCopyIcon from '@mui/icons-material/ContentCopy'
import DeleteIcon from '@mui/icons-material/Delete'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'

import { api, type ApiToken, type NewApiToken, type Role } from '../api'
import { useAuth } from '../main'

// Least privilege first, so the default selection is the safest useful token.
const ROLES: { value: Role; label: string }[] = [
  { value: 'viewer', label: 'Viewer — read only' },
  { value: 'editor', label: 'Editor — create & edit own' },
  { value: 'admin', label: 'Admin — full access' },
]

/// Admin-only: mint and revoke API tokens. A token is an `Authorization: Bearer`
/// credential that acts with the abilities of the admin who created it, for
/// scripts and CI that can't hold a browser cookie. The plaintext is shown once,
/// right after creation — only its hash is stored, so it can't be shown again.
export default function ApiTokensPanel() {
  const { user } = useAuth()
  const queryClient = useQueryClient()
  const [name, setName] = useState('')
  const [role, setRole] = useState<Role>('viewer') // default to the safest token
  const [expires, setExpires] = useState('') // yyyy-mm-dd, or '' for never
  const [error, setError] = useState('')
  // The just-created token, held so its plaintext can be shown and copied once.
  const [fresh, setFresh] = useState<NewApiToken | null>(null)
  const [copied, setCopied] = useState(false)

  const { data: tokens } = useQuery({
    queryKey: ['api-tokens'],
    queryFn: () => api.apiTokens(),
    enabled: user?.role === 'admin',
  })

  const create = useMutation({
    mutationFn: () =>
      api.createApiToken(name.trim(), role, expires ? new Date(expires).toISOString() : null),
    onSuccess: (token) => {
      setFresh(token)
      setCopied(false)
      setName('')
      setRole('viewer')
      setExpires('')
      setError('')
      void queryClient.invalidateQueries({ queryKey: ['api-tokens'] })
    },
    onError: (e) => setError(e instanceof Error ? e.message : String(e)),
  })

  const revoke = useMutation({
    mutationFn: (id: string) => api.deleteApiToken(id),
    onSuccess: (_data, id) => {
      // If the revoked token is the one still shown in the copy box, clear it.
      setFresh((f) => (f && f.id === id ? null : f))
      setError('')
      void queryClient.invalidateQueries({ queryKey: ['api-tokens'] })
    },
    onError: (e) => setError(e instanceof Error ? e.message : String(e)),
  })

  const copy = async () => {
    if (!fresh) return
    try {
      await navigator.clipboard.writeText(fresh.token)
      setCopied(true)
    } catch {
      setCopied(false)
    }
  }

  const when = (iso: string | null) => (iso ? new Date(iso).toLocaleDateString() : '—')

  return (
    <Paper variant="outlined" sx={{ p: 3, mt: 3 }}>
      <Typography variant="h6" sx={{ mb: 0.5 }}>
        API tokens
      </Typography>
      <Typography variant="body2" color="text.secondary" sx={{ mb: 2 }}>
        A token lets a script or CI job reach the API without a browser, by sending{' '}
        <code>Authorization: Bearer &lt;token&gt;</code>. Pick the least role it needs — a viewer
        token can only read. A token can never exceed your own role, and is capped at whatever your
        role is when it’s used. Keep it secret and revoke any you no longer use.
      </Typography>

      {error && (
        <Alert severity="error" sx={{ mb: 2 }} onClose={() => setError('')}>
          {error}
        </Alert>
      )}

      {/* Shown once, immediately after creation. */}
      {fresh && (
        <Alert severity="success" sx={{ mb: 2 }} onClose={() => setFresh(null)}>
          <Typography variant="body2" sx={{ mb: 1 }}>
            Token <strong>{fresh.name}</strong> ({fresh.role}) created. Copy it now — it won’t be
            shown again.
          </Typography>
          <Stack direction="row" spacing={1} sx={{ alignItems: 'center' }}>
            <Box
              component="code"
              sx={{
                flexGrow: 1,
                p: 1,
                borderRadius: 1,
                bgcolor: 'action.hover',
                fontFamily: 'monospace',
                fontSize: 13,
                overflowWrap: 'anywhere',
              }}
            >
              {fresh.token}
            </Box>
            <Button size="small" variant="outlined" startIcon={<ContentCopyIcon />} onClick={copy}>
              {copied ? 'Copied' : 'Copy'}
            </Button>
          </Stack>
        </Alert>
      )}

      <Stack
        direction={{ xs: 'column', sm: 'row' }}
        spacing={2}
        sx={{ mb: 2, alignItems: { sm: 'center' } }}
      >
        <TextField
          label="Name"
          placeholder="e.g. CI deploy"
          size="small"
          value={name}
          onChange={(e) => setName(e.target.value)}
          sx={{ flexGrow: 1 }}
        />
        <TextField
          select
          label="Role"
          size="small"
          value={role}
          onChange={(e) => setRole(e.target.value as Role)}
          sx={{ minWidth: 220 }}
        >
          {ROLES.map((r) => (
            <MenuItem key={r.value} value={r.value}>
              {r.label}
            </MenuItem>
          ))}
        </TextField>
        <TextField
          label="Expires (optional)"
          type="date"
          size="small"
          value={expires}
          onChange={(e) => setExpires(e.target.value)}
          slotProps={{ inputLabel: { shrink: true } }}
        />
        <Button
          variant="contained"
          onClick={() => create.mutate()}
          disabled={!name.trim() || create.isPending}
        >
          Generate
        </Button>
      </Stack>

      <Table size="small">
        <TableHead>
          <TableRow>
            <TableCell>Name</TableCell>
            <TableCell>Role</TableCell>
            <TableCell>Created</TableCell>
            <TableCell>Last used</TableCell>
            <TableCell>Expires</TableCell>
            <TableCell sx={{ width: 48 }} align="right" />
          </TableRow>
        </TableHead>
        <TableBody>
          {(tokens ?? []).map((t: ApiToken) => (
            <TableRow key={t.id}>
              <TableCell>
                {t.name}
                <Typography component="span" color="text.secondary" sx={{ ml: 1 }}>
                  ({t.created_by_username})
                </Typography>
              </TableCell>
              <TableCell sx={{ textTransform: 'capitalize' }}>{t.role}</TableCell>
              <TableCell>{when(t.created_at)}</TableCell>
              <TableCell>{when(t.last_used_at)}</TableCell>
              <TableCell>{when(t.expires_at)}</TableCell>
              <TableCell align="right">
                <Tooltip title="Revoke token">
                  <IconButton
                    size="small"
                    color="error"
                    disabled={revoke.isPending}
                    onClick={() => revoke.mutate(t.id)}
                  >
                    <DeleteIcon sx={{ fontSize: 18 }} />
                  </IconButton>
                </Tooltip>
              </TableCell>
            </TableRow>
          ))}
          {tokens?.length === 0 && (
            <TableRow>
              <TableCell colSpan={6}>
                <Typography color="text.secondary">No tokens yet.</Typography>
              </TableCell>
            </TableRow>
          )}
        </TableBody>
      </Table>
    </Paper>
  )
}
