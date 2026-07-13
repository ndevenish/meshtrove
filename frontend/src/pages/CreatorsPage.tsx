import { useState } from 'react'
import { Link, useSearchParams } from 'react-router-dom'
import {
  Container,
  Typography,
  Paper,
  Stack,
  Chip,
  Button,
  Dialog,
  DialogTitle,
  DialogContent,
  DialogActions,
  TextField,
  MenuItem,
  Box,
} from '@mui/material'
import AddIcon from '@mui/icons-material/Add'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import { api, type Creator } from '../api'
import { useAuth } from '../main'

export default function CreatorsPage() {
  const [params] = useSearchParams()
  const { user } = useAuth()
  const queryClient = useQueryClient()
  const q = params.get('q') ?? ''
  const [editing, setEditing] = useState<Creator | 'new' | null>(null)

  const { data: creators } = useQuery({
    queryKey: ['creators', q],
    queryFn: () => api.creators(q),
  })

  const canEdit = user && user.role !== 'viewer'

  return (
    <Container maxWidth="md" sx={{ py: 3 }}>
      <Stack direction="row" sx={{ alignItems: 'center', mb: 2 }}>
        <Typography variant="h5">Creators</Typography>
        <Box sx={{ flexGrow: 1 }} />
        {canEdit && (
          <Button variant="contained" startIcon={<AddIcon />} onClick={() => setEditing('new')}>
            New creator
          </Button>
        )}
      </Stack>
      <Stack spacing={1.5}>
        {(creators ?? []).map((creator) => (
          <Paper key={creator.id} variant="outlined" sx={{ p: 2 }}>
            <Stack sx={{ alignItems: 'center' }} direction="row" spacing={1.5}>
              <Typography sx={{ fontWeight: 600 }}>{creator.name}</Typography>
              <Chip label={creator.kind} size="small" variant="outlined" />
              <Typography variant="body2" color="text.secondary">
                <Link to={`/?q=${encodeURIComponent(creator.name)}`}>
                  {creator.model_count} model{creator.model_count === 1 ? '' : 's'}
                </Link>
              </Typography>
              <Box sx={{ flexGrow: 1 }} />
              {creator.url && (
                <Typography variant="body2">
                  <a href={creator.url} target="_blank" rel="noreferrer">
                    site
                  </a>
                </Typography>
              )}
              {canEdit && (
                <Button size="small" onClick={() => setEditing(creator)}>
                  Edit
                </Button>
              )}
            </Stack>
            {creator.notes && (
              <Typography variant="body2" color="text.secondary" sx={{ mt: 1 }}>
                {creator.notes}
              </Typography>
            )}
          </Paper>
        ))}
        {creators?.length === 0 && (
          <Typography color="text.secondary">No creators{q ? ` matching "${q}"` : ''}.</Typography>
        )}
      </Stack>
      <CreatorEditDialog
        key={editing === 'new' ? 'new' : (editing?.id ?? 'closed')}
        open={editing !== null}
        creator={editing === 'new' ? undefined : (editing ?? undefined)}
        onClose={() => setEditing(null)}
        onSaved={() => void queryClient.invalidateQueries({ queryKey: ['creators'] })}
      />
    </Container>
  )
}

function CreatorEditDialog({
  open,
  creator,
  onClose,
  onSaved,
}: {
  open: boolean
  creator?: Creator
  onClose: () => void
  onSaved: () => void
}) {
  const [name, setName] = useState(creator?.name ?? '')
  const [kind, setKind] = useState(creator?.kind ?? 'author')
  const [url, setUrl] = useState(creator?.url ?? '')
  const [notes, setNotes] = useState(creator?.notes ?? '')

  const submit = async () => {
    const body = { name, kind, url: url || null, notes: notes || null }
    if (creator) await api.updateCreator(creator.id, body)
    else await api.createCreator(body)
    onSaved()
    onClose()
  }

  return (
    <Dialog open={open} onClose={onClose} maxWidth="xs" fullWidth>
      <DialogTitle>{creator ? 'Edit creator' : 'New creator'}</DialogTitle>
      <DialogContent>
        <Stack spacing={2} sx={{ mt: 1 }}>
          <TextField
            label="Name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            autoFocus
          />
          <TextField
            select
            label="Kind"
            value={kind}
            onChange={(e) => setKind(e.target.value as Creator['kind'])}
          >
            <MenuItem value="author">Author</MenuItem>
            <MenuItem value="company">Company</MenuItem>
            <MenuItem value="site">Site</MenuItem>
          </TextField>
          <TextField label="URL" value={url} onChange={(e) => setUrl(e.target.value)} />
          <TextField
            label="Notes"
            value={notes}
            onChange={(e) => setNotes(e.target.value)}
            multiline
            minRows={2}
          />
        </Stack>
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="contained" onClick={submit} disabled={!name.trim()}>
          Save
        </Button>
      </DialogActions>
    </Dialog>
  )
}
