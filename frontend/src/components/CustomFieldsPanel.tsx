import { useState } from 'react'
import {
  Alert,
  Autocomplete,
  Button,
  Checkbox,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogContentText,
  DialogTitle,
  FormControlLabel,
  IconButton,
  MenuItem,
  Paper,
  Stack,
  Table,
  TableBody,
  TableCell,
  TableHead,
  TableRow,
  TextField,
  Tooltip,
  Typography,
} from '@mui/material'
import DeleteIcon from '@mui/icons-material/Delete'
import EditIcon from '@mui/icons-material/Edit'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import {
  api,
  type CustomFieldDef,
  type CustomFieldInput,
  type CustomFieldKind,
  type CustomFieldVisibility,
} from '../api'
import { useAuth } from '../main'

const KINDS: { value: CustomFieldKind; label: string; hint: string }[] = [
  { value: 'text', label: 'Text', hint: 'A free-text note.' },
  { value: 'checkbox', label: 'Checkbox', hint: 'A yes/no flag.' },
  { value: 'choice', label: 'Choice', hint: 'One of a fixed list of values.' },
  { value: 'rating', label: 'Rating', hint: 'Stars, out of a maximum you set.' },
  {
    value: 'file',
    label: 'File',
    hint: 'A single dropped file, shown on the page but kept out of the file list.',
  },
]

const VISIBILITIES: { value: CustomFieldVisibility; label: string }[] = [
  { value: 'anonymous', label: 'Everyone' },
  { value: 'viewer', label: 'Signed in' },
  { value: 'editor', label: 'Editors and admins' },
  { value: 'admin', label: 'Admins only' },
]

const kindLabel = (kind: CustomFieldKind) => KINDS.find((k) => k.value === kind)?.label ?? kind
const visibilityLabel = (v: CustomFieldVisibility) =>
  VISIBILITIES.find((x) => x.value === v)?.label ?? v

const BLANK: CustomFieldInput = {
  key: '',
  name: '',
  kind: 'text',
  options: {},
  applies_to_models: true,
  applies_to_bundles: false,
  bundle_persists_to_model: false,
  bundle_persist_overwrites: false,
  visibility: 'anonymous',
  position: 0,
}

/// Admin-only: the meshtrove-wide vocabulary of extra metadata fields. A field
/// defined here appears on every model and/or bundle it applies to — this is a
/// site setting, not something attached to one item.
export default function CustomFieldsPanel() {
  const { user } = useAuth()
  const queryClient = useQueryClient()
  const [editing, setEditing] = useState<CustomFieldDef | 'new' | null>(null)
  const [deleting, setDeleting] = useState<CustomFieldDef | null>(null)
  const [error, setError] = useState('')

  const { data: fields } = useQuery({
    queryKey: ['custom-fields'],
    queryFn: () => api.customFields(),
    enabled: user?.role === 'admin',
  })

  const refresh = () => void queryClient.invalidateQueries({ queryKey: ['custom-fields'] })

  const remove = async () => {
    if (!deleting) return
    try {
      await api.deleteCustomField(deleting.id)
      setDeleting(null)
      refresh()
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    }
  }

  return (
    <Paper variant="outlined" sx={{ p: 3, mt: 3 }}>
      <Typography variant="h6" sx={{ mb: 0.5 }}>
        Custom fields
      </Typography>
      <Typography variant="body2" color="text.secondary" sx={{ mb: 2 }}>
        Extra metadata beyond the built-in fields — a &ldquo;Printed?&rdquo; checkbox, a material
        choice, a star rating, a reference PDF. A field defined here is available on every model
        and/or bundle it applies to.
      </Typography>
      {error && (
        <Alert severity="error" sx={{ mb: 2 }} onClose={() => setError('')}>
          {error}
        </Alert>
      )}
      <Table size="small">
        <TableHead>
          <TableRow>
            <TableCell>Field</TableCell>
            <TableCell sx={{ width: 110 }}>Kind</TableCell>
            <TableCell sx={{ width: 150 }}>Applies to</TableCell>
            <TableCell sx={{ width: 170 }}>Visible to</TableCell>
            <TableCell sx={{ width: 96 }} align="right">
              Actions
            </TableCell>
          </TableRow>
        </TableHead>
        <TableBody>
          {(fields ?? []).map((f) => (
            <TableRow key={f.id}>
              <TableCell>
                {f.name}
                <Typography component="span" color="text.secondary" sx={{ ml: 1 }}>
                  <code>{f.key}</code>
                </Typography>
              </TableCell>
              <TableCell>{kindLabel(f.kind)}</TableCell>
              <TableCell>
                <Stack direction="row" spacing={0.5}>
                  {f.applies_to_models && <Chip size="small" label="Models" />}
                  {f.applies_to_bundles && <Chip size="small" label="Bundles" />}
                </Stack>
                {f.bundle_persists_to_model && (
                  <Typography variant="caption" color="text.secondary">
                    {f.bundle_persist_overwrites
                      ? 'bundle value overwrites members'
                      : 'bundle value fills in members'}
                  </Typography>
                )}
              </TableCell>
              <TableCell>{visibilityLabel(f.visibility)}</TableCell>
              <TableCell align="right">
                <Stack direction="row" spacing={0.5} sx={{ justifyContent: 'flex-end' }}>
                  <Tooltip title="Edit field">
                    <IconButton size="small" onClick={() => setEditing(f)}>
                      <EditIcon sx={{ fontSize: 18 }} />
                    </IconButton>
                  </Tooltip>
                  <Tooltip title="Delete field">
                    <IconButton size="small" color="error" onClick={() => setDeleting(f)}>
                      <DeleteIcon sx={{ fontSize: 18 }} />
                    </IconButton>
                  </Tooltip>
                </Stack>
              </TableCell>
            </TableRow>
          ))}
          {fields?.length === 0 && (
            <TableRow>
              <TableCell colSpan={5}>
                <Typography color="text.secondary">No custom fields defined yet.</Typography>
              </TableCell>
            </TableRow>
          )}
        </TableBody>
      </Table>
      <Button sx={{ mt: 2 }} variant="outlined" onClick={() => setEditing('new')}>
        Add field
      </Button>

      <CustomFieldDialog
        target={editing}
        onClose={() => setEditing(null)}
        onDone={() => {
          setEditing(null)
          refresh()
        }}
      />

      <Dialog open={!!deleting} onClose={() => setDeleting(null)} maxWidth="xs" fullWidth>
        <DialogTitle>Delete custom field?</DialogTitle>
        <DialogContent>
          <DialogContentText>
            Delete <strong>{deleting?.name}</strong>? Every value stored under it — on every model
            and bundle, including any uploaded files — goes with it. This can't be undone.
          </DialogContentText>
        </DialogContent>
        <DialogActions>
          <Button onClick={() => setDeleting(null)}>Cancel</Button>
          <Button variant="contained" color="error" onClick={remove}>
            Delete field
          </Button>
        </DialogActions>
      </Dialog>
    </Paper>
  )
}

/// Add or edit one definition. `target` is an existing field, the string 'new',
/// or null when closed.
function CustomFieldDialog({
  target,
  onClose,
  onDone,
}: {
  target: CustomFieldDef | 'new' | null
  onClose: () => void
  onDone: () => void
}) {
  const [draft, setDraft] = useState<CustomFieldInput>(BLANK)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  // Reset the form whenever the dialog is pointed at something else.
  const targetId = target === 'new' ? 'new' : (target?.id ?? null)
  const [lastId, setLastId] = useState<string | null>(null)
  if (target && targetId !== lastId) {
    setLastId(targetId)
    setDraft(target === 'new' ? BLANK : { ...target })
    setError('')
  }

  const set = <K extends keyof CustomFieldInput>(key: K, value: CustomFieldInput[K]) =>
    setDraft((prev) => ({ ...prev, [key]: value }))

  const choices = draft.options.choices ?? []
  const canSubmit =
    draft.name.trim() !== '' &&
    draft.key.trim() !== '' &&
    (draft.applies_to_models || draft.applies_to_bundles) &&
    (draft.kind !== 'choice' || choices.length > 0) &&
    !busy

  const submit = async () => {
    setBusy(true)
    setError('')
    try {
      // The persistence flags are only meaningful while their preconditions
      // hold; unticking "Bundles" shouldn't leave a stale one behind for the
      // backend to reject.
      const persists =
        draft.applies_to_models && draft.applies_to_bundles && draft.bundle_persists_to_model
      const body: CustomFieldInput = {
        ...draft,
        key: draft.key.trim(),
        name: draft.name.trim(),
        bundle_persists_to_model: persists,
        bundle_persist_overwrites: persists && draft.bundle_persist_overwrites,
      }
      if (target === 'new') await api.createCustomField(body)
      else if (target) await api.updateCustomField(target.id, body)
      onDone()
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={!!target} onClose={onClose} maxWidth="sm" fullWidth>
      <DialogTitle>{target === 'new' ? 'New custom field' : 'Edit custom field'}</DialogTitle>
      <DialogContent>
        <Stack spacing={2} sx={{ mt: 1 }}>
          {error && <Alert severity="error">{error}</Alert>}
          <TextField
            label="Name"
            value={draft.name}
            onChange={(e) => set('name', e.target.value)}
            autoFocus
            required
          />
          <TextField
            label="Key"
            value={draft.key}
            onChange={(e) => set('key', e.target.value)}
            required
            helperText="Letters, digits, - and _. Scraped metadata is matched against this, so keep it stable."
          />
          <TextField
            select
            label="Kind"
            value={draft.kind}
            onChange={(e) => set('kind', e.target.value as CustomFieldKind)}
            helperText={KINDS.find((k) => k.value === draft.kind)?.hint}
          >
            {KINDS.map((k) => (
              <MenuItem key={k.value} value={k.value}>
                {k.label}
              </MenuItem>
            ))}
          </TextField>

          {draft.kind === 'choice' && (
            <Autocomplete
              multiple
              freeSolo
              options={[] as string[]}
              value={choices}
              onChange={(_, value) => set('options', { ...draft.options, choices: value })}
              renderInput={(props) => (
                <TextField {...props} label="Choices" placeholder="add a choice…" />
              )}
            />
          )}
          {draft.kind === 'rating' && (
            <TextField
              label="Stars"
              type="number"
              value={draft.options.max ?? 5}
              onChange={(e) =>
                set('options', { ...draft.options, max: Number(e.target.value) || 1 })
              }
              slotProps={{ htmlInput: { min: 1, max: 10 } }}
              sx={{ width: 140 }}
            />
          )}

          <Stack direction="row" spacing={2}>
            <FormControlLabel
              control={
                <Checkbox
                  checked={draft.applies_to_models}
                  onChange={(e) => set('applies_to_models', e.target.checked)}
                />
              }
              label="Models"
            />
            <FormControlLabel
              control={
                <Checkbox
                  checked={draft.applies_to_bundles}
                  onChange={(e) => set('applies_to_bundles', e.target.checked)}
                />
              }
              label="Bundles"
            />
          </Stack>

          {/* Persistence is a bundle→model flow, so it only means anything on a
              field that lives at both ends. */}
          <FormControlLabel
            disabled={!(draft.applies_to_models && draft.applies_to_bundles)}
            control={
              <Checkbox
                checked={draft.bundle_persists_to_model}
                onChange={(e) => set('bundle_persists_to_model', e.target.checked)}
              />
            }
            label="Setting this on a bundle also sets it on its member models"
          />
          <FormControlLabel
            disabled={!draft.bundle_persists_to_model}
            control={
              <Checkbox
                checked={draft.bundle_persist_overwrites}
                onChange={(e) => set('bundle_persist_overwrites', e.target.checked)}
              />
            }
            label="…even where the model already has a value of its own"
            sx={{ ml: 3, mt: -1 }}
          />

          <TextField
            select
            label="Visible to"
            value={draft.visibility}
            onChange={(e) => set('visibility', e.target.value as CustomFieldVisibility)}
          >
            {VISIBILITIES.map((v) => (
              <MenuItem key={v.value} value={v.value}>
                {v.label}
              </MenuItem>
            ))}
          </TextField>
          <TextField
            label="Position"
            type="number"
            value={draft.position}
            onChange={(e) => set('position', Number(e.target.value) || 0)}
            helperText="Lower sorts first; ties fall back to the name."
            sx={{ width: 160 }}
          />
        </Stack>
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="contained" onClick={submit} disabled={!canSubmit}>
          {target === 'new' ? 'Create field' : 'Save field'}
        </Button>
      </DialogActions>
    </Dialog>
  )
}
