import { useMemo, useState } from 'react'
import {
  Alert,
  Box,
  Chip,
  Paper,
  Switch,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableRow,
  TextField,
  Typography,
} from '@mui/material'
import { useMutation, useQuery, useQueryClient } from '@tanstack/react-query'

import { api, type Tag } from '../api'
import { useAuth } from '../main'

/// Admin-only: the model/bundle tag vocabulary, with a per-tag toggle that hides
/// a tag from browsing and search. A hidden tag stays on its models — hiding is
/// reversible and loses nothing — but drops out of the filter sidebar, model and
/// bundle cards and detail pages, and the full-text search index.
export default function TagsPanel() {
  const { user } = useAuth()
  const queryClient = useQueryClient()
  const [filter, setFilter] = useState('')
  const [error, setError] = useState('')

  const { data: tags } = useQuery({
    queryKey: ['tags', 'manage'],
    queryFn: () => api.manageTags(),
    enabled: user?.role === 'admin',
  })

  const toggle = useMutation({
    mutationFn: ({ id, hidden }: { id: string; hidden: boolean }) => api.setTagHidden(id, hidden),
    onError: (e) => setError(e instanceof Error ? e.message : String(e)),
    onSuccess: () => {
      setError('')
      // Every browsing surface (sidebar counts, cards, detail, search) may now
      // differ — drop the whole cache rather than surgically patch each query.
      void queryClient.invalidateQueries()
    },
  })

  const shown = useMemo(() => {
    const q = filter.trim().toLowerCase()
    return (tags ?? []).filter((t) => !q || t.name.toLowerCase().includes(q))
  }, [tags, filter])

  const hiddenCount = (tags ?? []).filter((t) => t.hidden).length

  return (
    <Paper variant="outlined" sx={{ p: 3, mt: 3 }}>
      <Typography variant="h6" sx={{ mb: 0.5 }}>
        Tags
      </Typography>
      <Typography variant="body2" color="text.secondary" sx={{ mb: 2 }}>
        Every model/bundle tag. Hiding a tag removes it from the filter sidebar, from model and
        bundle cards and pages, and from search — everywhere a visitor browses. It stays on its
        models, so hiding is reversible and loses nothing.
        {hiddenCount > 0 && ` ${hiddenCount} currently hidden.`}
      </Typography>
      {error && (
        <Alert severity="error" sx={{ mb: 2 }} onClose={() => setError('')}>
          {error}
        </Alert>
      )}
      {(tags ?? []).length > 0 && (
        <TextField
          size="small"
          fullWidth
          placeholder="Filter tags…"
          value={filter}
          onChange={(e) => setFilter(e.target.value)}
          sx={{ mb: 2, maxWidth: 320 }}
        />
      )}
      <Box sx={{ maxHeight: 420, overflowY: 'auto' }}>
        <Table size="small" stickyHeader>
          <TableHead>
            <TableRow>
              <TableCell>Tag</TableCell>
              <TableCell sx={{ width: 90 }} align="right">
                Models
              </TableCell>
              <TableCell sx={{ width: 100 }} align="center">
                Hidden
              </TableCell>
            </TableRow>
          </TableHead>
          <TableBody>
            {shown.map((tag: Tag) => (
              <TableRow key={tag.id}>
                <TableCell>
                  {tag.name}
                  {tag.hidden && <Chip size="small" label="hidden" sx={{ ml: 1 }} />}
                </TableCell>
                <TableCell align="right">{tag.model_count}</TableCell>
                <TableCell align="center">
                  <Switch
                    size="small"
                    checked={tag.hidden}
                    onChange={(e) => toggle.mutate({ id: tag.id, hidden: e.target.checked })}
                  />
                </TableCell>
              </TableRow>
            ))}
            {shown.length === 0 && (
              <TableRow>
                <TableCell colSpan={3}>
                  <Typography color="text.secondary">
                    {(tags ?? []).length === 0 ? 'No tags yet.' : 'No tags match that filter.'}
                  </Typography>
                </TableCell>
              </TableRow>
            )}
          </TableBody>
        </Table>
      </Box>
    </Paper>
  )
}
