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
  MenuItem,
  TextField,
  ListSubheader,
} from '@mui/material'
import TuneIcon from '@mui/icons-material/Tune'
import Inventory2Icon from '@mui/icons-material/Inventory2'
import ViewInArIcon from '@mui/icons-material/ViewInAr'
import { useQuery } from '@tanstack/react-query'

import {
  api,
  formatBytes,
  type CustomFieldMapping,
  type RestoreCustomField,
  type RestoreEntity,
  type RestoreLocalField,
  type RestoreSummary,
} from '../api'

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
  // Per exported custom field, the choice encoded as one select value:
  // 'skip', 'create', or `existing:<local field id>`. Unset rows fall back to
  // the server's suggestion, which is also what the select shows.
  const [fieldChoice, setFieldChoice] = useState<Record<string, string>>({})
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

  /// The select value for one exported field: the user's choice if they made
  /// one, else the server's suggestion rendered the same way.
  const choiceFor = (f: RestoreCustomField): string =>
    fieldChoice[f.id] ?? (f.suggested_field_id ? `existing:${f.suggested_field_id}` : 'create')

  /// Only the rows the user actually changed need sending — the server applies
  /// the same suggestion to the rest — but sending every row keeps what was on
  /// screen and what happens identical even if the vocabulary shifts under us.
  const mappings = (): Record<string, CustomFieldMapping> =>
    Object.fromEntries(
      (preview?.custom_fields ?? []).map((f) => {
        const value = choiceFor(f)
        if (value === 'skip') return [f.id, { action: 'skip' } as CustomFieldMapping]
        if (value === 'create') return [f.id, { action: 'create' } as CustomFieldMapping]
        return [
          f.id,
          { action: 'existing', field_id: value.slice('existing:'.length) } as CustomFieldMapping,
        ]
      }),
    )

  const commit = async () => {
    setError('')
    setCommitting(true)
    try {
      const summary = await api.restoreCommit(importId, [...fresh], mappings())
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

  // A skipped entity keeps everything it has, custom fields included — so any
  // values the archive carries for it go nowhere. That is the intended
  // behaviour, but it is invisible unless the screen says so. Fields the user
  // has explicitly set to Skip are already a deliberate choice and don't count
  // towards the warning.
  const anyFieldKept = preview.custom_fields.some((f) => choiceFor(f) !== 'skip')
  const skippedWithValues = anyFieldKept
    ? [...preview.models, ...preview.bundles].filter(
        (e) => e.exists && !fresh.has(e.id) && e.custom_field_values > 0,
      )
    : []
  const droppedValues = skippedWithValues.reduce((n, e) => n + e.custom_field_values, 0)

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
      {preview.custom_fields.length > 0 && (
        <CustomFieldMap
          fields={preview.custom_fields}
          local={preview.local_custom_fields}
          choice={choiceFor}
          onChange={(id, value) => setFieldChoice((prev) => ({ ...prev, [id]: value }))}
        />
      )}

      {droppedValues > 0 && (
        <Alert severity="warning">
          {skippedWithValues.length} of these are already here and will be skipped — a restore
          brings entities in, it doesn't drop metadata onto ones you already have. The{' '}
          <strong>{droppedValues} custom field value(s)</strong> the archive carries for them will
          not be applied. Tick <em>fresh copy</em> on the ones you want brought in with their
          values.
        </Alert>
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

/// The archive's custom field vocabulary against this instance's. Each exported
/// field is skipped, pointed at a field already here, or created — a choice,
/// because the vocabulary is an instance-wide admin setting and two instances
/// have no reason to agree on it.
function CustomFieldMap({
  fields,
  local,
  choice,
  onChange,
}: {
  fields: RestoreCustomField[]
  local: RestoreLocalField[]
  choice: (f: RestoreCustomField) => string
  onChange: (id: string, value: string) => void
}) {
  return (
    <Box>
      <Stack direction="row" spacing={1} sx={{ alignItems: 'center', mb: 0.5 }}>
        <TuneIcon fontSize="small" />
        <Typography variant="subtitle2">Custom fields ({fields.length})</Typography>
      </Stack>
      <Divider sx={{ mb: 1 }} />
      <Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>
        The archive brings its own field definitions. Point each one at a field you already have,
        create it here, or skip it and drop its values.
      </Typography>
      <Stack spacing={1.5}>
        {fields.map((f) => {
          const value = choice(f)
          const target = value.startsWith('existing:')
            ? local.find((l) => l.id === value.slice('existing:'.length))
            : undefined
          // Mapping onto a field of another kind is allowed — the user may know
          // better — but the values won't survive it, so say so.
          const mismatch = target && target.kind !== f.kind
          return (
            <Stack key={f.id} direction="row" spacing={1.5} sx={{ alignItems: 'flex-start' }}>
              <Box sx={{ flexGrow: 1, minWidth: 0 }}>
                <Typography noWrap>
                  {f.name}{' '}
                  <Typography component="span" variant="body2" color="text.secondary">
                    <code>{f.key}</code> · {f.kind} · {f.value_count} value(s)
                  </Typography>
                </Typography>
                {mismatch && (
                  <Typography variant="caption" color="warning.main">
                    {target.name} is a {target.kind} field — the values won't fit and will be
                    dropped.
                  </Typography>
                )}
              </Box>
              <TextField
                select
                size="small"
                sx={{ width: 280 }}
                value={value}
                onChange={(e) => onChange(f.id, e.target.value)}
              >
                <MenuItem value="create">Create &ldquo;{f.name}&rdquo;</MenuItem>
                <MenuItem value="skip">Skip — drop its values</MenuItem>
                {local.length > 0 && <ListSubheader>Use a field already here</ListSubheader>}
                {local.map((l) => (
                  <MenuItem key={l.id} value={`existing:${l.id}`}>
                    {l.name} ({l.kind})
                  </MenuItem>
                ))}
              </TextField>
            </Stack>
          )
        })}
      </Stack>
    </Box>
  )
}
