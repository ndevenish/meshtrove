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
} from '@mui/material'
import ExpandMoreIcon from '@mui/icons-material/ExpandMore'
import AddIcon from '@mui/icons-material/Add'
import DownloadIcon from '@mui/icons-material/Download'
import UploadFileIcon from '@mui/icons-material/UploadFile'
import FolderIcon from '@mui/icons-material/Folder'
import DeleteIcon from '@mui/icons-material/Delete'
import InsertDriveFileIcon from '@mui/icons-material/InsertDriveFile'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import {
  api,
  downloadUrl,
  formatBytes,
  type FileRecord,
  type ModelDetail,
  type VariantDetail,
} from '../api'

export default function VariantSection({
  model,
  canEdit,
  onChange,
}: {
  model: ModelDetail
  canEdit: boolean
  onChange: () => void
}) {
  const [editing, setEditing] = useState<VariantDetail | 'new' | null>(null)

  return (
    <Box>
      <Stack direction="row" sx={{ alignItems: 'center', mb: 1 }}>
        <Typography variant="h6">Variants</Typography>
        <Box sx={{ flexGrow: 1 }} />
        {canEdit && (
          <Button startIcon={<AddIcon />} size="small" onClick={() => setEditing('new')}>
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
          onChange={onChange}
          onEdit={() => setEditing(variant)}
        />
      ))}
      <VariantEditDialog
        open={editing !== null}
        variant={editing === 'new' ? undefined : (editing ?? undefined)}
        modelId={model.id}
        onClose={() => setEditing(null)}
        onChange={onChange}
      />
    </Box>
  )
}

function VariantRow({
  variant,
  canEdit,
  onChange,
  onEdit,
}: {
  variant: VariantDetail
  canEdit: boolean
  onChange: () => void
  onEdit: () => void
}) {
  const queryClient = useQueryClient()
  const [expanded, setExpanded] = useState(false)
  const [uploading, setUploading] = useState(false)
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
          <Typography sx={{ fontWeight: 600 }}>{variant.name}</Typography>
          {Object.entries(variant.options).map(([axis, value]) => (
            <Chip key={axis} label={`${axis}: ${value}`} size="small" variant="outlined" />
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
        {uploading && <LinearProgress sx={{ mb: 1 }} />}
        {expanded && files && <FileTree files={files} />}
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
            <Button
              size="small"
              color="error"
              onClick={async () => {
                if (confirm(`Delete variant "${variant.name}" and its files?`)) {
                  await api.deleteVariant(variant.id)
                  onChange()
                }
              }}
            >
              Delete
            </Button>
          </Stack>
        )}
      </AccordionDetails>
    </Accordion>
  )
}

/// Rebuild the kept folder structure from the flat path column.
function FileTree({ files }: { files: FileRecord[] }) {
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
              <InsertDriveFileIcon sx={{ fontSize: 16, opacity: 0.5 }} />
              <Typography variant="body2" sx={{ flexGrow: 1 }} noWrap>
                {file.filename}
              </Typography>
              <Chip label={file.kind} size="small" variant="outlined" sx={{ height: 20 }} />
              <Typography variant="caption" color="text.secondary" sx={{ width: 64 }}>
                {formatBytes(file.size)}
              </Typography>
              <Tooltip title="Download">
                <IconButton size="small" component="a" href={downloadUrl(file.id)}>
                  <DownloadIcon sx={{ fontSize: 18 }} />
                </IconButton>
              </Tooltip>
            </Stack>
          ))}
        </Box>
      ))}
    </Box>
  )
}

function VariantEditDialog({
  open,
  variant,
  modelId,
  onClose,
  onChange,
}: {
  open: boolean
  variant?: VariantDetail
  modelId: string
  onClose: () => void
  onChange: () => void
}) {
  const { data: axes } = useQuery({ queryKey: ['axes'], queryFn: () => api.axes() })
  const [name, setName] = useState(variant?.name ?? '')
  const [options, setOptions] = useState<Record<string, string>>(variant?.options ?? {})
  const [notes, setNotes] = useState(variant?.print_notes ?? '')
  const [newAxis, setNewAxis] = useState('')
  const [error, setError] = useState('')

  // Reset when target changes (dialog reused between add/edit)
  const [lastKey, setLastKey] = useState<string | null>(null)
  const key = variant?.id ?? 'new'
  if (open && key !== lastKey) {
    setLastKey(key)
    setName(variant?.name ?? '')
    setOptions(variant?.options ?? {})
    setNotes(variant?.print_notes ?? '')
    setError('')
  }

  const axisNames = new Set([...(axes ?? []).map((a) => a.name), ...Object.keys(options)])

  const submit = async () => {
    try {
      const body = { name, options, print_notes: notes || null }
      if (variant) await api.updateVariant(variant.id, body)
      else await api.createVariant(modelId, body)
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
          <TextField
            label="Name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder="e.g. 32mm Supported, My merged remix"
            autoFocus
          />
          {[...axisNames].map((axis) => {
            const axisOptions = axes?.find((a) => a.name === axis)?.options ?? []
            return (
              <Stack sx={{ alignItems: 'center' }} key={axis} direction="row" spacing={1}>
                <Autocomplete
                  freeSolo
                  fullWidth
                  options={axisOptions.map((o) => o.value)}
                  value={options[axis] ?? ''}
                  onInputChange={(_, value) =>
                    setOptions((previous) => ({ ...previous, [axis]: value }))
                  }
                  renderInput={(params) => <TextField {...params} label={axis} size="small" />}
                />
                <IconButton
                  size="small"
                  onClick={() =>
                    setOptions((previous) => {
                      const next = { ...previous }
                      delete next[axis]
                      return next
                    })
                  }
                >
                  <DeleteIcon fontSize="small" />
                </IconButton>
              </Stack>
            )
          })}
          <Stack direction="row" spacing={1}>
            <TextField
              label="Add category (axis)"
              size="small"
              value={newAxis}
              onChange={(e) => setNewAxis(e.target.value)}
              placeholder="e.g. base, pose, material"
            />
            <Button
              onClick={() => {
                const axis = newAxis.trim()
                if (axis && !(axis in options)) {
                  setOptions((previous) => ({ ...previous, [axis]: '' }))
                }
                setNewAxis('')
              }}
            >
              Add
            </Button>
          </Stack>
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
        <Button
          variant="contained"
          onClick={submit}
          disabled={!name.trim() || Object.values(options).some((v) => !v.trim())}
        >
          {variant ? 'Save' : 'Add'}
        </Button>
      </DialogActions>
    </Dialog>
  )
}
