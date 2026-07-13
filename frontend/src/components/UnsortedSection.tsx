import { useEffect, useMemo, useRef, useState } from 'react'
import {
  Box,
  Typography,
  Stack,
  Button,
  LinearProgress,
  Alert,
  Dialog,
  DialogTitle,
  DialogContent,
  DialogActions,
  TextField,
  Autocomplete,
  Select,
  MenuItem,
  FormControlLabel,
  Switch,
  ToggleButton,
  ToggleButtonGroup,
} from '@mui/material'
import DeleteIcon from '@mui/icons-material/Delete'
import DriveFileMoveIcon from '@mui/icons-material/DriveFileMove'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import { api, type FileRecord, type Job, type ModelDetail, type VariantDetail } from '../api'
import { FileTree } from './VariantSection'

const isActive = (j: Job) => j.status === 'queued' || j.status === 'running'

/// The "unsorted" bucket: files owned directly by a model (from importing an
/// archive), plus the tools to classify them — set each file's kind, move a
/// selection into a variant (new or existing), or delete files.
export default function UnsortedSection({
  model,
  canEdit,
  onChange,
}: {
  model: ModelDetail
  canEdit: boolean
  onChange: () => void
}) {
  const queryClient = useQueryClient()
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [showArchive, setShowArchive] = useState(false)
  const [moveOpen, setMoveOpen] = useState(false)

  const { data: files } = useQuery({
    queryKey: ['model-files', model.id],
    queryFn: () => api.modelFiles(model.id),
  })

  // Scope import jobs to this model's own archive file rows via the job payload.
  const archiveIds = useMemo(
    () => new Set((files ?? []).filter((f) => f.kind === 'archive').map((f) => f.id)),
    [files],
  )
  const { data: jobs } = useQuery({
    queryKey: ['jobs', 'all'],
    queryFn: () => api.jobs(''),
    refetchInterval: (query) => {
      const active = (query.state.data ?? []).some(
        (j) => j.kind === 'import_archive' && isActive(j) && matchesArchive(j, archiveIds),
      )
      return active ? 1500 : false
    },
  })
  const importJobs = (jobs ?? []).filter(
    (j) => j.kind === 'import_archive' && matchesArchive(j, archiveIds),
  )
  const importing = importJobs.some(isActive)
  const failed = importJobs.find((j) => j.status === 'failed')

  // When an import finishes, refresh the file list and the model (variant counts).
  const prevImporting = useRef(false)
  useEffect(() => {
    if (prevImporting.current && !importing) {
      void queryClient.invalidateQueries({ queryKey: ['model-files', model.id] })
      onChange()
    }
    prevImporting.current = importing
  }, [importing, model.id, onChange, queryClient])

  const visible = (files ?? []).filter((f) => showArchive || f.kind !== 'archive')
  const archiveCount =
    (files ?? []).length - (files ?? []).filter((f) => f.kind !== 'archive').length

  if (!canEdit) return null
  if (!importing && !failed && visible.length === 0) return null

  const invalidate = () => queryClient.invalidateQueries({ queryKey: ['model-files', model.id] })

  const toggle = (id: string) =>
    setSelected((prev) => {
      const next = new Set(prev)
      next.has(id) ? next.delete(id) : next.add(id)
      return next
    })

  const setKind = async (id: string, kind: FileRecord['kind']) => {
    await api.updateFile(id, { kind })
    await invalidate()
  }

  const removeFile = async (id: string) => {
    await api.deleteFile(id)
    setSelected((prev) => {
      const next = new Set(prev)
      next.delete(id)
      return next
    })
    await invalidate()
  }

  const removeSelected = async () => {
    if (!confirm(`Delete ${selected.size} file(s)?`)) return
    await Promise.all([...selected].map((id) => api.deleteFile(id)))
    setSelected(new Set())
    await invalidate()
  }

  return (
    <Box sx={{ mb: 3 }}>
      <Stack direction="row" sx={{ alignItems: 'center', mb: 1 }} spacing={1}>
        <Typography variant="h6">Unsorted files</Typography>
        {visible.length > 0 && (
          <Typography variant="body2" color="text.secondary">
            {visible.length} to classify
          </Typography>
        )}
        <Box sx={{ flexGrow: 1 }} />
        {archiveCount > 0 && (
          <FormControlLabel
            control={
              <Switch
                size="small"
                checked={showArchive}
                onChange={(e) => setShowArchive(e.target.checked)}
              />
            }
            label={`show archive (${archiveCount})`}
            slotProps={{ typography: { variant: 'body2' } }}
          />
        )}
      </Stack>

      {importing && (
        <Box sx={{ mb: 1.5 }}>
          <LinearProgress />
          <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5 }}>
            Unpacking archive…
          </Typography>
        </Box>
      )}

      {failed && !importing && (
        <Alert
          severity="error"
          sx={{ mb: 1.5 }}
          action={
            <Button
              color="inherit"
              size="small"
              onClick={async () => {
                await api.retryJob(failed.id)
                await queryClient.invalidateQueries({ queryKey: ['jobs', 'all'] })
              }}
            >
              Retry
            </Button>
          }
        >
          Import failed: {failed.last_error ?? 'unknown error'}
        </Alert>
      )}

      {visible.length > 0 && (
        <>
          <Stack direction="row" spacing={1} sx={{ mb: 1, alignItems: 'center' }}>
            <Button
              size="small"
              startIcon={<DriveFileMoveIcon />}
              variant="contained"
              disabled={selected.size === 0}
              onClick={() => setMoveOpen(true)}
            >
              Move {selected.size || ''} to variant
            </Button>
            <Button
              size="small"
              color="error"
              startIcon={<DeleteIcon />}
              disabled={selected.size === 0}
              onClick={removeSelected}
            >
              Delete
            </Button>
            <Box sx={{ flexGrow: 1 }} />
            <Button
              size="small"
              onClick={() =>
                setSelected(
                  selected.size === visible.length ? new Set() : new Set(visible.map((f) => f.id)),
                )
              }
            >
              {selected.size === visible.length ? 'Clear' : 'Select all'}
            </Button>
          </Stack>

          <FileTree
            files={visible}
            selectable
            selected={selected}
            onToggle={toggle}
            onKindChange={setKind}
            onDelete={removeFile}
          />
        </>
      )}

      <MoveToVariantDialog
        open={moveOpen}
        model={model}
        count={selected.size}
        onClose={() => setMoveOpen(false)}
        onConfirm={async (variantId) => {
          await Promise.all(
            [...selected].map((id) => api.updateFile(id, { variant_id: variantId })),
          )
          await queryClient.invalidateQueries({ queryKey: ['model-files', model.id] })
          await queryClient.invalidateQueries({ queryKey: ['variant-files', variantId] })
          setSelected(new Set())
          setMoveOpen(false)
          onChange()
        }}
      />
    </Box>
  )
}

function matchesArchive(job: Job, archiveIds: Set<string>): boolean {
  const payload = job.payload as { archive_file_id?: string } | undefined
  return !!payload?.archive_file_id && archiveIds.has(payload.archive_file_id)
}

/// Move the selected files onto a variant: either an existing one or a new
/// variant created inline with its scale/support axis options.
function MoveToVariantDialog({
  open,
  model,
  count,
  onClose,
  onConfirm,
}: {
  open: boolean
  model: ModelDetail
  count: number
  onClose: () => void
  onConfirm: (variantId: string) => Promise<void>
}) {
  const { data: axes } = useQuery({ queryKey: ['axes'], queryFn: () => api.axes(), enabled: open })
  const [mode, setMode] = useState<'existing' | 'new'>(model.variants.length ? 'existing' : 'new')
  const [existingId, setExistingId] = useState('')
  const [name, setName] = useState('')
  const [options, setOptions] = useState<Record<string, string>>({})
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  // Reset each time it opens.
  const [wasOpen, setWasOpen] = useState(false)
  if (open && !wasOpen) {
    setWasOpen(true)
    setMode(model.variants.length ? 'existing' : 'new')
    setExistingId(model.variants[0]?.id ?? '')
    setName('')
    setOptions({})
    setError('')
  }
  if (!open && wasOpen) setWasOpen(false)

  const submit = async () => {
    setBusy(true)
    setError('')
    try {
      let variantId = existingId
      if (mode === 'new') {
        const cleaned = Object.fromEntries(Object.entries(options).filter(([, v]) => v.trim()))
        const variant: VariantDetail = await api.createVariant(model.id, {
          name: name.trim(),
          options: cleaned,
        })
        variantId = variant.id
      }
      await onConfirm(variantId)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }

  const canSubmit = mode === 'existing' ? !!existingId : name.trim().length > 0

  return (
    <Dialog open={open} onClose={onClose} maxWidth="sm" fullWidth>
      <DialogTitle>Move {count} file(s) to a variant</DialogTitle>
      <DialogContent>
        <Stack spacing={2} sx={{ mt: 1 }}>
          {error && <Alert severity="error">{error}</Alert>}
          <ToggleButtonGroup
            size="small"
            exclusive
            value={mode}
            onChange={(_, v) => v && setMode(v)}
          >
            <ToggleButton value="existing" disabled={!model.variants.length}>
              Existing variant
            </ToggleButton>
            <ToggleButton value="new">New variant</ToggleButton>
          </ToggleButtonGroup>

          {mode === 'existing' ? (
            <Select
              size="small"
              value={existingId}
              onChange={(e) => setExistingId(e.target.value)}
              displayEmpty
            >
              {model.variants.map((v) => (
                <MenuItem key={v.id} value={v.id}>
                  {v.name}
                </MenuItem>
              ))}
            </Select>
          ) : (
            <>
              <TextField
                label="Variant name"
                size="small"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="e.g. Supported, 32mm Lychee"
                autoFocus
              />
              {(axes ?? []).map((axis) => (
                <Autocomplete
                  key={axis.id}
                  freeSolo
                  options={axis.options.map((o) => o.value)}
                  value={options[axis.name] ?? ''}
                  onInputChange={(_, value) =>
                    setOptions((prev) => ({ ...prev, [axis.name]: value }))
                  }
                  renderInput={(params) => <TextField {...params} label={axis.name} size="small" />}
                />
              ))}
            </>
          )}
        </Stack>
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="contained" onClick={submit} disabled={busy || !canSubmit}>
          Move
        </Button>
      </DialogActions>
    </Dialog>
  )
}
