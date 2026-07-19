import { Link } from 'react-router-dom'
import {
  Container,
  Typography,
  Paper,
  Stack,
  Box,
  Button,
  Chip,
  CircularProgress,
} from '@mui/material'
import UploadFileIcon from '@mui/icons-material/UploadFile'
import { useQuery } from '@tanstack/react-query'

import { api } from '../api'
import DropboxPanel from '../components/DropboxPanel'

/// Everything dropped but not yet placed. Imports live only here — they are
/// deliberately absent from browse until they become a model or a bundle.
export default function ImportsPage() {
  const { data: staged, isLoading } = useQuery({
    queryKey: ['imports'],
    queryFn: () => api.imports(),
    refetchInterval: 3000,
  })

  return (
    <Container maxWidth="md" sx={{ py: 3 }}>
      <Typography variant="h5" sx={{ mb: 0.5 }}>
        Importing
      </Typography>
      <Typography color="text.secondary" sx={{ mb: 2 }}>
        Dropped archives wait here until you say what they are. They don't appear in browse yet.
      </Typography>

      {/* Admins can also pull straight from the server's dropbox folder, without
          the round trip through the browser. */}
      <DropboxPanel />

      {!isLoading && staged?.length === 0 && (
        <Paper variant="outlined" sx={{ p: 6, textAlign: 'center' }}>
          <UploadFileIcon sx={{ fontSize: 48, color: 'text.disabled' }} />
          <Typography color="text.secondary" sx={{ mt: 1 }}>
            Nothing importing. Drop a file anywhere to start.
          </Typography>
        </Paper>
      )}

      <Stack spacing={1}>
        {(staged ?? []).map((item) => (
          <Paper key={item.id} variant="outlined" sx={{ p: 2 }}>
            <Stack direction="row" spacing={2} sx={{ alignItems: 'center' }}>
              <Box sx={{ flexGrow: 1, minWidth: 0 }}>
                <Typography sx={{ fontWeight: 600 }} noWrap>
                  {item.name}
                </Typography>
                <Typography variant="body2" color="text.secondary">
                  {item.file_count} file{item.file_count === 1 ? '' : 's'}
                </Typography>
              </Box>
              {item.unpacking ? (
                <Chip
                  size="small"
                  icon={<CircularProgress size={12} sx={{ ml: 1 }} />}
                  label="Unpacking"
                />
              ) : item.partial ? (
                // A "keep unmatched files" carve already placed some of this
                // drop; what's staged is the remainder.
                <Chip size="small" color="warning" label="Partially imported" />
              ) : (
                <Chip size="small" color="primary" label="Ready to place" />
              )}
              <Button component={Link} to={`/imports/${item.id}`} variant="contained">
                Open
              </Button>
            </Stack>
          </Paper>
        ))}
      </Stack>
    </Container>
  )
}
