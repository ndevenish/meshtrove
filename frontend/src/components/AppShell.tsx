import { useState } from 'react'
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
  alpha,
} from '@mui/material'
import SearchIcon from '@mui/icons-material/Search'
import ViewInArIcon from '@mui/icons-material/ViewInAr'
import DarkModeIcon from '@mui/icons-material/DarkMode'
import LightModeIcon from '@mui/icons-material/LightMode'
import AccountCircleIcon from '@mui/icons-material/AccountCircle'

import { useAuth, useColorMode } from '../main'
import { api } from '../api'

export default function AppShell() {
  const { user, refresh } = useAuth()
  const { mode, toggle } = useColorMode()
  const navigate = useNavigate()
  const [params] = useSearchParams()
  const [search, setSearch] = useState(params.get('q') ?? '')
  const [menuAnchor, setMenuAnchor] = useState<HTMLElement | null>(null)

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
    </Box>
  )
}
