import { useMemo, useState } from 'react'
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
  FormControl,
  FormControlLabel,
  MenuItem,
  Select,
  Stack,
  Typography,
} from '@mui/material'
import ReactMarkdown from 'react-markdown'
import { useQueryClient } from '@tanstack/react-query'

import { api, type PatchApplyOptions, type PatchMember, type PatchPreview } from '../api'
import Dropzone from './Dropzone'

/// Merge a scraped bundle patch onto this bundle. Drop the
/// zip, see what matched to which member — and for what did not, pick a member by
/// hand — tick what to apply, and apply.
///
/// Defaults encode the usual intent: the scraped photo replaces the auto-generated
/// render (a real picture beats an f3d preview), tags merge onto what the model
/// already has. Rename is per-model and off by default — a bigger commitment than
/// a tag, decided one at a time.
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

  // patch label -> chosen member id (ambiguous + manual matches).
  const [resolved, setResolved] = useState<Record<string, string>>({})
  // patch labels to rename to the scraped name.
  const [renameSet, setRenameSet] = useState<Set<string>>(new Set())

  const [opts, setOpts] = useState<Omit<PatchApplyOptions, 'matches' | 'rename'>>({
    model_tags: 'merge',
    model_images: 'replace_generated',
    bundle_cover: true,
    bundle_description: true,
  })

  const membersById = useMemo(() => {
    const m = new Map<string, PatchMember>()
    for (const mem of preview?.members ?? []) m.set(mem.id, mem)
    return m
  }, [preview])

  const reset = () => {
    setZip(null)
    setPreview(null)
    setError('')
    setDone('')
    setResolved({})
    setRenameSet(new Set())
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
      const result = await api.applyBundlePatch(bundleId, zip, {
        ...opts,
        matches: resolved,
        rename: [...renameSet],
      })
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

  // Every row that resolves to a member (auto-matched, or manually chosen), with
  // the target model, the tags that would be added (patch tags minus what the
  // member already has), and whether a rename would change the name.
  type Row = {
    label: string
    modelId: string
    modelName: string
    addTags: string[]
    hasImage: boolean
    nameDiffers: boolean
  }
  const rows: Row[] = useMemo(() => {
    if (!preview) return []
    const out: Row[] = []
    for (const m of preview.matched) {
      out.push({
        label: m.patch_name,
        modelId: m.model_id,
        modelName: m.model_name,
        addTags: m.add_tags,
        hasImage: m.has_image,
        nameDiffers: m.patch_name !== m.model_name,
      })
    }
    // Ambiguous + unmatched become rows once (and only when) the user picks a member.
    for (const u of [...preview.ambiguous, ...preview.unmatched_patch]) {
      const chosen = resolved[u.patch_name]
      if (!chosen) continue
      const member = membersById.get(chosen)
      const have = new Set((member?.tags ?? []).map((t) => t.toLowerCase()))
      out.push({
        label: u.patch_name,
        modelId: chosen,
        modelName: member?.name ?? '',
        addTags: u.patch_tags.filter((t) => !have.has(t.toLowerCase())),
        hasImage: u.has_image,
        nameDiffers: !!member && u.patch_name !== member.name,
      })
    }
    return out
  }, [preview, resolved, membersById])

  // "Select all" targets the rows where a rename would actually change something.
  const renameable = rows.filter((r) => r.nameDiffers).map((r) => r.label)
  const allRenamed = renameable.length > 0 && renameable.every((l) => renameSet.has(l))
  const toggleAllRenames = () => setRenameSet(allRenamed ? new Set() : new Set(renameable))
  const toggleRename = (label: string) =>
    setRenameSet((s) => {
      const next = new Set(s)
      if (next.has(label)) next.delete(label)
      else next.add(label)
      return next
    })

  const tagChips = (tags: string[]) =>
    tags.map((t) => (
      <Chip key={t} size="small" label={`+${t}`} color="primary" variant="outlined" />
    ))

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
            {/* Bundle cover + description preview */}
            {(preview.bundle_covers.length > 0 || preview.bundle_description) && (
              <Box>
                <Typography variant="subtitle2" gutterBottom>
                  Bundle
                </Typography>
                <Stack direction="row" spacing={2}>
                  {preview.bundle_covers.length > 0 && (
                    <Stack spacing={0.5}>
                      <FormControlLabel
                        control={
                          <Checkbox
                            checked={opts.bundle_cover}
                            onChange={(e) =>
                              setOpts((o) => ({ ...o, bundle_cover: e.target.checked }))
                            }
                          />
                        }
                        label="Set cover"
                      />
                      <Stack direction="row" spacing={1}>
                        {preview.bundle_covers.map((src, i) => (
                          <Box
                            key={i}
                            component="img"
                            src={src}
                            sx={{
                              width: 96,
                              height: 96,
                              objectFit: 'cover',
                              borderRadius: 1,
                              opacity: opts.bundle_cover ? 1 : 0.4,
                              border: (t) =>
                                i === 0 ? `2px solid ${t.palette.primary.main}` : '1px solid #8884',
                            }}
                          />
                        ))}
                      </Stack>
                    </Stack>
                  )}
                  {preview.bundle_description && (
                    <Box sx={{ flexGrow: 1, minWidth: 0 }}>
                      <FormControlLabel
                        control={
                          <Checkbox
                            checked={opts.bundle_description}
                            onChange={(e) =>
                              setOpts((o) => ({ ...o, bundle_description: e.target.checked }))
                            }
                          />
                        }
                        label="Set description"
                      />
                      <Box
                        sx={{
                          fontSize: 13,
                          maxHeight: 120,
                          overflow: 'auto',
                          opacity: opts.bundle_description ? 1 : 0.4,
                          '& p': { m: 0 },
                        }}
                      >
                        <ReactMarkdown>{preview.bundle_description}</ReactMarkdown>
                      </Box>
                    </Box>
                  )}
                </Stack>
              </Box>
            )}

            <Divider />

            {/* Models that resolve to a member */}
            <Box>
              <Stack direction="row" sx={{ alignItems: 'center', mb: 0.5 }} spacing={1}>
                <Typography variant="subtitle2">{rows.length} model(s) will be updated</Typography>
                <Box sx={{ flexGrow: 1 }} />
                {renameable.length > 0 && (
                  <Button size="small" onClick={toggleAllRenames}>
                    {allRenamed ? 'Deselect all renames' : 'Select all renames'}
                  </Button>
                )}
              </Stack>
              <Stack spacing={0.5}>
                {rows.map((r) => (
                  <Stack
                    key={r.label}
                    direction="row"
                    spacing={1}
                    sx={{ alignItems: 'center', flexWrap: 'wrap' }}
                  >
                    {/* Rename control + the name that results */}
                    {r.nameDiffers ? (
                      <FormControlLabel
                        sx={{ minWidth: 240, mr: 0 }}
                        control={
                          <Checkbox
                            size="small"
                            checked={renameSet.has(r.label)}
                            onChange={() => toggleRename(r.label)}
                          />
                        }
                        label={
                          <Typography variant="body2">
                            {renameSet.has(r.label) ? (
                              <>
                                <Box
                                  component="span"
                                  sx={{ textDecoration: 'line-through', color: 'text.disabled' }}
                                >
                                  {r.modelName}
                                </Box>{' '}
                                → {r.label}
                              </>
                            ) : (
                              <>
                                {r.modelName}{' '}
                                <Typography
                                  component="span"
                                  variant="caption"
                                  color="text.secondary"
                                >
                                  (rename to “{r.label}”)
                                </Typography>
                              </>
                            )}
                          </Typography>
                        }
                      />
                    ) : (
                      <Typography variant="body2" sx={{ minWidth: 240 }}>
                        {r.modelName}
                      </Typography>
                    )}
                    {r.hasImage && <Chip size="small" label="image" variant="outlined" />}
                    {tagChips(r.addTags)}
                    {!r.hasImage && r.addTags.length === 0 && (
                      <Typography variant="caption" color="text.secondary">
                        nothing new
                      </Typography>
                    )}
                  </Stack>
                ))}
              </Stack>
            </Box>

            {/* Rows needing a manual choice */}
            {[...preview.ambiguous, ...preview.unmatched_patch].filter(
              (u) => !resolved[u.patch_name],
            ).length > 0 && (
              <Box>
                <Typography variant="subtitle2" gutterBottom>
                  Choose a model for these (or leave unset to skip)
                </Typography>
                {[...preview.ambiguous, ...preview.unmatched_patch].map((u) => {
                  // Ambiguous rows offer their candidates; fully unmatched offer everyone.
                  const options = u.candidates.length ? u.candidates : preview.members
                  return (
                    <Stack
                      key={u.patch_name}
                      direction="row"
                      spacing={1}
                      sx={{ alignItems: 'center', mb: 0.5, flexWrap: 'wrap' }}
                    >
                      <Typography variant="body2" sx={{ minWidth: 180 }}>
                        {u.patch_name}
                      </Typography>
                      <FormControl size="small" sx={{ minWidth: 260 }}>
                        <Select
                          displayEmpty
                          value={resolved[u.patch_name] ?? ''}
                          onChange={(e) =>
                            setResolved((r) => ({ ...r, [u.patch_name]: e.target.value }))
                          }
                          renderValue={(id) =>
                            id ? (membersById.get(id)?.name ?? '') : <em>skip</em>
                          }
                        >
                          <MenuItem value="">
                            <em>skip</em>
                          </MenuItem>
                          {options.map((m) => (
                            <MenuItem key={m.id} value={m.id}>
                              <Box>
                                <Typography variant="body2">{m.name}</Typography>
                                <Typography variant="caption" color="text.secondary">
                                  {m.tags.length ? m.tags.join(', ') : 'no tags'}
                                </Typography>
                              </Box>
                            </MenuItem>
                          ))}
                        </Select>
                      </FormControl>
                      {/* what would be applied to the (as yet unpicked) row */}
                      {tagChips(u.patch_tags)}
                      {u.has_image && <Chip size="small" label="image" variant="outlined" />}
                    </Stack>
                  )
                })}
              </Box>
            )}

            <Divider />

            {/* Global apply rules */}
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
