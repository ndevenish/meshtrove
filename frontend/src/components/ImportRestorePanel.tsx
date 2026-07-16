import { useState } from 'react'
import {
  Button,
  Typography,
  Stack,
  Alert,
  Chip,
  Checkbox,
  FormControlLabel,
  Divider,
  CircularProgress,
  Box,
} from '@mui/material'
import Inventory2Icon from '@mui/icons-material/Inventory2'
import ViewInArIcon from '@mui/icons-material/ViewInAr'
import { useQuery } from '@tanstack/react-query'

import { api, formatBytes, type RestoreEntity, type RestoreSummary } from '../api'

/// Shown on the Import page when a dropped archive turns out to be a MeshTrove
/// export: preview what it holds (flagging entities already present), then
/// restore. Existing entities are skipped unless ticked for a fresh copy.
export default function ImportRestorePanel({
  importId,
  onImported,
}: {
  importId: string
  /** fired after a successful restore, with the summary the server returned */
  onImported: (summary: RestoreSummary) => void
}) {
  const [fresh, setFresh] = useState<Set<string>>(new Set())
  const [committing, setCommitting] = useState(false)
  const [error, setError] = useState('')

  const { data: preview, isLoading } = useQuery({
    queryKey: ['restore-preview', importId],
    queryFn: () => api.restorePreview(importId),
  })

  const toggleFresh = (id: string) => {
    setFresh((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  const commit = async () => {
    setError('')
    setCommitting(true)
    try {
      const summary = await api.restoreCommit(importId, [...fresh])
      onImported(summary)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
      setCommitting(false)
    }
  }

  if (isLoading || !preview) {
    return (
      <Stack spacing={2} sx={{ alignItems: 'center', py: 4 }}>
        <CircularProgress />
      </Stack>
    )
  }

  const nothing = preview.models.length === 0 && preview.bundles.length === 0

  return (
    <Stack spacing={2}>
      {error && <Alert severity="error">{error}</Alert>}
      <Typography variant="body2" color="text.secondary">
        This is a MeshTrove export from {new Date(preview.exported_at).toLocaleString()} —{' '}
        {preview.blob_count} file(s), {formatBytes(preview.total_size)}. Entities already here are
        skipped unless you ask for a fresh copy.
      </Typography>

      {preview.models.length > 0 && (
        <EntityList
          title="Models"
          icon={<ViewInArIcon fontSize="small" />}
          entities={preview.models}
          fresh={fresh}
          onToggle={toggleFresh}
        />
      )}
      {preview.bundles.length > 0 && (
        <EntityList
          title="Bundles"
          icon={<Inventory2Icon fontSize="small" />}
          entities={preview.bundles}
          fresh={fresh}
          onToggle={toggleFresh}
        />
      )}

      <Box>
        <Button
          variant="contained"
          size="large"
          onClick={() => void commit()}
          disabled={committing || nothing}
          startIcon={committing ? <CircularProgress size={16} color="inherit" /> : undefined}
        >
          {committing ? 'Restoring…' : 'Restore'}
        </Button>
      </Box>
    </Stack>
  )
}

function EntityList({
  title,
  icon,
  entities,
  fresh,
  onToggle,
}: {
  title: string
  icon: React.ReactNode
  entities: RestoreEntity[]
  fresh: Set<string>
  onToggle: (id: string) => void
}) {
  return (
    <Box>
      <Stack direction="row" spacing={1} sx={{ alignItems: 'center', mb: 0.5 }}>
        {icon}
        <Typography variant="subtitle2">
          {title} ({entities.length})
        </Typography>
      </Stack>
      <Divider sx={{ mb: 1 }} />
      <Stack spacing={0.5}>
        {entities.map((e) => (
          <Stack
            key={e.id}
            direction="row"
            spacing={1}
            sx={{ alignItems: 'center', flexWrap: 'wrap' }}
          >
            <Typography sx={{ flexGrow: 1, minWidth: 0 }} noWrap>
              {e.name}
              {e.members !== undefined && (
                <Typography component="span" variant="body2" color="text.secondary">
                  {' '}
                  · {e.members} model(s)
                </Typography>
              )}
            </Typography>
            {e.exists ? (
              <>
                <Chip label="already here" size="small" color="warning" variant="outlined" />
                <FormControlLabel
                  sx={{ mr: 0 }}
                  control={
                    <Checkbox
                      size="small"
                      checked={fresh.has(e.id)}
                      onChange={() => onToggle(e.id)}
                    />
                  }
                  label={<Typography variant="body2">fresh copy</Typography>}
                />
              </>
            ) : (
              <Chip label="new" size="small" color="success" variant="outlined" />
            )}
          </Stack>
        ))}
      </Stack>
    </Box>
  )
}
