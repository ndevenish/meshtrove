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
import Inventory2Icon from '@mui/icons-material/Inventory2'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useNavigate } from 'react-router-dom'

import {
  api,
  variantLabel,
  type BundleRef,
  type FileRecord,
  type Job,
  type ModelDetail,
  type VariantDetail,
} from '../api'
import { FileTree } from './VariantSection'

const isActive = (j: Job) => j.status === 'queued' || j.status === 'running'

/// The "unsorted" bucket: files owned directly by a model (from importing an
/// archive), plus the tools to classify them — set each file's kind, move a
/// selection into a variant (new or existing), or delete files.
export default function UnsortedSection({
  model,
  canEdit,
  editing = false,
  onChange,
}: {
  model: ModelDetail
  canEdit: boolean
  /** Edit mode: destructive controls (delete a file) only appear here. */
  editing?: boolean
  onChange: () => void
}) {
  const queryClient = useQueryClient()
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [showArchive, setShowArchive] = useState(false)
  const [moveOpen, setMoveOpen] = useState(false)
  const [toBundleOpen, setToBundleOpen] = useState(false)

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
            {model.bundles.length > 0 && (
              <Button
                size="small"
                startIcon={<Inventory2Icon />}
                disabled={selected.size === 0}
                onClick={() => setToBundleOpen(true)}
              >
                Move to bundle
              </Button>
            )}
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
            onKindChange={editing ? setKind : undefined}
            onDelete={editing ? removeFile : undefined}
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

      <MoveToBundleDialog
        open={toBundleOpen}
        bundles={model.bundles}
        count={selected.size}
        onClose={() => setToBundleOpen(false)}
        onConfirm={async (bundleId) => {
          await Promise.all([...selected].map((id) => api.updateFile(id, { bundle_id: bundleId })))
          await queryClient.invalidateQueries({ queryKey: ['model-files', model.id] })
          await queryClient.invalidateQueries({ queryKey: ['bundle-files', bundleId] })
          setSelected(new Set())
          setToBundleOpen(false)
          onChange()
        }}
      />
    </Box>
  )
}

/// Push the selected files up into a bundle the model belongs to, where they can
/// be carved into separate member models (splitting a too-big model).
function MoveToBundleDialog({
  open,
  bundles,
  count,
  onClose,
  onConfirm,
}: {
  open: boolean
  bundles: BundleRef[]
  count: number
  onClose: () => void
  onConfirm: (bundleId: string) => Promise<void>
}) {
  const navigate = useNavigate()
  const [bundleId, setBundleId] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  const [wasOpen, setWasOpen] = useState(false)
  if (open && !wasOpen) {
    setWasOpen(true)
    setBundleId(bundles[0]?.id ?? '')
    setError('')
  }
  if (!open && wasOpen) setWasOpen(false)

  return (
    <Dialog open={open} onClose={onClose} maxWidth="sm" fullWidth>
      <DialogTitle>Move {count} file(s) to a bundle</DialogTitle>
      <DialogContent>
        <Stack spacing={2} sx={{ mt: 1 }}>
          {error && <Alert severity="error">{error}</Alert>}
          {bundles.length > 1 && (
            <Select size="small" value={bundleId} onChange={(e) => setBundleId(e.target.value)}>
              {bundles.map((b) => (
                <MenuItem key={b.id} value={b.id}>
                  {b.name}
                </MenuItem>
              ))}
            </Select>
          )}
          <Typography variant="body2" color="text.secondary">
            Files move to the bundle's unsorted area, where you can split them into separate member
            models.
          </Typography>
        </Stack>
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose}>Cancel</Button>
        <Button
          variant="contained"
          disabled={busy || !bundleId}
          onClick={async () => {
            setBusy(true)
            setError('')
            try {
              await onConfirm(bundleId)
              navigate(`/bundles/${bundleId}`)
            } catch (e) {
              setError(e instanceof Error ? e.message : String(e))
            } finally {
              setBusy(false)
            }
          }}
        >
          Move &amp; open bundle
        </Button>
      </DialogActions>
    </Dialog>
  )
}

function matchesArchive(job: Job, archiveIds: Set<string>): boolean {
  const payload = job.payload as { archive_file_id?: string } | undefined
  return !!payload?.archive_file_id && archiveIds.has(payload.archive_file_id)
}

/// Move the selected files onto a variant: either an existing one or a new one
/// created inline from its tags. Tagging the new variant with a set that already
/// exists lands the files on that variant, which is the intent anyway.
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
  const { data: vocabulary } = useQuery({
    queryKey: ['variant-tags'],
    queryFn: () => api.variantTags(),
    enabled: open,
  })
  const [mode, setMode] = useState<'existing' | 'new'>(model.variants.length ? 'existing' : 'new')
  const [existingId, setExistingId] = useState('')
  const [name, setName] = useState('')
  const [tags, setTags] = useState<string[]>([])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  // Reset each time it opens.
  const [wasOpen, setWasOpen] = useState(false)
  if (open && !wasOpen) {
    setWasOpen(true)
    setMode(model.variants.length ? 'existing' : 'new')
    setExistingId(model.variants[0]?.id ?? '')
    setName('')
    setTags([])
    setError('')
  }
  if (!open && wasOpen) setWasOpen(false)

  const submit = async () => {
    setBusy(true)
    setError('')
    try {
      let variantId = existingId
      if (mode === 'new') {
        // Get-or-create: an existing variant with these tags is returned as-is.
        const variant: VariantDetail = await api.createVariant(model.id, {
          name: name.trim() || null,
          tags,
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

  // A new variant needs neither name nor tags: untagged is a valid variant.
  const canSubmit = mode === 'existing' ? !!existingId : true

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
                  {variantLabel(v)}
                </MenuItem>
              ))}
            </Select>
          ) : (
            <>
              <Autocomplete
                multiple
                freeSolo
                autoFocus
                size="small"
                options={(vocabulary ?? []).map((t) => t.name)}
                value={tags}
                onChange={(_, value) =>
                  setTags([...new Set(value.map((t) => t.trim()).filter(Boolean))])
                }
                renderInput={(params) => (
                  <TextField
                    {...params}
                    label="Variant tags"
                    placeholder="32mm, supported…"
                    helperText={
                      tags.length
                        ? 'These tags identify the variant.'
                        : "No tags — the model's untagged variant."
                    }
                  />
                )}
              />
              <TextField
                label="Name (optional)"
                size="small"
                value={name}
                onChange={(e) => setName(e.target.value)}
                placeholder="e.g. My merged remix"
              />
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
