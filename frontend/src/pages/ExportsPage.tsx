import {
  Container,
  Typography,
  Paper,
  Stack,
  Box,
  Button,
  Chip,
  CircularProgress,
  Tooltip,
} from '@mui/material'
import DownloadIcon from '@mui/icons-material/Download'
import DeleteIcon from '@mui/icons-material/Delete'
import ArchiveIcon from '@mui/icons-material/Archive'
import ContentCopyIcon from '@mui/icons-material/ContentCopy'
import CheckIcon from '@mui/icons-material/Check'
import { useState } from 'react'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import { api, exportDownloadUrl, formatBytes } from '../api'

/// Exports the user has built. A build runs as a background job, so this page
/// polls while any are still building; a finished one is downloaded here and
/// deleted when no longer needed.
export default function ExportsPage() {
  const queryClient = useQueryClient()
  const { data: exports, isLoading } = useQuery({
    queryKey: ['exports'],
    queryFn: () => api.exports(),
    // Poll while anything is still building.
    refetchInterval: (query) =>
      (query.state.data ?? []).some((e) => e.status === 'building') ? 2000 : false,
  })

  const remove = async (id: string) => {
    await api.deleteExport(id)
    await queryClient.invalidateQueries({ queryKey: ['exports'] })
  }

  return (
    <Container maxWidth="md" sx={{ py: 3 }}>
      <Typography variant="h5" sx={{ mb: 0.5 }}>
        Exports
      </Typography>
      <Typography color="text.secondary" sx={{ mb: 2 }}>
        Archives you&rsquo;ve built. Start one from the Export button on a model or bundle.
      </Typography>

      {!isLoading && exports?.length === 0 && (
        <Paper variant="outlined" sx={{ p: 6, textAlign: 'center' }}>
          <ArchiveIcon sx={{ fontSize: 48, color: 'text.disabled' }} />
          <Typography color="text.secondary" sx={{ mt: 1 }}>
            No exports yet. Use Export on a model or bundle to build one.
          </Typography>
        </Paper>
      )}

      <Stack spacing={1}>
        {(exports ?? []).map((e) => (
          <Paper key={e.id} variant="outlined" sx={{ p: 2 }}>
            <Stack direction="row" spacing={2} sx={{ alignItems: 'center' }}>
              <Box sx={{ flexGrow: 1, minWidth: 0 }}>
                <Typography sx={{ fontWeight: 600 }} noWrap>
                  {e.name}
                </Typography>
                <Typography variant="body2" color="text.secondary">
                  {e.model_count} model{e.model_count === 1 ? '' : 's'}
                  {e.status === 'ready' && e.size != null && ` · ${formatBytes(e.size)}`}
                </Typography>
              </Box>

              {e.status === 'building' && (
                <Chip
                  size="small"
                  icon={<CircularProgress size={12} sx={{ ml: 1 }} />}
                  label="Building"
                />
              )}
              {e.status === 'failed' && (
                <Tooltip title={e.error ?? 'Build failed'}>
                  <Chip size="small" color="error" label="Failed" />
                </Tooltip>
              )}
              {e.status === 'ready' && (
                <Button
                  component="a"
                  href={exportDownloadUrl(e.id)}
                  variant="contained"
                  startIcon={<DownloadIcon />}
                >
                  Download
                </Button>
              )}
              <Tooltip title="Delete export">
                <Button color="error" onClick={() => void remove(e.id)} startIcon={<DeleteIcon />}>
                  Delete
                </Button>
              </Tooltip>
            </Stack>
            {/* Admins get the artifact's on-disk path (backend sends it only to
                them, only when ready) — to grab the zip straight off the store. */}
            {e.path && <PathBox path={e.path} />}
          </Paper>
        ))}
      </Stack>
    </Container>
  )
}

/// The store path of a ready export, in a monospace box with a copy button.
function PathBox({ path }: { path: string }) {
  const [copied, setCopied] = useState(false)
  const copy = async () => {
    try {
      await navigator.clipboard.writeText(path)
      setCopied(true)
      setTimeout(() => setCopied(false), 1500)
    } catch {
      // Clipboard blocked (e.g. a non-secure origin) — the path stays selectable.
    }
  }
  return (
    <Stack direction="row" spacing={1} sx={{ mt: 1.5, alignItems: 'stretch' }}>
      <Box
        title={path}
        sx={{
          flexGrow: 1,
          minWidth: 0,
          fontFamily: 'monospace',
          fontSize: 12,
          bgcolor: 'action.hover',
          borderRadius: 1,
          px: 1,
          py: 0.75,
          overflowX: 'auto',
          whiteSpace: 'nowrap',
          display: 'flex',
          alignItems: 'center',
        }}
      >
        {path}
      </Box>
      <Button
        size="small"
        variant="outlined"
        onClick={() => void copy()}
        startIcon={copied ? <CheckIcon /> : <ContentCopyIcon />}
        sx={{ flexShrink: 0, whiteSpace: 'nowrap' }}
      >
        {copied ? 'Copied' : 'Copy path'}
      </Button>
    </Stack>
  )
}
