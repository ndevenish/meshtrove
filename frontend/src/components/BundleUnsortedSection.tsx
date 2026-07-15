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

import { api, type BundleDetail, type FileRecord, type Job } from '../api'
import { FileTree } from './VariantSection'

const isActive = (j: Job) => j.status === 'queued' || j.status === 'running'

function matchesArchive(job: Job, archiveIds: Set<string>): boolean {
  const payload = job.payload as { archive_file_id?: string } | undefined
  return !!payload?.archive_file_id && archiveIds.has(payload.archive_file_id)
}

/// A bundle's "unsorted" bucket: files from a dropped archive, to be carved into
/// member models. Mirrors the model-level UnsortedSection, but the move target
/// is a member model (new or existing) rather than a variant.
export default function BundleUnsortedSection({
  bundle,
  canEdit,
  editing = false,
  onChange,
}: {
  bundle: BundleDetail
  canEdit: boolean
  /** Edit mode: deleting a staged file only offered here. */
  editing?: boolean
  onChange: () => void
}) {
  const queryClient = useQueryClient()
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [showArchive, setShowArchive] = useState(false)
  const [moveOpen, setMoveOpen] = useState(false)

  const { data: files } = useQuery({
    queryKey: ['bundle-files', bundle.id],
    queryFn: () => api.bundleFiles(bundle.id),
  })

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

  const prevImporting = useRef(false)
  useEffect(() => {
    if (prevImporting.current && !importing) {
      void queryClient.invalidateQueries({ queryKey: ['bundle-files', bundle.id] })
      onChange()
    }
    prevImporting.current = importing
  }, [importing, bundle.id, onChange, queryClient])

  const visible = (files ?? []).filter((f) => showArchive || f.kind !== 'archive')
  const archiveCount =
    (files ?? []).length - (files ?? []).filter((f) => f.kind !== 'archive').length

  if (!canEdit) return null
  if (!importing && !failed && visible.length === 0) return null

  const invalidate = () => queryClient.invalidateQueries({ queryKey: ['bundle-files', bundle.id] })

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
            {visible.length} to sort into models
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
          {/* Classifying and moving files is edit-mode work; browsing a bundle
              shows the unsorted files read-only (still downloadable). */}
          {editing && (
            <Stack direction="row" spacing={1} sx={{ mb: 1, alignItems: 'center' }}>
              <Button
                size="small"
                startIcon={<DriveFileMoveIcon />}
                variant="contained"
                disabled={selected.size === 0}
                onClick={() => setMoveOpen(true)}
              >
                Move {selected.size || ''} to model
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
                    selected.size === visible.length
                      ? new Set()
                      : new Set(visible.map((f) => f.id)),
                  )
                }
              >
                {selected.size === visible.length ? 'Clear' : 'Select all'}
              </Button>
            </Stack>
          )}

          <FileTree
            files={visible}
            selectable={editing}
            selected={selected}
            onToggle={toggle}
            onKindChange={editing ? setKind : undefined}
            onDelete={editing ? removeFile : undefined}
          />
        </>
      )}

      <MoveToModelDialog
        open={moveOpen}
        bundle={bundle}
        count={selected.size}
        onClose={() => setMoveOpen(false)}
        onConfirm={async (modelId) => {
          await Promise.all([...selected].map((id) => api.updateFile(id, { model_id: modelId })))
          await queryClient.invalidateQueries({ queryKey: ['bundle-files', bundle.id] })
          await queryClient.invalidateQueries({ queryKey: ['bundle', bundle.id] })
          await queryClient.invalidateQueries({ queryKey: ['model-files', modelId] })
          setSelected(new Set())
          setMoveOpen(false)
          onChange()
        }}
      />
    </Box>
  )
}

/// Move the selected bundle files into a member model — an existing one, or a
/// new member model created inline (created + added to the bundle).
function MoveToModelDialog({
  open,
  bundle,
  count,
  onClose,
  onConfirm,
}: {
  open: boolean
  bundle: BundleDetail
  count: number
  onClose: () => void
  onConfirm: (modelId: string) => Promise<void>
}) {
  const [mode, setMode] = useState<'new' | 'existing'>(bundle.models.length ? 'existing' : 'new')
  const [existingId, setExistingId] = useState('')
  const [name, setName] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  const [wasOpen, setWasOpen] = useState(false)
  if (open && !wasOpen) {
    setWasOpen(true)
    setMode(bundle.models.length ? 'existing' : 'new')
    setExistingId(bundle.models[0]?.id ?? '')
    setName('')
    setError('')
  }
  if (!open && wasOpen) setWasOpen(false)

  const submit = async () => {
    setBusy(true)
    setError('')
    try {
      let modelId = existingId
      if (mode === 'new') {
        const model = await api.createModel({ name: name.trim() })
        await api.addModelToBundle(bundle.id, model.id)
        modelId = model.id
      }
      await onConfirm(modelId)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }

  const canSubmit = mode === 'existing' ? !!existingId : name.trim().length > 0

  return (
    <Dialog open={open} onClose={onClose} maxWidth="sm" fullWidth>
      <DialogTitle>Move {count} file(s) to a member model</DialogTitle>
      <DialogContent>
        <Stack spacing={2} sx={{ mt: 1 }}>
          {error && <Alert severity="error">{error}</Alert>}
          <ToggleButtonGroup
            size="small"
            exclusive
            value={mode}
            onChange={(_, v) => v && setMode(v)}
          >
            <ToggleButton value="existing" disabled={!bundle.models.length}>
              Existing model
            </ToggleButton>
            <ToggleButton value="new">New model</ToggleButton>
          </ToggleButtonGroup>

          {mode === 'existing' ? (
            <Select
              size="small"
              value={existingId}
              onChange={(e) => setExistingId(e.target.value)}
              displayEmpty
            >
              {bundle.models.map((m) => (
                <MenuItem key={m.id} value={m.id}>
                  {m.name}
                </MenuItem>
              ))}
            </Select>
          ) : (
            <TextField
              label="New model name"
              size="small"
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder="e.g. Gynosphinx"
              autoFocus
            />
          )}
          <Typography variant="body2" color="text.secondary">
            Files land in that model's unsorted bucket, where you sort them into variants.
          </Typography>
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
