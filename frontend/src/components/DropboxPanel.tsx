import { useState } from 'react'
import { useNavigate } from 'react-router-dom'
import {
  Alert,
  Box,
  Button,
  Chip,
  CircularProgress,
  Collapse,
  IconButton,
  Paper,
  Stack,
  Tooltip,
  Typography,
} from '@mui/material'
import ExpandMoreIcon from '@mui/icons-material/ExpandMore'
import ExpandLessIcon from '@mui/icons-material/ExpandLess'
import FolderIcon from '@mui/icons-material/Folder'
import InsertDriveFileIcon from '@mui/icons-material/InsertDriveFile'
import RefreshIcon from '@mui/icons-material/Refresh'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'

import { api, formatBytes, type DropboxEntry } from '../api'
import { useAuth } from '../main'

/// The other way in: a folder on the server (`<store>/imports`) that an admin
/// fills over ssh or a file share, listed here with a button per entry. The
/// browser is the wrong pipe for a 40GB box set that is already on the machine —
/// uploading it would copy it back over the network to land where it started.
///
/// A pickup produces the same staged import a browser drop does, so the flow
/// after pressing the button is the ordinary one: open the import, say what it
/// is, commit. Admin-only, because it reads the server's filesystem; the panel is
/// simply absent for everyone else.
export default function DropboxPanel() {
  const { user } = useAuth()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [open, setOpen] = useState(false)
  const [error, setError] = useState('')

  const isAdmin = user?.role === 'admin'
  const { data, isLoading, isFetching, refetch } = useQuery({
    queryKey: ['dropbox'],
    queryFn: () => api.dropbox(),
    enabled: isAdmin,
    // Listing sizes a folder by walking it, so this is more expensive than the
    // imports list next to it — poll gently, and only while the panel is open.
    refetchInterval: open ? 10_000 : false,
  })

  const pickUp = useMutation({
    mutationFn: (entry: string) => api.pickUpDropboxEntry(entry),
    onSuccess: async (staged) => {
      setError('')
      // The copy runs as a job; the import already exists, so go straight to it
      // and let the page follow the fill through `unpacking`.
      await queryClient.invalidateQueries({ queryKey: ['imports'] })
      await queryClient.invalidateQueries({ queryKey: ['dropbox'] })
      navigate(`/imports/${staged.id}`)
    },
    onError: (e: Error) => setError(e.message),
  })

  if (!isAdmin) return null

  const entries = data?.entries ?? []

  return (
    <Paper variant="outlined" sx={{ p: 2, mb: 2 }}>
      <Stack direction="row" spacing={1} sx={{ alignItems: 'center' }}>
        <Box sx={{ flexGrow: 1, minWidth: 0 }}>
          <Typography sx={{ fontWeight: 600 }}>
            From the server{' '}
            {entries.length > 0 && <Chip size="small" label={entries.length} sx={{ ml: 0.5 }} />}
          </Typography>
          <Typography
            variant="body2"
            color="text.secondary"
            title={data?.path}
            sx={{
              fontFamily: 'monospace',
              fontSize: 12,
              mt: 0.25,
              overflowX: 'auto',
              whiteSpace: 'nowrap',
            }}
          >
            {data?.path ?? '…'}
          </Typography>
        </Box>
        <Tooltip title="Rescan the folder">
          <span>
            <IconButton onClick={() => void refetch()} disabled={isFetching}>
              <RefreshIcon />
            </IconButton>
          </span>
        </Tooltip>
        <Button
          onClick={() => setOpen((v) => !v)}
          endIcon={open ? <ExpandLessIcon /> : <ExpandMoreIcon />}
        >
          {open ? 'Hide' : 'Show'}
        </Button>
      </Stack>

      <Collapse in={open}>
        <Box sx={{ mt: 2 }}>
          {error && (
            <Alert severity="error" onClose={() => setError('')} sx={{ mb: 1.5 }}>
              {error}
            </Alert>
          )}
          {isLoading && <CircularProgress size={20} />}
          {!isLoading && entries.length === 0 && (
            <Typography variant="body2" color="text.secondary">
              Nothing here. Copy an archive or a model folder into the path above and rescan.
            </Typography>
          )}
          <Stack spacing={1}>
            {entries.map((entry) => (
              <EntryRow
                key={entry.name}
                entry={entry}
                busy={pickUp.isPending && pickUp.variables === entry.name}
                onImport={() => pickUp.mutate(entry.name)}
              />
            ))}
          </Stack>
          {entries.length > 0 && (
            <Typography variant="caption" color="text.secondary" sx={{ display: 'block', mt: 1.5 }}>
              Importing copies the entry into the store; the original stays here until you delete
              it.
            </Typography>
          )}
        </Box>
      </Collapse>
    </Paper>
  )
}

function EntryRow({
  entry,
  busy,
  onImport,
}: {
  entry: DropboxEntry
  busy: boolean
  onImport: () => void
}) {
  // An entry is not consumed by a pickup, so nothing stops a second one but this
  // — and a second pickup of a big folder is a long, silent waste of disk.
  const disabled = busy || entry.importing || entry.file_count === 0
  return (
    <Stack
      direction="row"
      spacing={1.5}
      sx={{ alignItems: 'center', px: 1.5, py: 1, borderRadius: 1, bgcolor: 'action.hover' }}
    >
      {entry.is_dir ? <FolderIcon color="action" /> : <InsertDriveFileIcon color="action" />}
      <Box sx={{ flexGrow: 1, minWidth: 0 }}>
        <Typography noWrap title={entry.name}>
          {entry.name}
        </Typography>
        <Typography variant="body2" color="text.secondary">
          {entry.file_count} file{entry.file_count === 1 ? '' : 's'} · {formatBytes(entry.size)}
        </Typography>
      </Box>
      {entry.importing && (
        <Chip size="small" icon={<CircularProgress size={12} sx={{ ml: 1 }} />} label="Importing" />
      )}
      <Button variant="contained" onClick={onImport} disabled={disabled}>
        {busy ? 'Starting…' : 'Import'}
      </Button>
    </Stack>
  )
}
