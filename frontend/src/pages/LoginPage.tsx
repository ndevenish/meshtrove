import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { Box, Paper, Typography, TextField, Button, Stack, Alert, Tabs, Tab } from '@mui/material'
import ViewInArIcon from '@mui/icons-material/ViewInAr'

import { api } from '../api'
import { useAuth } from '../main'

export default function LoginPage() {
  const navigate = useNavigate()
  const { refresh } = useAuth()
  const [tab, setTab] = useState(0)
  const [username, setUsername] = useState('')
  const [password, setPassword] = useState('')
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)

  const submit = async (e: React.FormEvent) => {
    e.preventDefault()
    setBusy(true)
    setError('')
    try {
      if (tab === 0) await api.login(username, password)
      else await api.register(username, password)
      refresh()
      navigate('/')
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Box
      sx={{
        minHeight: '100vh',
        display: 'flex',
        alignItems: 'center',
        justifyContent: 'center',
        bgcolor: 'background.default',
      }}
    >
      <Paper sx={{ p: 4, width: 380 }} component="form" onSubmit={submit}>
        <Stack sx={{ alignItems: 'center' }} spacing={2}>
          <Stack sx={{ alignItems: 'center' }} direction="row" spacing={1}>
            <ViewInArIcon sx={{ color: 'primary.main', fontSize: 36 }} />
            <Typography variant="h5" sx={{ fontWeight: 800 }}>
              Mesh
              <Box component="span" sx={{ color: 'primary.main' }}>
                Trove
              </Box>
            </Typography>
          </Stack>
          <Tabs value={tab} onChange={(_, value) => setTab(value)}>
            <Tab label="Log in" />
            <Tab label="Register" />
          </Tabs>
          {error && (
            <Alert severity="error" sx={{ width: '100%' }}>
              {error}
            </Alert>
          )}
          <TextField
            label="Username"
            fullWidth
            value={username}
            onChange={(e) => setUsername(e.target.value)}
            autoFocus
          />
          <TextField
            label="Password"
            type="password"
            fullWidth
            value={password}
            onChange={(e) => setPassword(e.target.value)}
            helperText={tab === 1 ? 'At least 8 characters' : undefined}
          />
          <Button type="submit" variant="contained" fullWidth disabled={busy}>
            {tab === 0 ? 'Log in' : 'Create account'}
          </Button>
        </Stack>
      </Paper>
    </Box>
  )
}
