import { useState } from 'react'
import {
  Alert,
  Box,
  Button,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Divider,
  FormControl,
  FormControlLabel,
  MenuItem,
  Checkbox,
  Select,
  Stack,
  Typography,
} from '@mui/material'
import { useQueryClient } from '@tanstack/react-query'

import { api, type PatchApplyOptions, type PatchPreview } from '../api'
import Dropzone from './Dropzone'

/// Merge a scraped bundle patch onto this bundle. Drop the
/// zip, see what it matched to which member, tick what to apply, and apply.
///
/// The defaults encode the usual intent: the scraped photo replaces the
/// auto-generated render (a real picture beats an f3d preview), while tags are
/// merged onto whatever the model already has rather than overwriting them.
export default function BundlePatchDialog({
  bundleId,
  open,
  onClose,
  onApplied,
}: {
  bundleId: string
  open: boolean
  onClose: () => void
  onApplied: () => void
}) {
  const queryClient = useQueryClient()
  const [zip, setZip] = useState<File | null>(null)
  const [preview, setPreview] = useState<PatchPreview | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [done, setDone] = useState('')

  // Which member each ambiguous / unmatched patch model should apply to.
  const [resolved, setResolved] = useState<Record<string, string>>({})

  const [opts, setOpts] = useState<Omit<PatchApplyOptions, 'matches'>>({
    rename_models: false,
    model_tags: 'merge',
    model_images: 'replace_generated',
    bundle_cover: true,
    bundle_description: true,
  })

  const reset = () => {
    setZip(null)
    setPreview(null)
    setError('')
    setDone('')
    setResolved({})
  }

  const runPreview = async (file: File) => {
    setZip(file)
    setBusy(true)
    setError('')
    try {
      setPreview(await api.previewBundlePatch(bundleId, file))
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy(false)
    }
  }

  const runApply = async () => {
    if (!zip) return
    setBusy(true)
    setError('')
    try {
      const result = await api.applyBundlePatch(bundleId, zip, { ...opts, matches: resolved })
      await queryClient.invalidateQueries({ queryKey: ['bundle', bundleId] })
      onApplied()
      setDone(
        `Applied: ${result.models_updated} model(s) updated, ${result.tags_added} tag(s) added, ` +
          `${result.images_added} image(s) added.`,
      )
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onClose={onClose} maxWidth="md" fullWidth>
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
            hint="patch.json + images"
            accept=".zip"
            busy={busy}
            onDrop={(drop) => {
              const file = drop.files[0]?.file
              if (file) void runPreview(file)
            }}
          />
        ) : (
          <Stack spacing={2}>
            {/* What matched */}
            <Box>
              <Typography variant="subtitle2" gutterBottom>
                {preview.matched.length} of{' '}
                {preview.matched.length + preview.ambiguous.length + preview.unmatched_patch.length}{' '}
                patch models matched a member
              </Typography>
              <Stack spacing={0.5}>
                {preview.matched.map((m) => (
                  <Stack
                    key={m.model_id}
                    direction="row"
                    spacing={1}
                    sx={{ alignItems: 'center', flexWrap: 'wrap' }}
                  >
                    <Typography variant="body2" sx={{ minWidth: 160 }}>
                      {m.patch_name}
                      {m.patch_name !== m.model_name && (
                        <Typography component="span" variant="caption" color="text.secondary">
                          {' → '}
                          {m.model_name}
                        </Typography>
                      )}
                    </Typography>
                    {m.has_image && <Chip size="small" label="image" variant="outlined" />}
                    {m.add_tags.map((t) => (
                      <Chip
                        key={t}
                        size="small"
                        label={`+${t}`}
                        color="primary"
                        variant="outlined"
                      />
                    ))}
                    {!m.has_image && m.add_tags.length === 0 && (
                      <Typography variant="caption" color="text.secondary">
                        nothing new
                      </Typography>
                    )}
                  </Stack>
                ))}
              </Stack>
            </Box>

            {/* Ambiguous — the user picks */}
            {preview.ambiguous.length > 0 && (
              <Box>
                <Typography variant="subtitle2" gutterBottom>
                  Ambiguous — choose the model to apply to
                </Typography>
                {preview.ambiguous.map((a) => (
                  <Stack
                    key={a.patch_name}
                    direction="row"
                    spacing={1}
                    sx={{ alignItems: 'center', mb: 0.5 }}
                  >
                    <Typography variant="body2" sx={{ minWidth: 160 }}>
                      {a.patch_name}
                    </Typography>
                    <FormControl size="small" sx={{ minWidth: 200 }}>
                      <Select
                        displayEmpty
                        value={resolved[a.patch_name] ?? ''}
                        onChange={(e) =>
                          setResolved((r) => ({ ...r, [a.patch_name]: e.target.value }))
                        }
                      >
                        <MenuItem value="">
                          <em>skip</em>
                        </MenuItem>
                        {a.candidates.map((c) => (
                          <MenuItem key={c.id} value={c.id}>
                            {c.name}
                          </MenuItem>
                        ))}
                      </Select>
                    </FormControl>
                  </Stack>
                ))}
              </Box>
            )}

            {preview.unmatched_patch.length > 0 && (
              <Alert severity="warning" sx={{ py: 0.5 }}>
                No member matched: {preview.unmatched_patch.join(', ')}. These are skipped — nothing
                in the bundle to apply them to.
              </Alert>
            )}

            <Divider />

            {/* What to apply */}
            <Box>
              <Typography variant="subtitle2" gutterBottom>
                What to apply
              </Typography>
              <Stack>
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
                <FormControlLabel
                  sx={{ mt: 1 }}
                  control={
                    <Checkbox
                      checked={opts.rename_models}
                      onChange={(e) => setOpts((o) => ({ ...o, rename_models: e.target.checked }))}
                    />
                  }
                  label="Rename models to the scraped names"
                />
                <FormControlLabel
                  control={
                    <Checkbox
                      checked={opts.bundle_cover}
                      disabled={preview.bundle_cover_count === 0}
                      onChange={(e) => setOpts((o) => ({ ...o, bundle_cover: e.target.checked }))}
                    />
                  }
                  label={`Set the bundle cover${
                    preview.bundle_cover_count ? '' : ' (none in patch)'
                  }`}
                />
                <FormControlLabel
                  control={
                    <Checkbox
                      checked={opts.bundle_description}
                      disabled={!preview.bundle_has_description}
                      onChange={(e) =>
                        setOpts((o) => ({ ...o, bundle_description: e.target.checked }))
                      }
                    />
                  }
                  label={`Set the bundle description${
                    preview.bundle_has_description ? '' : ' (none in patch)'
                  }`}
                />
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
            {preview && (
              <Button onClick={reset} disabled={busy}>
                Choose a different file
              </Button>
            )}
            <Box sx={{ flexGrow: 1 }} />
            <Button onClick={onClose} disabled={busy}>
              Cancel
            </Button>
            {preview && (
              <Button variant="contained" onClick={runApply} disabled={busy}>
                Apply
              </Button>
            )}
          </>
        )}
      </DialogActions>
    </Dialog>
  )
}
