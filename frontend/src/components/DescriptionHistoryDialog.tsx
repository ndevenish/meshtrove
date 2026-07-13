import { useState } from 'react'
import {
  Box,
  Dialog,
  DialogTitle,
  DialogContent,
  DialogActions,
  Button,
  Stack,
  Typography,
  Paper,
  Chip,
  TextField,
} from '@mui/material'
import ReactMarkdown from 'react-markdown'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import { api, type ModelDetail } from '../api'

/// Full revision history of a model's markdown description: label revisions
/// ("v1", "v2"), and restore an old revision by re-saving it as the newest.
export default function DescriptionHistoryDialog({
  open,
  onClose,
  model,
  canEdit,
  onChange,
}: {
  open: boolean
  onClose: () => void
  model: ModelDetail
  canEdit: boolean
  onChange: () => void
}) {
  const queryClient = useQueryClient()
  const [labelDrafts, setLabelDrafts] = useState<Record<string, string>>({})

  const { data: revisions } = useQuery({
    queryKey: ['revisions', model.id],
    queryFn: () => api.revisions(model.id),
    enabled: open,
  })

  const refresh = async () => {
    await queryClient.invalidateQueries({ queryKey: ['revisions', model.id] })
    onChange()
  }

  return (
    <Dialog open={open} onClose={onClose} maxWidth="md" fullWidth>
      <DialogTitle>Description history — {model.name}</DialogTitle>
      <DialogContent>
        <Stack spacing={2} sx={{ mt: 1 }}>
          {(revisions ?? []).map((revision, index) => (
            <Paper key={revision.id} variant="outlined" sx={{ p: 2 }}>
              <Stack direction="row" spacing={1} sx={{ alignItems: 'center', mb: 1 }}>
                {index === 0 && <Chip label="current" color="primary" size="small" />}
                {revision.label && <Chip label={revision.label} size="small" />}
                <Typography variant="body2" color="text.secondary">
                  {new Date(revision.created_at).toLocaleString()} by {revision.author}
                </Typography>
                <Stack direction="row" spacing={1} sx={{ alignItems: 'center', ml: 'auto' }}>
                  {canEdit && (
                    <>
                      <TextField
                        size="small"
                        placeholder="label e.g. v1"
                        value={labelDrafts[revision.id] ?? revision.label ?? ''}
                        onChange={(e) =>
                          setLabelDrafts((d) => ({ ...d, [revision.id]: e.target.value }))
                        }
                        sx={{ width: 130 }}
                      />
                      <Button
                        size="small"
                        onClick={async () => {
                          const label = (labelDrafts[revision.id] ?? '').trim()
                          await api.labelRevision(model.id, revision.id, label || null)
                          await refresh()
                        }}
                      >
                        Set label
                      </Button>
                      {index !== 0 && (
                        <Button
                          size="small"
                          onClick={async () => {
                            await api.updateDescription(model.id, revision.body_md)
                            await refresh()
                          }}
                        >
                          Restore
                        </Button>
                      )}
                    </>
                  )}
                </Stack>
              </Stack>
              <Box sx={{ '& p': { my: 0.5 } }}>
                <ReactMarkdown>{revision.body_md}</ReactMarkdown>
              </Box>
            </Paper>
          ))}
          {revisions?.length === 0 && (
            <Typography color="text.secondary">No description revisions yet.</Typography>
          )}
        </Stack>
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose}>Close</Button>
      </DialogActions>
    </Dialog>
  )
}
