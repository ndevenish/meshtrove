import { Box, Typography, Snackbar, Alert, Button } from '@mui/material'
import { useQuery } from '@tanstack/react-query'

import { api } from '../api'

/// How often to ask the server what it is running. A redeploy is not urgent —
/// the open page keeps working until it hits a changed API — so this is a slow
/// background check, not a heartbeat.
const POLL_MS = 60_000

/// The build stamp, bottom of every page, plus the nag that follows from it.
///
/// `__APP_VERSION__` is baked into this bundle at build time while `/api/version`
/// reports what the server is running *now*; the two are stamped from the same
/// `git describe` (see vite.config.ts and backend/build.rs), so they match until
/// the server is redeployed under a page someone left open. That mismatch is the
/// signal — the loaded JS is now older than the API it is talking to, which is
/// how you get a stale client calling an endpoint that has changed shape.
///
/// Not in dev: the two stamps drift independently there (the SPA's is fixed when
/// Vite starts, the backend's when it was last compiled), so the check would nag
/// about a redeploy that never happened.
export default function VersionBar() {
  const { data } = useQuery({
    queryKey: ['version'],
    queryFn: () => api.version(),
    enabled: !import.meta.env.DEV,
    refetchInterval: POLL_MS,
    // A failed poll means the server is down or restarting — which is normal
    // mid-redeploy. Say nothing and try again on the next tick.
    retry: false,
  })

  const serverVersion = data?.version
  const stale = !!serverVersion && serverVersion !== __APP_VERSION__

  return (
    <>
      <Box
        component="footer"
        sx={{ px: 2, py: 1.5, textAlign: 'center', opacity: 0.55, userSelect: 'text' }}
      >
        <Typography variant="caption" color="text.secondary">
          MeshTrove {__APP_VERSION__}
          {stale && ` — server is running ${serverVersion}`}
        </Typography>
      </Box>

      <Snackbar open={stale} anchorOrigin={{ vertical: 'bottom', horizontal: 'left' }}>
        <Alert
          severity="info"
          action={
            <Button color="inherit" size="small" onClick={() => window.location.reload()}>
              Reload
            </Button>
          }
        >
          A new version is available.
        </Alert>
      </Snackbar>
    </>
  )
}
