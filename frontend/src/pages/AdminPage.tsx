import { useState } from 'react'
import {
  Container,
  Typography,
  Paper,
  Stack,
  TextField,
  Button,
  Alert,
  MenuItem,
} from '@mui/material'
import { useQuery } from '@tanstack/react-query'

import { api } from '../api'
import { useAuth } from '../main'

/// Renderer configuration + bulk re-render. Changing the renderer only
/// affects new renders; "re-render stale" migrates existing images.
export default function AdminPage() {
  const { user } = useAuth()
  const [tool, setTool] = useState<string | null>(null)
  const [args, setArgs] = useState<string | null>(null)
  const [scope, setScope] = useState<'stale' | 'all'>('stale')
  const [mode, setMode] = useState<'add' | 'replace'>('replace')
  const [message, setMessage] = useState('')
  const [error, setError] = useState('')

  const { data: config, refetch } = useQuery({
    queryKey: ['renderer-config'],
    queryFn: () => api.rendererConfig(),
    enabled: user?.role === 'admin',
  })

  if (user && user.role !== 'admin') {
    return (
      <Container sx={{ py: 3 }}>
        <Alert severity="error">Admin access required.</Alert>
      </Container>
    )
  }
  if (!config) return null

  const toolValue = tool ?? config.tool
  const argsValue = args ?? config.args.join('\n')

  const save = async () => {
    setError('')
    setMessage('')
    try {
      await api.setRendererConfig({
        tool: toolValue.trim(),
        args: argsValue
          .split('\n')
          .map((a) => a.trim())
          .filter(Boolean),
      })
      await refetch()
      setMessage(
        'Renderer saved. Existing images are untouched — use re-render below to refresh them.',
      )
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }

  const rerender = async () => {
    setError('')
    setMessage('')
    try {
      const result = await api.rerender(scope, mode)
      setMessage(`Queued ${result.jobs_queued} render job(s). Watch progress on the Jobs page.`)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }

  return (
    <Container maxWidth="md" sx={{ py: 3 }}>
      <Typography variant="h5" sx={{ mb: 2 }}>
        Admin settings
      </Typography>
      {message && (
        <Alert severity="success" sx={{ mb: 2 }}>
          {message}
        </Alert>
      )}
      {error && (
        <Alert severity="error" sx={{ mb: 2 }}>
          {error}
        </Alert>
      )}

      <Paper variant="outlined" sx={{ p: 3, mb: 3 }}>
        <Typography variant="h6" sx={{ mb: 0.5 }}>
          Preview renderer
        </Typography>
        <Typography variant="body2" color="text.secondary" sx={{ mb: 2 }}>
          External command used to render STL previews. <code>{'{input}'}</code> and{' '}
          <code>{'{output}'}</code> are substituted per render. Changing this affects{' '}
          <strong>new renders only</strong>.
        </Typography>
        <Stack spacing={2}>
          <TextField
            label="Tool"
            value={toolValue}
            onChange={(e) => setTool(e.target.value)}
            sx={{ maxWidth: 300 }}
          />
          <TextField
            label="Arguments (one per line)"
            value={argsValue}
            onChange={(e) => setArgs(e.target.value)}
            multiline
            minRows={5}
            sx={{ fontFamily: 'monospace' }}
          />
          <Button variant="contained" onClick={save} sx={{ alignSelf: 'flex-start' }}>
            Save renderer
          </Button>
        </Stack>
      </Paper>

      <Paper variant="outlined" sx={{ p: 3 }}>
        <Typography variant="h6" sx={{ mb: 0.5 }}>
          Re-render previews
        </Typography>
        <Typography variant="body2" color="text.secondary" sx={{ mb: 2 }}>
          Queue re-renders for images produced by a previous renderer configuration
          (&ldquo;stale&rdquo;) or for everything.
        </Typography>
        <Stack sx={{ alignItems: 'center' }} direction="row" spacing={2}>
          <TextField
            select
            label="Scope"
            value={scope}
            onChange={(e) => setScope(e.target.value as 'stale' | 'all')}
            sx={{ width: 160 }}
          >
            <MenuItem value="stale">Stale only</MenuItem>
            <MenuItem value="all">All rendered</MenuItem>
          </TextField>
          <TextField
            select
            label="Mode"
            value={mode}
            onChange={(e) => setMode(e.target.value as 'add' | 'replace')}
            sx={{ width: 200 }}
          >
            <MenuItem value="replace">Replace old image</MenuItem>
            <MenuItem value="add">Add alongside</MenuItem>
          </TextField>
          <Button variant="contained" onClick={rerender}>
            Queue re-renders
          </Button>
        </Stack>
      </Paper>
    </Container>
  )
}
