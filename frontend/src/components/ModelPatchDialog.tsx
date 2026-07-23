import { useCallback, useEffect, useRef, useState } from 'react'
import {
  Alert,
  Box,
  Button,
  Checkbox,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Divider,
  FormControlLabel,
  MenuItem,
  Radio,
  Select,
  Stack,
  Tooltip,
  Typography,
} from '@mui/material'
import { useQueryClient } from '@tanstack/react-query'

import {
  api,
  type ModelPatchApplyOptions,
  type ModelPatchPreview,
  type ModelPatchRow,
} from '../api'
import Dropzone from './Dropzone'

/// Merge a scraped metadata patch onto this one model. Drop the zip; if it
/// carries a single model that model is applied straight away, and if it carries
/// several the user picks which one this model is. The bundle block of the patch
/// contributes only creator/source (filled where the model has none) — its
/// name/cover/description describe a bundle, and there isn't one here.
///
/// Defaults match the bundle dialog: the scraped photo replaces the auto render,
/// tags merge, the description is taken, and a name the patch improves on is a
/// pre-ticked rename.
export default function ModelPatchDialog({
  modelId,
  open,
  initialFile,
  onClose,
  onApplied,
}: {
  modelId: string
  open: boolean
  /** A zip already dropped on the page's inline box — previewed on open, so the
   * dialog lands on the model choice instead of its own drop step. */
  initialFile?: File | null
  onClose: () => void
  onApplied: () => void
}) {
  const queryClient = useQueryClient()
  const [zip, setZip] = useState<File | null>(null)
  const [preview, setPreview] = useState<ModelPatchPreview | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [done, setDone] = useState('')

  // Which patch model this model is. Auto-set to the only one; null until picked
  // when the patch carries several.
  const [selected, setSelected] = useState<number | null>(null)
  const [rename, setRename] = useState(false)

  const [opts, setOpts] = useState<Omit<ModelPatchApplyOptions, 'model_index' | 'rename'>>({
    model_tags: 'merge',
    model_images: 'replace_generated',
    model_descriptions: true,
  })

  const reset = () => {
    setZip(null)
    setPreview(null)
    setError('')
    setDone('')
    setSelected(null)
    setRename(false)
  }

  // A rename is worth offering when the patch's name is not the one the model
  // already carries (case-insensitively).
  const nameDiffers = useCallback(
    (row: ModelPatchRow | undefined) =>
      !!row && !!preview && row.patch_name.toLowerCase() !== preview.model_name.toLowerCase(),
    [preview],
  )

  const runPreview = useCallback(
    async (file: File) => {
      setZip(file)
      setBusy(true)
      setError('')
      try {
        const p = await api.previewModelPatch(modelId, file)
        setPreview(p)
        // One model: select it and pre-tick a rename if it improves the name.
        // Several: make the user choose before anything is offered.
        if (p.models.length === 1) {
          setSelected(0)
          setRename(p.models[0].patch_name.toLowerCase() !== p.model_name.toLowerCase())
        } else {
          setSelected(null)
        }
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err))
      } finally {
        setBusy(false)
      }
    },
    [modelId],
  )

  // Preview the file the parent handed us the moment we open, once per drop —
  // tracked by ref so a re-render doesn't re-fire it; reset on close.
  const previewedFile = useRef<File | null>(null)
  useEffect(() => {
    if (!open) {
      previewedFile.current = null
      return
    }
    if (initialFile && previewedFile.current !== initialFile) {
      previewedFile.current = initialFile
      void runPreview(initialFile)
    }
  }, [open, initialFile, runPreview])

  const rows = preview?.models ?? []
  const chosen = selected === null ? undefined : rows.find((r) => r.key === selected)

  const pick = (row: ModelPatchRow) => {
    setSelected(row.key)
    setRename(nameDiffers(row))
  }

  const runApply = async () => {
    if (!zip || selected === null) return
    setBusy(true)
    setError('')
    try {
      const result = await api.applyModelPatch(modelId, zip, {
        ...opts,
        model_index: selected,
        rename,
      })
      await queryClient.invalidateQueries({ queryKey: ['model', modelId] })
      onApplied()
      setDone(
        `Applied: ${result.tags_added} tag(s) added, ${result.images_added} image(s) added, ` +
          `${result.descriptions_added} description(s) set, ${result.aliases_added} alias(es) recorded, ` +
          `${result.creator_refs_set} Creator ID(s) set, ` +
          `${result.custom_fields_set} custom field value(s) set.`,
      )
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy(false)
    }
  }

  const brings = (row: ModelPatchRow) => (
    <Stack direction="row" spacing={0.5} sx={{ flexWrap: 'wrap', alignItems: 'center', gap: 0.5 }}>
      {row.has_image && <Chip size="small" label="image" variant="outlined" />}
      {row.has_description && <Chip size="small" label="description" variant="outlined" />}
      {row.source_url && <Chip size="small" label="source URL" variant="outlined" />}
      {row.creator_ref && (
        <Tooltip title="Creator ID — replaces what the model has">
          <Chip size="small" label={row.creator_ref} variant="outlined" />
        </Tooltip>
      )}
      {row.custom_fields_applied > 0 && (
        <Chip size="small" label={`${row.custom_fields_applied} field(s)`} variant="outlined" />
      )}
      {opts.model_tags !== 'skip' &&
        row.tags.map((t) => (
          <Chip key={t} size="small" label={`+${t}`} color="primary" variant="outlined" />
        ))}
      {!row.has_image &&
        !row.has_description &&
        !row.source_url &&
        !row.creator_ref &&
        row.custom_fields_applied === 0 &&
        (opts.model_tags === 'skip' || row.tags.length === 0) && (
          <Typography variant="caption" color="text.secondary">
            nothing new
          </Typography>
        )}
    </Stack>
  )

  const warnings = chosen?.custom_field_warnings ?? []
  // The bundle block fills creator/source only where the model has none.
  const fills =
    (preview?.bundle_creator && !preview.model_has_creator) ||
    (preview?.bundle_source_url && !preview.model_has_source_url)

  return (
    <Dialog open={open} onClose={onClose} maxWidth="sm" fullWidth>
      <DialogTitle>Import scraped metadata</DialogTitle>
      <DialogContent>
        {error && (
          <Alert severity="error" sx={{ mb: 2 }}>
            {error}
          </Alert>
        )}
        {done ? (
          <Alert severity="success">{done}</Alert>
        ) : !preview ? (
          <Dropzone
            label={busy ? 'Reading…' : 'Drop a bundle-patch zip'}
            hint="patch.json + images — applies its metadata to this model"
            accept=".zip"
            busy={busy}
            onDrop={(drop) => {
              const file = drop.files[0]?.file
              if (file) void runPreview(file)
            }}
          />
        ) : (
          <Stack spacing={2}>
            {warnings.length > 0 && (
              <Alert severity="warning">
                <Typography variant="body2" sx={{ mb: 0.5 }}>
                  These custom field values will be skipped:
                </Typography>
                <Stack direction="row" sx={{ flexWrap: 'wrap', gap: 0.5 }}>
                  {warnings.map((w, i) => (
                    <Chip
                      key={`${w.key}-${i}`}
                      size="small"
                      variant="outlined"
                      color="warning"
                      label={`${w.key} — ${w.reason}`}
                    />
                  ))}
                </Stack>
              </Alert>
            )}

            <Box>
              <Typography variant="subtitle2" gutterBottom>
                {rows.length > 1 ? 'Which model is this?' : 'Model'}
              </Typography>
              {rows.length > 1 ? (
                // Several models in the patch: the user says which one this model
                // is. Each row shows what it would bring, so the choice is informed.
                <Box sx={{ borderTop: (t) => `1px solid ${t.palette.divider}` }}>
                  {rows.map((r) => (
                    <Box
                      key={r.key}
                      onClick={() => pick(r)}
                      sx={{
                        display: 'flex',
                        alignItems: 'flex-start',
                        gap: 1,
                        py: 0.75,
                        cursor: 'pointer',
                        borderBottom: (t) => `1px solid ${t.palette.divider}`,
                      }}
                    >
                      <Radio size="small" sx={{ p: 0, mt: 0.25 }} checked={selected === r.key} />
                      <Box sx={{ minWidth: 0, flexGrow: 1 }}>
                        <Typography variant="body2" sx={{ overflowWrap: 'anywhere' }}>
                          {r.patch_name || <em>(unnamed)</em>}
                          {r.category && (
                            <Typography component="span" variant="caption" color="text.secondary">
                              {'  ·  '}
                              {r.category}
                            </Typography>
                          )}
                        </Typography>
                        <Box sx={{ mt: 0.5 }}>{brings(r)}</Box>
                      </Box>
                    </Box>
                  ))}
                </Box>
              ) : (
                chosen && (
                  <Box>
                    <Typography variant="body2" sx={{ overflowWrap: 'anywhere' }}>
                      {chosen.patch_name || <em>(unnamed)</em>}
                    </Typography>
                    <Box sx={{ mt: 0.5 }}>{brings(chosen)}</Box>
                  </Box>
                )
              )}
            </Box>

            {/* Rename — only when a model is chosen and its scraped name differs. */}
            {chosen && nameDiffers(chosen) && (
              <FormControlLabel
                control={
                  <Checkbox checked={rename} onChange={(e) => setRename(e.target.checked)} />
                }
                label={
                  <Typography variant="body2">
                    Rename{' '}
                    <Box component="span" sx={{ color: 'text.secondary' }}>
                      “{preview.model_name}”
                    </Box>{' '}
                    → <Box component="span">“{chosen.patch_name}”</Box>
                  </Typography>
                }
              />
            )}

            {fills && (
              <Alert severity="info">
                Will set this model’s{' '}
                {[
                  preview.bundle_creator && !preview.model_has_creator
                    ? `creator (${preview.bundle_creator})`
                    : null,
                  preview.bundle_source_url && !preview.model_has_source_url ? 'source URL' : null,
                ]
                  .filter(Boolean)
                  .join(' and ')}{' '}
                from the patch, since it has none.
              </Alert>
            )}

            <Divider />

            <Box>
              <Typography variant="subtitle2" gutterBottom>
                How to apply
              </Typography>
              <Stack direction="row" spacing={1} sx={{ alignItems: 'center' }}>
                <Typography variant="body2" sx={{ minWidth: 120 }}>
                  Model tags
                </Typography>
                <Select
                  size="small"
                  value={opts.model_tags}
                  onChange={(e) =>
                    setOpts((o) => ({ ...o, model_tags: e.target.value as typeof o.model_tags }))
                  }
                >
                  <MenuItem value="merge">Merge (add new)</MenuItem>
                  <MenuItem value="replace">Replace (overwrite)</MenuItem>
                  <MenuItem value="skip">Skip</MenuItem>
                </Select>
              </Stack>
              <Stack direction="row" spacing={1} sx={{ alignItems: 'center', mt: 1 }}>
                <Typography variant="body2" sx={{ minWidth: 120 }}>
                  Model images
                </Typography>
                <Select
                  size="small"
                  value={opts.model_images}
                  onChange={(e) =>
                    setOpts((o) => ({
                      ...o,
                      model_images: e.target.value as typeof o.model_images,
                    }))
                  }
                >
                  <MenuItem value="replace_generated">Replace the render</MenuItem>
                  <MenuItem value="add">Add alongside</MenuItem>
                  <MenuItem value="skip">Skip</MenuItem>
                </Select>
              </Stack>
              <Stack direction="row" spacing={1} sx={{ alignItems: 'center', mt: 1 }}>
                <Typography variant="body2" sx={{ minWidth: 120 }}>
                  Model description
                </Typography>
                <Select
                  size="small"
                  value={opts.model_descriptions ? 'apply' : 'skip'}
                  onChange={(e) =>
                    setOpts((o) => ({ ...o, model_descriptions: e.target.value === 'apply' }))
                  }
                >
                  <MenuItem value="apply">Set (adds a revision)</MenuItem>
                  <MenuItem value="skip">Skip</MenuItem>
                </Select>
              </Stack>
            </Box>
          </Stack>
        )}
      </DialogContent>
      <DialogActions>
        {done ? (
          <Button
            onClick={() => {
              reset()
              onClose()
            }}
          >
            Close
          </Button>
        ) : (
          <>
            <Box sx={{ flexGrow: 1 }} />
            <Button onClick={onClose} disabled={busy}>
              Cancel
            </Button>
            {preview && (
              <Button variant="contained" onClick={runApply} disabled={busy || selected === null}>
                Apply
              </Button>
            )}
          </>
        )}
      </DialogActions>
    </Dialog>
  )
}
