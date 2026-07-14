import { useMemo, useState } from 'react'
import {
  Box,
  Typography,
  Stack,
  Button,
  Accordion,
  AccordionSummary,
  AccordionDetails,
  Chip,
  IconButton,
  Tooltip,
  LinearProgress,
  Dialog,
  DialogTitle,
  DialogContent,
  DialogActions,
  TextField,
  Autocomplete,
  Alert,
  Checkbox,
  Select,
  MenuItem,
} from '@mui/material'
import ExpandMoreIcon from '@mui/icons-material/ExpandMore'
import AddIcon from '@mui/icons-material/Add'
import DownloadIcon from '@mui/icons-material/Download'
import UploadFileIcon from '@mui/icons-material/UploadFile'
import FolderIcon from '@mui/icons-material/Folder'
import DeleteIcon from '@mui/icons-material/Delete'
import PhotoCameraIcon from '@mui/icons-material/PhotoCamera'
import InsertDriveFileIcon from '@mui/icons-material/InsertDriveFile'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import {
  api,
  downloadUrl,
  formatBytes,
  variantLabel,
  type FileRecord,
  type ModelDetail,
  type VariantDetail,
} from '../api'

/// Wait for one job to settle. The render is the *job's* doing, so the picture is
/// there when the job says so — no inferring it from the shape of the queue.
/// Gives up after ~2 minutes and lets the caller refetch anyway; a render that
/// slow has bigger problems than a stale gallery.
async function waitForJob(jobId: number): Promise<void> {
  for (let i = 0; i < 120; i++) {
    const job = await api.job(jobId)
    if (job.status === 'succeeded' || job.status === 'failed' || job.status === 'cancelled') {
      return
    }
    await new Promise((resolve) => setTimeout(resolve, 1000))
  }
}

export default function VariantSection({
  model,
  canEdit,
  editing = false,
  onChange,
}: {
  model: ModelDetail
  canEdit: boolean
  /** Edit mode: deleting a file or a whole variant only offered here. */
  editing?: boolean
  onChange: () => void
}) {
  const [editingVariant, setEditingVariant] = useState<VariantDetail | 'new' | null>(null)

  return (
    <Box>
      <Stack direction="row" sx={{ alignItems: 'center', mb: 1 }}>
        <Typography variant="h6">Variants</Typography>
        <Box sx={{ flexGrow: 1 }} />
        {canEdit && (
          <Button startIcon={<AddIcon />} size="small" onClick={() => setEditingVariant('new')}>
            Add variant
          </Button>
        )}
      </Stack>
      {model.variants.length === 0 && (
        <Typography color="text.secondary" variant="body2">
          No variants yet{canEdit ? ' — add one to attach files' : ''}.
        </Typography>
      )}
      {model.variants.map((variant) => (
        <VariantRow
          key={variant.id}
          variant={variant}
          canEdit={canEdit}
          editing={editing}
          onChange={onChange}
          onEdit={() => setEditingVariant(variant)}
        />
      ))}
      <VariantEditDialog
        open={editingVariant !== null}
        variant={editingVariant === 'new' ? undefined : (editingVariant ?? undefined)}
        model={model}
        onClose={() => setEditingVariant(null)}
        onChange={onChange}
      />
    </Box>
  )
}

function VariantRow({
  variant,
  canEdit,
  editing,
  onChange,
  onEdit,
}: {
  variant: VariantDetail
  canEdit: boolean
  editing: boolean
  onChange: () => void
  onEdit: () => void
}) {
  const queryClient = useQueryClient()
  const [expanded, setExpanded] = useState(false)
  const [uploading, setUploading] = useState(false)
  const [rendering, setRendering] = useState(false)
  const { data: files } = useQuery({
    queryKey: ['variant-files', variant.id],
    queryFn: () => api.variantFiles(variant.id),
    enabled: expanded,
  })

  const upload = async (fileList: FileList) => {
    setUploading(true)
    try {
      const form = new FormData()
      for (const file of fileList) form.append('file', file)
      await api.uploadVariantFiles(variant.id, form)
      await queryClient.invalidateQueries({ queryKey: ['variant-files', variant.id] })
      onChange()
    } finally {
      setUploading(false)
    }
  }

  return (
    <Accordion
      variant="outlined"
      expanded={expanded}
      onChange={(_, next) => setExpanded(next)}
      disableGutters
    >
      <AccordionSummary expandIcon={<ExpandMoreIcon />}>
        <Stack direction="row" spacing={1.5} sx={{ alignItems: 'center', width: '100%', pr: 1 }}>
          {variant.name ? (
            <Typography sx={{ fontWeight: 600 }}>{variant.name}</Typography>
          ) : (
            variant.tags.length === 0 && (
              <Typography sx={{ fontWeight: 600, fontStyle: 'italic' }} color="text.secondary">
                Untagged
              </Typography>
            )
          )}
          {variant.tags.map((tag) => (
            <Chip key={tag} label={tag} size="small" variant="outlined" />
          ))}
          <Box sx={{ flexGrow: 1 }} />
          <Typography variant="body2" color="text.secondary">
            {variant.file_count} file{variant.file_count === 1 ? '' : 's'} ·{' '}
            {formatBytes(variant.total_size)}
          </Typography>
        </Stack>
      </AccordionSummary>
      <AccordionDetails>
        {variant.print_notes && (
          <Alert severity="info" icon={false} sx={{ mb: 1.5, whiteSpace: 'pre-wrap' }}>
            {variant.print_notes}
          </Alert>
        )}
        {(uploading || rendering) && <LinearProgress sx={{ mb: 1 }} />}
        {expanded && files && (
          <FileTree
            files={files}
            onDelete={
              editing
                ? async (fileId) => {
                    await api.deleteFile(fileId)
                    await queryClient.invalidateQueries({ queryKey: ['variant-files', variant.id] })
                    onChange()
                  }
                : undefined
            }
            onRender={
              canEdit
                ? async (fileId) => {
                    setRendering(true)
                    try {
                      // Wait for *this* job, then refetch. Inferring "my picture
                      // arrived" from a shared queue-watcher is how this broke
                      // twice: a render takes about a second, so whatever edge or
                      // poll the watcher is built on can be over before it looks.
                      // The job id is not ambiguous.
                      const { job_id } = await api.renderFile(fileId)
                      await waitForJob(job_id)
                    } finally {
                      setRendering(false)
                      await queryClient.invalidateQueries({ queryKey: ['model', variant.model_id] })
                      await queryClient.invalidateQueries({
                        queryKey: ['variant-files', variant.id],
                      })
                      await queryClient.invalidateQueries({ queryKey: ['jobs', 'all'] })
                    }
                  }
                : undefined
            }
          />
        )}
        {canEdit && (
          <Stack direction="row" spacing={1} sx={{ mt: 1.5 }}>
            <Button component="label" size="small" startIcon={<UploadFileIcon />}>
              Upload files (.zip auto-unpacks)
              <input
                hidden
                multiple
                type="file"
                onChange={(e) => {
                  if (e.target.files?.length) void upload(e.target.files)
                  e.target.value = ''
                }}
              />
            </Button>
            <Button size="small" onClick={onEdit}>
              Edit
            </Button>
            {/* Deleting a variant takes its files with it: edit mode only. */}
            {editing && (
              <Button
                size="small"
                color="error"
                onClick={async () => {
                  if (confirm(`Delete variant "${variantLabel(variant)}" and its files?`)) {
                    await api.deleteVariant(variant.id)
                    onChange()
                  }
                }}
              >
                Delete
              </Button>
            )}
          </Stack>
        )}
      </AccordionDetails>
    </Accordion>
  )
}

export const FILE_KINDS: FileRecord['kind'][] = [
  'model',
  'project',
  'raw',
  'document',
  'archive',
  'other',
]

/// Rebuild the kept folder structure from the flat path column. When the
/// optional editing props are supplied (used by the recategorisation UI), each
/// row gains a select checkbox, an inline kind selector, and a delete button.
export function FileTree({
  files,
  selectable = false,
  selected,
  onToggle,
  onKindChange,
  onDelete,
  onRender,
}: {
  files: FileRecord[]
  selectable?: boolean
  selected?: Set<string>
  onToggle?: (id: string) => void
  onKindChange?: (id: string, kind: FileRecord['kind']) => void
  onDelete?: (id: string) => void
  /** Force a preview render from this file; it joins the model's images. */
  onRender?: (id: string) => void
}) {
  const groups = useMemo(() => {
    const byDir = new Map<string, FileRecord[]>()
    for (const file of files) {
      const dir = file.path || '/'
      byDir.set(dir, [...(byDir.get(dir) ?? []), file])
    }
    return [...byDir.entries()].sort(([a], [b]) => a.localeCompare(b))
  }, [files])

  if (files.length === 0)
    return (
      <Typography variant="body2" color="text.secondary">
        No files yet.
      </Typography>
    )

  return (
    <Box>
      {groups.map(([dir, entries]) => (
        <Box key={dir} sx={{ mb: 1 }}>
          {dir !== '/' && (
            <Stack direction="row" spacing={0.75} sx={{ alignItems: 'center', mb: 0.25 }}>
              <FolderIcon sx={{ fontSize: 18, opacity: 0.6 }} />
              <Typography variant="body2" sx={{ fontWeight: 600 }}>
                {dir}
              </Typography>
            </Stack>
          )}
          {entries.map((file) => (
            <Stack
              key={file.id}
              direction="row"
              spacing={1}
              sx={{ alignItems: 'center', pl: dir !== '/' ? 3 : 0, py: 0.25 }}
            >
              {selectable && (
                <Checkbox
                  size="small"
                  sx={{ p: 0.25 }}
                  checked={selected?.has(file.id) ?? false}
                  onChange={() => onToggle?.(file.id)}
                />
              )}
              <InsertDriveFileIcon sx={{ fontSize: 16, opacity: 0.5 }} />
              <Typography variant="body2" sx={{ flexGrow: 1 }} noWrap>
                {file.filename}
              </Typography>
              {onKindChange ? (
                <Select
                  size="small"
                  variant="standard"
                  value={file.kind}
                  onChange={(e) => onKindChange(file.id, e.target.value as FileRecord['kind'])}
                  sx={{ minWidth: 96, fontSize: 13 }}
                >
                  {FILE_KINDS.map((k) => (
                    <MenuItem key={k} value={k} sx={{ fontSize: 13 }}>
                      {k}
                    </MenuItem>
                  ))}
                </Select>
              ) : (
                <Chip label={file.kind} size="small" variant="outlined" sx={{ height: 20 }} />
              )}
              <Typography variant="caption" color="text.secondary" sx={{ width: 64 }}>
                {formatBytes(file.size)}
              </Typography>
              <Tooltip title="Download">
                <IconButton size="small" component="a" href={downloadUrl(file.id)}>
                  <DownloadIcon sx={{ fontSize: 18 }} />
                </IconButton>
              </Tooltip>
              {/* The carve renders one picture per variant and picks the file
                  itself. This is the override for when it picked the base plate:
                  render *this* one, and it joins the model's images. */}
              {onRender && (file.kind === 'model' || file.kind === 'project') && (
                <Tooltip title="Render a preview from this file">
                  <IconButton size="small" onClick={() => onRender(file.id)}>
                    <PhotoCameraIcon sx={{ fontSize: 18 }} />
                  </IconButton>
                </Tooltip>
              )}
              {onDelete && (
                <Tooltip title="Delete file">
                  <IconButton size="small" color="error" onClick={() => onDelete(file.id)}>
                    <DeleteIcon sx={{ fontSize: 18 }} />
                  </IconButton>
                </Tooltip>
              )}
            </Stack>
          ))}
        </Box>
      ))}
    </Box>
  )
}

/// A variant is its tag set, so the tags are the substantive field here and the
/// name is just a label. Leaving the tags empty is legitimate — it addresses the
/// model's one anonymous variant.
function VariantEditDialog({
  open,
  variant,
  model,
  onClose,
  onChange,
}: {
  open: boolean
  variant?: VariantDetail
  model: ModelDetail
  onClose: () => void
  onChange: () => void
}) {
  const { data: vocabulary } = useQuery({
    queryKey: ['variant-tags'],
    queryFn: () => api.variantTags(),
  })
  const [name, setName] = useState(variant?.name ?? '')
  const [tags, setTags] = useState<string[]>(variant?.tags ?? [])
  const [notes, setNotes] = useState(variant?.print_notes ?? '')
  const [error, setError] = useState('')

  // Reset when target changes (dialog reused between add/edit)
  const [lastKey, setLastKey] = useState<string | null>(null)
  const key = variant?.id ?? 'new'
  if (open && key !== lastKey) {
    setLastKey(key)
    setName(variant?.name ?? '')
    setTags(variant?.tags ?? [])
    setNotes(variant?.print_notes ?? '')
    setError('')
  }

  // Same tags = same variant, so saving onto another variant's tag set folds
  // this one into it. Say so before it happens rather than after.
  const sameSet = (a: string[], b: string[]) =>
    a.length === b.length && [...a].sort().join(' ') === [...b].sort().join(' ')
  const collision = model.variants.find((v) => v.id !== variant?.id && sameSet(v.tags, tags))

  const submit = async () => {
    try {
      const body = { name: name.trim() || null, tags, print_notes: notes || null }
      if (variant) await api.updateVariant(variant.id, body)
      else await api.createVariant(model.id, body)
      onChange()
      onClose()
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }

  return (
    <Dialog open={open} onClose={onClose} maxWidth="sm" fullWidth>
      <DialogTitle>{variant ? 'Edit variant' : 'Add variant'}</DialogTitle>
      <DialogContent>
        <Stack spacing={2} sx={{ mt: 1 }}>
          {error && <Alert severity="error">{error}</Alert>}
          {collision && (
            <Alert severity="warning">
              {`These tags already identify "${variantLabel(collision)}"`}
              {variant
                ? '. Saving merges this variant into it, moving its files across.'
                : '. Its files will be added to that variant.'}
            </Alert>
          )}
          <Autocomplete
            multiple
            freeSolo
            autoFocus
            options={(vocabulary ?? []).map((t) => t.name)}
            value={tags}
            onChange={(_, value) =>
              setTags([...new Set(value.map((t) => t.trim()).filter(Boolean))])
            }
            renderInput={(params) => (
              <TextField
                {...params}
                label="Variant tags"
                placeholder="32mm, supported, lychee…"
                helperText={
                  tags.length
                    ? 'These tags identify the variant. New ones are created as you type.'
                    : "No tags — this is the model's single untagged variant."
                }
              />
            )}
          />
          <TextField
            label="Name (optional)"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="e.g. My merged remix"
            helperText="A display label only; the tags above are what separate variants."
          />
          <TextField
            label="Print notes"
            value={notes}
            onChange={(e) => setNotes(e.target.value)}
            multiline
            minRows={3}
            placeholder="Resin, exposure, orientation, supports…"
          />
        </Stack>
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="contained" onClick={submit}>
          {collision ? 'Merge' : variant ? 'Save' : 'Add'}
        </Button>
      </DialogActions>
    </Dialog>
  )
}
