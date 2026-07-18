import { createTheme } from '@mui/material/styles'

/// The filled-heart colour on a liked card. Deliberately not `palette.primary`:
/// the signature orange is the app's "act on this" colour, and a liked card is
/// not a call to action. Change it here and every heart follows.
export const LIKE_COLOR = '#e53935'

// Printables-inspired: signature orange on clean surfaces, rounded cards,
// dense grids. One theme per mode, switched by AppShell.
export function buildTheme(mode: 'light' | 'dark') {
  return createTheme({
    palette: {
      mode,
      primary: { main: '#FA6831', contrastText: '#ffffff' },
      secondary: { main: '#1a2530' },
      background:
        mode === 'light'
          ? { default: '#f5f5f5', paper: '#ffffff' }
          : { default: '#14191f', paper: '#1c232b' },
    },
    shape: { borderRadius: 10 },
    typography: {
      fontFamily: "'Inter', 'Roboto', 'Helvetica', 'Arial', sans-serif",
      h5: { fontWeight: 700 },
      h6: { fontWeight: 600 },
    },
    components: {
      MuiAppBar: {
        styleOverrides: {
          root:
            mode === 'light'
              ? { backgroundColor: '#ffffff', color: '#1a2530' }
              : { backgroundColor: '#1c232b', color: '#e8edf2' },
        },
      },
      MuiCard: {
        styleOverrides: {
          root: {
            transition: 'box-shadow 120ms ease',
            '&:hover': { boxShadow: '0 4px 20px rgba(0,0,0,0.18)' },
          },
        },
      },
      MuiButton: { styleOverrides: { root: { textTransform: 'none', fontWeight: 600 } } },
      MuiChip: { styleOverrides: { root: { fontWeight: 500 } } },
    },
  })
}
