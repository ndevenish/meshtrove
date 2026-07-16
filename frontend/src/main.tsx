import { StrictMode, useMemo, useState, createContext, useContext } from 'react'
import { createRoot } from 'react-dom/client'
import { BrowserRouter, Routes, Route } from 'react-router-dom'
import { QueryClient, QueryClientProvider, useQuery } from '@tanstack/react-query'
import { ThemeProvider, CssBaseline } from '@mui/material'
import useMediaQuery from '@mui/material/useMediaQuery'

import { buildTheme } from './theme'
import { api, type User, ApiError } from './api'
import AppShell from './components/AppShell'
import BrowsePage from './pages/BrowsePage'
import ModelPage from './pages/ModelPage'
import BundlePage from './pages/BundlePage'
import ImportsPage from './pages/ImportsPage'
import ImportPage from './pages/ImportPage'
import ExportsPage from './pages/ExportsPage'
import CreatorsPage from './pages/CreatorsPage'
import LoginPage from './pages/LoginPage'
import AdminPage from './pages/AdminPage'
import JobsPage from './pages/JobsPage'

const queryClient = new QueryClient({
  defaultOptions: { queries: { retry: 1, staleTime: 10_000 } },
})

interface AuthContextValue {
  user: User | null
  loading: boolean
  refresh: () => void
}
const AuthContext = createContext<AuthContextValue>({
  user: null,
  loading: true,
  refresh: () => {},
})
export const useAuth = () => useContext(AuthContext)

type ColorMode = 'light' | 'dark'
const ColorModeContext = createContext<{ mode: ColorMode; toggle: () => void }>({
  mode: 'light',
  toggle: () => {},
})
export const useColorMode = () => useContext(ColorModeContext)

function App() {
  const prefersDark = useMediaQuery('(prefers-color-scheme: dark)')
  const [mode, setMode] = useState<ColorMode>(
    (localStorage.getItem('meshtrove-mode') as ColorMode) ?? (prefersDark ? 'dark' : 'light'),
  )
  const theme = useMemo(() => buildTheme(mode), [mode])
  const toggle = () => {
    const next = mode === 'light' ? 'dark' : 'light'
    setMode(next)
    localStorage.setItem('meshtrove-mode', next)
  }

  const {
    data: user,
    isLoading,
    refetch,
  } = useQuery({
    queryKey: ['me'],
    queryFn: async () => {
      try {
        return await api.me()
      } catch (e) {
        if (e instanceof ApiError && e.status === 401) return null
        throw e
      }
    },
  })

  return (
    <ColorModeContext.Provider value={{ mode, toggle }}>
      <ThemeProvider theme={theme}>
        <CssBaseline />
        <AuthContext.Provider
          value={{ user: user ?? null, loading: isLoading, refresh: () => void refetch() }}
        >
          <Routes>
            <Route path="/login" element={<LoginPage />} />
            <Route element={<AppShell />}>
              <Route path="/" element={<BrowsePage />} />
              <Route path="/models/:id" element={<ModelPage />} />
              <Route path="/bundles/:id" element={<BundlePage />} />
              <Route path="/imports" element={<ImportsPage />} />
              <Route path="/imports/:id" element={<ImportPage />} />
              <Route path="/exports" element={<ExportsPage />} />
              <Route path="/creators" element={<CreatorsPage />} />
              <Route path="/jobs" element={<JobsPage />} />
              <Route path="/admin" element={<AdminPage />} />
            </Route>
          </Routes>
        </AuthContext.Provider>
      </ThemeProvider>
    </ColorModeContext.Provider>
  )
}

createRoot(document.getElementById('root')!).render(
  <StrictMode>
    <QueryClientProvider client={queryClient}>
      <BrowserRouter>
        <App />
      </BrowserRouter>
    </QueryClientProvider>
  </StrictMode>,
)
