import { useEffect, useState } from 'react'
import { Outlet, Link, useNavigate, useSearchParams } from 'react-router-dom'
import {
  AppBar,
  Toolbar,
  Typography,
  InputBase,
  Box,
  IconButton,
  Menu,
  MenuItem,
  Button,
  Tooltip,
  Snackbar,
  Alert,
  CircularProgress,
  alpha,
} from '@mui/material'
import { useQueryClient } from '@tanstack/react-query'
import SearchIcon from '@mui/icons-material/Search'
import ViewInArIcon from '@mui/icons-material/ViewInAr'
import DarkModeIcon from '@mui/icons-material/DarkMode'
import LightModeIcon from '@mui/icons-material/LightMode'
import AccountCircleIcon from '@mui/icons-material/AccountCircle'
import UploadFileIcon from '@mui/icons-material/UploadFile'

import { useAuth, useColorMode } from '../main'
import { api } from '../api'
import { importArchiveAsModel } from '../upload'

export default function AppShell() {
  const { user, refresh } = useAuth()
  const { mode, toggle } = useColorMode()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [params] = useSearchParams()
  const [search, setSearch] = useState(params.get('q') ?? '')
  const [menuAnchor, setMenuAnchor] = useState<HTMLElement | null>(null)
  const [dragging, setDragging] = useState(false)
  const [importing, setImporting] = useState(false)
  const [dropError, setDropError] = useState('')

  // Global file-first drop: dropping a file anywhere creates a new model named
  // after it and unpacks it. Editors/admins only; viewers can't create.
  const canCreate = !!user && user.role !== 'viewer'
  useEffect(() => {
    if (!canCreate) return
    let depth = 0
    const hasFiles = (e: DragEvent) => e.dataTransfer?.types.includes('Files')
    const onEnter = (e: DragEvent) => {
      if (!hasFiles(e)) return
      depth += 1
      setDragging(true)
    }
    const onLeave = () => {
      depth = Math.max(0, depth - 1)
      if (depth === 0) setDragging(false)
    }
    const onOver = (e: DragEvent) => {
      if (hasFiles(e)) e.preventDefault()
    }
    const onDrop = (e: DragEvent) => {
      if (!hasFiles(e)) return
      e.preventDefault()
      depth = 0
      setDragging(false)
      const file = e.dataTransfer?.files?.[0]
      if (!file) return
      setImporting(true)
      importArchiveAsModel(file)
        .then(async (model) => {
          await queryClient.invalidateQueries()
          navigate(`/models/${model.id}`)
        })
        .catch((err) => setDropError(err instanceof Error ? err.message : String(err)))
        .finally(() => setImporting(false))
    }
    window.addEventListener('dragenter', onEnter)
    window.addEventListener('dragleave', onLeave)
    window.addEventListener('dragover', onOver)
    window.addEventListener('drop', onDrop)
    return () => {
      window.removeEventListener('dragenter', onEnter)
      window.removeEventListener('dragleave', onLeave)
      window.removeEventListener('dragover', onOver)
      window.removeEventListener('drop', onDrop)
    }
  }, [canCreate, navigate, queryClient])

  const submitSearch = (e: React.FormEvent) => {
    e.preventDefault()
    const next = new URLSearchParams(params)
    if (search) next.set('q', search)
    else next.delete('q')
    next.delete('page')
    navigate(`/?${next}`)
  }

  return (
    <Box sx={{ minHeight: '100vh', display: 'flex', flexDirection: 'column' }}>
      <AppBar position="sticky" elevation={1}>
        <Toolbar sx={{ gap: 2 }}>
          <Box
            component={Link}
            to="/"
            sx={{
              display: 'flex',
              alignItems: 'center',
              gap: 1,
              textDecoration: 'none',
              color: 'inherit',
            }}
          >
            <ViewInArIcon sx={{ color: 'primary.main', fontSize: 30 }} />
            <Typography variant="h6" sx={{ fontWeight: 800, letterSpacing: -0.5 }}>
              Mesh
              <Box component="span" sx={{ color: 'primary.main' }}>
                Trove
              </Box>
            </Typography>
          </Box>

          <Box
            component="form"
            onSubmit={submitSearch}
            sx={(theme) => ({
              flexGrow: 1,
              maxWidth: 640,
              mx: 'auto',
              display: 'flex',
              alignItems: 'center',
              borderRadius: 99,
              px: 2,
              py: 0.5,
              backgroundColor: alpha(theme.palette.text.primary, 0.06),
              '&:focus-within': {
                backgroundColor: alpha(theme.palette.text.primary, 0.1),
              },
            })}
          >
            <SearchIcon sx={{ mr: 1, opacity: 0.6 }} />
            <InputBase
              fullWidth
              placeholder="Search models…"
              value={search}
              onChange={(e) => setSearch(e.target.value)}
            />
          </Box>

          <Button component={Link} to="/creators" color="inherit">
            Creators
          </Button>
          <Button component={Link} to="/jobs" color="inherit">
            Jobs
          </Button>
          <Tooltip title={mode === 'light' ? 'Dark mode' : 'Light mode'}>
            <IconButton onClick={toggle} color="inherit">
              {mode === 'light' ? <DarkModeIcon /> : <LightModeIcon />}
            </IconButton>
          </Tooltip>

          {user ? (
            <>
              <IconButton onClick={(e) => setMenuAnchor(e.currentTarget)} color="inherit">
                <AccountCircleIcon />
              </IconButton>
              <Menu
                anchorEl={menuAnchor}
                open={menuAnchor !== null}
                onClose={() => setMenuAnchor(null)}
              >
                <MenuItem disabled>
                  {user.username} ({user.role})
                </MenuItem>
                {user.role === 'admin' && (
                  <MenuItem
                    onClick={() => {
                      setMenuAnchor(null)
                      navigate('/admin')
                    }}
                  >
                    Admin settings
                  </MenuItem>
                )}
                <MenuItem
                  onClick={async () => {
                    setMenuAnchor(null)
                    await api.logout()
                    refresh()
                  }}
                >
                  Log out
                </MenuItem>
              </Menu>
            </>
          ) : (
            <Button component={Link} to="/login" variant="contained">
              Log in
            </Button>
          )}
        </Toolbar>
      </AppBar>
      <Box component="main" sx={{ flexGrow: 1 }}>
        <Outlet />
      </Box>

      {(dragging || importing) && (
        <Box
          sx={(theme) => ({
            position: 'fixed',
            inset: 0,
            zIndex: theme.zIndex.modal + 1,
            display: 'flex',
            flexDirection: 'column',
            alignItems: 'center',
            justifyContent: 'center',
            gap: 2,
            pointerEvents: 'none',
            backgroundColor: alpha(theme.palette.background.default, 0.85),
            border: `3px dashed ${theme.palette.primary.main}`,
          })}
        >
          {importing ? (
            <>
              <CircularProgress />
              <Typography variant="h6">Importing…</Typography>
            </>
          ) : (
            <>
              <UploadFileIcon sx={{ fontSize: 64, color: 'primary.main' }} />
              <Typography variant="h5" sx={{ fontWeight: 700 }}>
                Drop to import as a new model
              </Typography>
              <Typography color="text.secondary">.zip archives unpack automatically</Typography>
            </>
          )}
        </Box>
      )}

      <Snackbar
        open={!!dropError}
        autoHideDuration={8000}
        onClose={() => setDropError('')}
        anchorOrigin={{ vertical: 'bottom', horizontal: 'center' }}
      >
        <Alert severity="error" onClose={() => setDropError('')}>
          Import failed: {dropError}
        </Alert>
      </Snackbar>
    </Box>
  )
}
