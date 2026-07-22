import { useState } from 'react'
import {
  Checkbox,
  Chip,
  FormControlLabel,
  IconButton,
  MenuItem,
  Rating,
  Stack,
  TextField,
  Tooltip,
  Typography,
} from '@mui/material'
import AttachFileIcon from '@mui/icons-material/AttachFile'
import DeleteIcon from '@mui/icons-material/Delete'

import { formatBytes, type CustomFieldValue, type CustomFieldValueInput } from '../api'
import Dropzone from './Dropzone'

/// The value a scalar control hands back. Null means "unset it".
export type ScalarValue = string | boolean | number | null

/// Edit-mode state for a whole set of custom fields: what each scalar control
/// currently holds, and the payload to hang off the owner's save. File fields
/// are absent from the payload — they were already written when they were
/// dropped, and the backend refuses them here anyway.
export function useCustomFieldDraft(entries: CustomFieldValue[]) {
  const initial = () =>
    Object.fromEntries(entries.map((e) => [e.field.id, e.value])) as Record<string, ScalarValue>
  const [draft, setDraft] = useState<Record<string, ScalarValue>>(initial)

  return {
    valueOf: (entry: CustomFieldValue): ScalarValue => draft[entry.field.id] ?? null,
    setValue: (entry: CustomFieldValue, value: ScalarValue) =>
      setDraft((prev) => ({ ...prev, [entry.field.id]: value })),
    /** What to send as `custom_fields` alongside the rest of the edit. */
    payload: (): CustomFieldValueInput[] =>
      entries
        .filter((e) => e.field.kind !== 'file')
        .map((e) => ({ field_id: e.field.id, value: draft[e.field.id] ?? null })),
  }
}

/// One custom field, read-only: the label and what it says. Renders nothing
/// when the field is unset, so a page isn't a wall of blanks — the edit form is
/// where the blanks belong.
export function CustomFieldReadout({ entry }: { entry: CustomFieldValue }) {
  const { field, value, file } = entry

  if (field.kind === 'file') {
    if (!file) return null
    return (
      <Stack direction="row" spacing={1} sx={{ alignItems: 'center', mb: 1 }}>
        <Typography variant="body2" color="text.secondary">
          {field.name}:
        </Typography>
        <Chip
          size="small"
          icon={<AttachFileIcon />}
          label={file.filename}
          component="a"
          href={`/api/files/${file.file_id}/download`}
          clickable
        />
      </Stack>
    )
  }
  if (value === null || value === undefined || value === '') return null

  if (field.kind === 'rating') {
    return (
      <Stack direction="row" spacing={1} sx={{ alignItems: 'center', mb: 1 }}>
        <Typography variant="body2" color="text.secondary">
          {field.name}:
        </Typography>
        <Rating readOnly size="small" value={Number(value)} max={field.options.max ?? 5} />
      </Stack>
    )
  }
  return (
    <Typography variant="body2" color="text.secondary" sx={{ mb: 1 }}>
      {field.name}: {field.kind === 'checkbox' ? (value ? 'Yes' : 'No') : String(value)}
    </Typography>
  )
}

/// One custom field, editable. Scalars report upward through `onChange` and are
/// saved with the rest of the form; a file-kind field is bytes, so it uploads
/// (and clears) the moment you drop it, through `onUploadFile`/`onClearFile`.
export function CustomFieldControl({
  entry,
  value,
  onChange,
  onUploadFile,
  onClearFile,
}: {
  entry: CustomFieldValue
  value: ScalarValue
  onChange: (value: ScalarValue) => void
  /** Omitted where there is nothing to upload *to*, and the file kind then
      renders nothing at all. */
  onUploadFile?: (file: File) => Promise<void>
  onClearFile?: () => Promise<void>
}) {
  const { field, file } = entry
  const [busy, setBusy] = useState(false)

  switch (field.kind) {
    case 'checkbox':
      return (
        <FormControlLabel
          control={
            <Checkbox checked={value === true} onChange={(e) => onChange(e.target.checked)} />
          }
          label={field.name}
        />
      )
    case 'choice':
      return (
        <TextField
          select
          label={field.name}
          value={typeof value === 'string' ? value : ''}
          onChange={(e) => onChange(e.target.value)}
        >
          {/* An explicit way back to "not set": a select with no empty option
              can be filled in but never emptied. */}
          <MenuItem value="">
            <em>Not set</em>
          </MenuItem>
          {(field.options.choices ?? []).map((choice) => (
            <MenuItem key={choice} value={choice}>
              {choice}
            </MenuItem>
          ))}
        </TextField>
      )
    case 'rating':
      return (
        <Stack direction="row" spacing={1} sx={{ alignItems: 'center' }}>
          <Typography variant="body2" color="text.secondary">
            {field.name}
          </Typography>
          <Rating
            value={typeof value === 'number' ? value : 0}
            max={field.options.max ?? 5}
            onChange={(_, next) => onChange(next ?? 0)}
          />
        </Stack>
      )
    case 'file':
      if (!onUploadFile) return null
      return (
        <Stack spacing={1}>
          {file && (
            <Stack direction="row" spacing={1} sx={{ alignItems: 'center' }}>
              <Chip
                size="small"
                icon={<AttachFileIcon />}
                label={`${file.filename} · ${formatBytes(file.size)}`}
                component="a"
                href={`/api/files/${file.file_id}/download`}
                clickable
              />
              <Tooltip title="Remove file">
                <IconButton
                  size="small"
                  color="error"
                  disabled={busy}
                  onClick={() => {
                    setBusy(true)
                    void onClearFile?.().finally(() => setBusy(false))
                  }}
                >
                  <DeleteIcon sx={{ fontSize: 18 }} />
                </IconButton>
              </Tooltip>
            </Stack>
          )}
          <Dropzone
            label={busy ? 'Uploading…' : file ? `Replace ${field.name}` : field.name}
            hint="One file · dropped here it stays out of the file list"
            busy={busy}
            onDrop={(drop) => {
              const dropped = drop.files[0]?.file
              if (!dropped) return
              setBusy(true)
              void onUploadFile(dropped).finally(() => setBusy(false))
            }}
          />
        </Stack>
      )
    default:
      return (
        <TextField
          label={field.name}
          value={typeof value === 'string' ? value : ''}
          onChange={(e) => onChange(e.target.value)}
        />
      )
  }
}
