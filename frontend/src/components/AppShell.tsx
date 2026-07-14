import { useEffect, useRef, useState } from 'react'
import { Outlet, Link, useLocation, useNavigate, useSearchParams } from 'react-router-dom'
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
  Badge,
  Tooltip,
  LinearProgress,
  alpha,
} from '@mui/material'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import SearchIcon from '@mui/icons-material/Search'
import ViewInArIcon from '@mui/icons-material/ViewInAr'
import DarkModeIcon from '@mui/icons-material/DarkMode'
import LightModeIcon from '@mui/icons-material/LightMode'
import AccountCircleIcon from '@mui/icons-material/AccountCircle'
import UploadFileIcon from '@mui/icons-material/UploadFile'

import { useAuth, useColorMode } from '../main'
import { api } from '../api'
import { readDrop, startImport } from '../upload'
import ImportErrorDialog from './ImportErrorDialog'

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
  const [progress, setProgress] = useState(0)
  const [uploadName, setUploadName] = useState('')
  const [dropError, setDropError] = useState('')

  // The window-level drop listener is registered once; a ref keeps it reading the
  // *current* page rather than the one that was open when it was installed.
  const { pathname } = useLocation()
  const pathRef = useRef(pathname)
  pathRef.current = pathname

  // Global file-first drop: dropping a file anywhere stages it as an import and
  // takes you to its page, where you say what it is once it has unpacked.
  // Editors/admins only; viewers can't create.
  const canCreate = !!user && user.role !== 'viewer'

  // Imports are transient, so the badge polls: an unpack finishing is what makes
  // one actionable, and that happens in the background.
  const { data: staged } = useQuery({
    queryKey: ['imports'],
    queryFn: () => api.imports(),
    enabled: canCreate,
    refetchInterval: 5000,
  })

  useEffect(() => {
    if (!canCreate) return
    let depth = 0
    const hasFiles = (e: DragEvent) => e.dataTransfer?.types.includes('Files')
    // Stand down while a dialog is open: it has its own drop target (e.g. the
    // bundle-patch importer), and the whole-page overlay would otherwise swallow
    // the drop before the dialog ever saw it. MUI keeps a `.MuiDialog-root` in the
    // DOM only while a dialog is mounted; menus and selects use other classes.
    const dialogOpen = () => !!document.querySelector('.MuiDialog-root')
    const onEnter = (e: DragEvent) => {
      if (!hasFiles(e) || dialogOpen()) return
      depth += 1
      setDragging(true)
    }
    const onLeave = () => {
      depth = Math.max(0, depth - 1)
      if (depth === 0) setDragging(false)
    }
    const onOver = (e: DragEvent) => {
      if (hasFiles(e) && !dialogOpen()) e.preventDefault()
    }
    const onDrop = (e: DragEvent) => {
      if (!hasFiles(e) || !e.dataTransfer || dialogOpen()) return
      e.preventDefault()
      depth = 0
      setDragging(false)
      // A folder has to be walked into its files before anything can upload it,
      // and that read must start while the DataTransfer is still alive.
      readDrop(e.dataTransfer)
        .then((drop) => {
          if (!drop.files.length) return
          setImporting(true)
          setProgress(0)
          setUploadName(
            drop.files.length === 1
              ? drop.files[0].file.name
              : `${drop.name} — ${drop.files.length} files`,
          )
          return startImport(drop, setProgress)
            .then(async (staged) => {
              await queryClient.invalidateQueries()
              // Dropped while standing on a bundle? Then that is where it is
              // going, and the import page opens preset to "add to this bundle" —
              // which is why the bundle page needs no drop target of its own.
              const onBundle = pathRef.current.match(/^\/bundles\/([0-9a-fA-F-]{36})/)
              navigate(
                onBundle ? `/imports/${staged.id}?bundle=${onBundle[1]}` : `/imports/${staged.id}`,
              )
            })
            .finally(() => setImporting(false))
        })
        .catch((err) => setDropError(err instanceof Error ? err.message : String(err)))
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

          {canCreate && (
            <Button component={Link} to="/imports" color="inherit">
              <Badge badgeContent={staged?.length ?? 0} color="primary" sx={{ px: 0.5 }}>
                Importing
              </Badge>
            </Button>
          )}
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
            <Box sx={{ width: 360, textAlign: 'center' }}>
              {progress < 1 ? (
                <>
                  <Typography variant="h6" sx={{ mb: 1 }}>
                    Uploading {Math.round(progress * 100)}%
                  </Typography>
                  <LinearProgress variant="determinate" value={progress * 100} />
                </>
              ) : (
                <>
                  <Typography variant="h6" sx={{ mb: 1 }}>
                    Unpacking…
                  </Typography>
                  <LinearProgress />
                </>
              )}
              <Typography variant="body2" color="text.secondary" sx={{ mt: 1 }} noWrap>
                {uploadName}
              </Typography>
            </Box>
          ) : (
            <>
              <UploadFileIcon sx={{ fontSize: 64, color: 'primary.main' }} />
              <Typography variant="h5" sx={{ fontWeight: 700 }}>
                Drop to import
              </Typography>
              <Typography color="text.secondary">
                .zip archives unpack, then you choose: model or bundle
              </Typography>
            </>
          )}
        </Box>
      )}

      <ImportErrorDialog error={dropError} onClose={() => setDropError('')} />
    </Box>
  )
}
