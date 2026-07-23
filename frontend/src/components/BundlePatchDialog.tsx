import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
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
  Tooltip,
  Typography,
} from '@mui/material'
import ReactMarkdown from 'react-markdown'
import { useQueryClient } from '@tanstack/react-query'

import { api, type PatchApplyOptions, type PatchMember, type PatchPreview } from '../api'
import Dropzone from './Dropzone'

/// Merge a scraped bundle patch onto this bundle. Drop the
/// zip; every patch model is one row — matched to a member, or a dropdown to pick
/// one by hand — with the tags it would add, whether it brings an image, and a
/// rename toggle. Tick what to apply, and apply.
///
/// Defaults encode the usual intent: the scraped photo replaces the auto render,
/// tags merge, and a name the patch improves on is renamed (all pre-ticked, since
/// that is normally what you want — untick the few you don't).
export default function BundlePatchDialog({
  bundleId,
  open,
  initialFiles,
  onClose,
  onApplied,
}: {
  bundleId: string
  open: boolean
  /** Zips already dropped on the page's inline box — preview them straight
   * away, so the dialog opens on the match table instead of its own drop step. */
  initialFiles?: File[] | null
  onClose: () => void
  onApplied: () => void
}) {
  const queryClient = useQueryClient()
  const [zips, setZips] = useState<File[]>([])
  const [preview, setPreview] = useState<PatchPreview | null>(null)
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const [done, setDone] = useState('')

  const [resolved, setResolved] = useState<Record<string, string>>({})
  const [renameSet, setRenameSet] = useState<Set<string>>(new Set())

  const [opts, setOpts] = useState<Omit<PatchApplyOptions, 'matches' | 'rename'>>({
    model_tags: 'merge',
    model_images: 'replace_generated',
    model_descriptions: true,
    bundle_cover: true,
    bundle_description: true,
  })

  const membersById = useMemo(() => {
    const m = new Map<string, PatchMember>()
    for (const mem of preview?.members ?? []) m.set(mem.id, mem)
    return m
  }, [preview])

  const reset = () => {
    setZips([])
    setPreview(null)
    setError('')
    setDone('')
    setResolved({})
    setRenameSet(new Set())
  }

  // Memoised so the auto-preview effect can depend on it without re-firing every
  // render; it only changes when the bundle does.
  const runPreview = useCallback(
    async (files: File[]) => {
      setZips(files)
      setBusy(true)
      setError('')
      try {
        const p = await api.previewBundlePatch(bundleId, files)
        setPreview(p)
        // Pre-tick a rename wherever the scraped name is one the matched model
        // isn't already known by — its current name or any of its aliases. The
        // scraped name is usually the better one; but a name already on the
        // model's alias list is a rename we've effectively seen and declined, so
        // don't offer it again.
        const knownNames = new Map(
          p.members.map((m) => [m.id, new Set([m.name, ...m.aliases].map((n) => n.toLowerCase()))]),
        )
        setRenameSet(
          new Set(
            p.matched
              .filter((m) => !knownNames.get(m.model_id)?.has(m.patch_name.toLowerCase()))
              .map((m) => String(m.key)),
          ),
        )
      } catch (err) {
        setError(err instanceof Error ? err.message : String(err))
      } finally {
        setBusy(false)
      }
    },
    [bundleId],
  )

  // When the parent hands us files already dropped on its inline box, preview
  // them the moment we open — once per drop, tracked by ref so a re-render
  // doesn't re-fire it; the ref resets on close so the next drop previews
  // afresh. Identity is the array the parent passed, which it replaces per drop.
  const previewedFiles = useRef<File[] | null>(null)
  useEffect(() => {
    if (!open) {
      previewedFiles.current = null
      return
    }
    if (initialFiles?.length && previewedFiles.current !== initialFiles) {
      previewedFiles.current = initialFiles
      void runPreview(initialFiles)
    }
  }, [open, initialFiles, runPreview])

  const runApply = async () => {
    if (!zips.length) return
    setBusy(true)
    setError('')
    try {
      const result = await api.applyBundlePatch(bundleId, zips, {
        ...opts,
        // Drop rows left on "skip": an empty value is not a member id, and the
        // server rejects the whole request trying to parse "" as a UUID.
        matches: Object.fromEntries(Object.entries(resolved).filter(([, v]) => v)),
        rename: [...renameSet],
      })
      await queryClient.invalidateQueries({ queryKey: ['bundle', bundleId] })
      onApplied()
      setDone(
        `Applied: ${result.models_updated} model(s) updated, ${result.tags_added} tag(s) added, ` +
          `${result.images_added} image(s) added, ${result.descriptions_added} description(s) set, ` +
          `${result.aliases_added} alias(es) recorded, ` +
          `${result.creator_refs_set} Creator ID(s) set, ` +
          `${result.custom_fields_set} custom field value(s) set.`,
      )
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setBusy(false)
    }
  }

  // One row per patch model. `auto` fixes the member; `choices` (present for
  // ambiguous / unmatched) offers a dropdown. The chosen member, add-tags and
  // rename state all derive from here so each model shows up exactly once.
  type Row = {
    key: string // stable identity (patch model index); names are not unique
    label: string // display name (patch_name)
    category: string | null
    patchTags: string[]
    hasImage: boolean
    hasDescription: boolean
    fixed?: { id: string; name: string }
    choices?: PatchMember[]
    /** how many leading `choices` are the suggested candidates (rest is every
     * other member, so a wrong or missing auto-match can always be corrected) */
    shortlist?: number
    /** the Creator ID this row would write — it replaces what the model has */
    creatorRef?: string | null
  }
  const rows: Row[] = useMemo(() => {
    if (!preview) return []
    const out: Row[] = preview.matched.map((m) => ({
      key: String(m.key),
      label: m.patch_name,
      category: m.category,
      patchTags: m.add_tags,
      hasImage: m.has_image,
      hasDescription: m.has_description,
      creatorRef: m.creator_ref,
      fixed: { id: m.model_id, name: m.model_name },
    }))
    for (const u of [...preview.ambiguous, ...preview.unmatched_patch]) {
      // Suggestions first, then every other member — the ambiguous candidate list
      // is only a shortlist, and the right model may not be on it (a versioned or
      // renamed member the matcher missed).
      const others = preview.members.filter((m) => !u.candidates.some((c) => c.id === m.id))
      out.push({
        key: String(u.key),
        label: u.patch_name,
        category: u.category,
        patchTags: u.patch_tags,
        hasImage: u.has_image,
        hasDescription: u.has_description,
        choices: [...u.candidates, ...others],
        shortlist: u.candidates.length,
      })
    }
    return out.sort((a, b) => a.label.localeCompare(b.label))
  }, [preview])

  // The member a row currently targets (fixed match, or the manual pick).
  const targetId = (r: Row) => r.fixed?.id ?? resolved[r.key] ?? ''
  // Tags that would actually be added: the patch's, minus what the target has.
  // (Matched rows already arrive pre-filtered; recompute for manual picks.)
  const addTags = (r: Row) => {
    if (r.fixed) return r.patchTags
    const have = new Set((membersById.get(targetId(r))?.tags ?? []).map((t) => t.toLowerCase()))
    return r.patchTags.filter((t) => !have.has(t.toLowerCase()))
  }
  // A name the target model already answers to — its current name or any alias.
  // Renaming to such a name is one we've effectively already declined, so it is
  // not offered.
  const isKnownName = (memberId: string, name: string) => {
    const m = membersById.get(memberId)
    if (!m) return false
    const lower = name.toLowerCase()
    return [m.name, ...m.aliases].some((n) => n.toLowerCase() === lower)
  }
  const nameDiffers = (r: Row) => {
    const id = targetId(r)
    return !!id && !isKnownName(id, r.label)
  }

  const renameable = rows.filter((r) => targetId(r) && nameDiffers(r)).map((r) => r.key)
  const allRenamed = renameable.length > 0 && renameable.every((k) => renameSet.has(k))
  const toggleAllRenames = () => setRenameSet(allRenamed ? new Set() : new Set(renameable))
  const toggleRename = (key: string) =>
    setRenameSet((s) => {
      const next = new Set(s)
      if (next.has(key)) next.delete(key)
      else next.add(key)
      return next
    })

  const pickMember = (row: Row, id: string) => {
    setResolved((r) => ({ ...r, [row.key]: id }))
    // Default the rename on when the pick renames, like the auto matches — but
    // not to a name the model already answers to.
    const differs = !!id && !isKnownName(id, row.label)
    setRenameSet((s) => {
      const next = new Set(s)
      if (differs) next.add(row.key)
      else next.delete(row.key)
      return next
    })
  }

  // Grid keeps the rename box, category, name and tags in aligned columns across
  // every row regardless of whether a row has a rename control or a dropdown.
  const GRID = {
    display: 'grid',
    gridTemplateColumns: '32px 88px minmax(180px, 300px) 1fr',
    alignItems: 'center',
    columnGap: 8,
  }
  // Strike a name when it is *not* the result: the old name when renaming, the new
  // one when not — so the target is always shown, just crossed out if it won't apply.
  const strike = (on: boolean) =>
    on ? { textDecoration: 'line-through', color: 'text.disabled' } : undefined

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
            label={busy ? 'Reading…' : 'Drop bundle-patch zips'}
            hint="patch.json + images — several at once merge into one apply"
            accept=".zip"
            multiple
            busy={busy}
            onDrop={(drop) => {
              const files = drop.files.map((f) => f.file)
              if (files.length) void runPreview(files)
            }}
          />
        ) : (
          <Stack spacing={2}>
            {/* A scrape carries whatever the shop page had, so a key nobody has
                defined a field for is worth saying out loud — but it never
                stops the rest of the patch applying. */}
            {preview.custom_field_warnings.length > 0 && (
              <Alert severity="warning">
                <Typography variant="body2" sx={{ mb: 0.5 }}>
                  {preview.custom_fields_applied > 0
                    ? `${preview.custom_fields_applied} custom field value(s) will be applied; these will be skipped:`
                    : 'These custom field values will be skipped:'}
                </Typography>
                <Stack direction="row" sx={{ flexWrap: 'wrap', gap: 0.5 }}>
                  {preview.custom_field_warnings.map((w, i) => (
                    <Chip
                      key={`${w.source}-${w.key}-${i}`}
                      size="small"
                      variant="outlined"
                      color="warning"
                      label={`${w.source}: ${w.key} — ${w.reason}`}
                    />
                  ))}
                </Stack>
              </Alert>
            )}
            {preview.files.length > 1 && (
              <Alert severity="info">
                Merged {preview.files.length} files into one apply:{' '}
                {preview.files.map((f) => `${f.name} (${f.models})`).join(', ')}
              </Alert>
            )}
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

            <Box>
              <Stack direction="row" sx={{ alignItems: 'center', mb: 0.5 }} spacing={1}>
                <Typography variant="subtitle2">Models</Typography>
                <Box sx={{ flexGrow: 1 }} />
                {renameable.length > 0 && (
                  <Button size="small" onClick={toggleAllRenames}>
                    {allRenamed ? 'Deselect all renames' : 'Select all renames'}
                  </Button>
                )}
              </Stack>

              <Box sx={{ borderTop: (t) => `1px solid ${t.palette.divider}` }}>
                {rows.map((r) => {
                  const id = targetId(r)
                  const renames = !!id && nameDiffers(r)
                  const willRename = renames && renameSet.has(r.key)
                  return (
                    <Box
                      key={r.key}
                      sx={{
                        ...GRID,
                        py: 0.75,
                        borderBottom: (t) => `1px solid ${t.palette.divider}`,
                      }}
                    >
                      {/* col 1 — rename checkbox, only where a rename is possible */}
                      {renames ? (
                        <Tooltip title={`Rename to “${r.label}”`}>
                          <Checkbox
                            size="small"
                            sx={{ p: 0 }}
                            checked={renameSet.has(r.key)}
                            onChange={() => toggleRename(r.key)}
                          />
                        </Tooltip>
                      ) : (
                        <Box />
                      )}

                      {/* col 2 — category, to tell same-named models apart */}
                      <Typography variant="caption" color="text.secondary" noWrap>
                        {r.category ?? ''}
                      </Typography>

                      {/* col 3 — the model. Auto matches show old → new; manual
                          matches keep their dropdown mounted so a wrong pick can be
                          changed, with the rename target shown beneath. */}
                      <Box sx={{ minWidth: 0 }}>
                        {r.fixed ? (
                          nameDiffers(r) ? (
                            <Typography variant="body2" noWrap>
                              <Box component="span" sx={strike(willRename)}>
                                {r.fixed.name}
                              </Box>
                              {' → '}
                              <Box component="span" sx={strike(!willRename)}>
                                {r.label}
                              </Box>
                            </Typography>
                          ) : (
                            <Typography variant="body2" noWrap>
                              {r.fixed.name}
                            </Typography>
                          )
                        ) : (
                          <>
                            <FormControl size="small" fullWidth>
                              <Select
                                displayEmpty
                                value={resolved[r.key] ?? ''}
                                onChange={(e) => pickMember(r, e.target.value)}
                                renderValue={(v) =>
                                  v ? (
                                    (membersById.get(v)?.name ?? '')
                                  ) : (
                                    <em>{r.label} — pick a model</em>
                                  )
                                }
                              >
                                <MenuItem value="">
                                  <em>skip</em>
                                </MenuItem>
                                {(r.choices ?? []).flatMap((m, i) => [
                                  ...(r.shortlist &&
                                  i === r.shortlist &&
                                  r.shortlist < (r.choices?.length ?? 0)
                                    ? [<Divider key="all-members" component="li" />]
                                    : []),
                                  <MenuItem key={m.id} value={m.id} sx={{ display: 'block' }}>
                                    <Box
                                      sx={{
                                        display: 'flex',
                                        alignItems: 'baseline',
                                        gap: 1,
                                        whiteSpace: 'nowrap',
                                        overflow: 'hidden',
                                        textOverflow: 'ellipsis',
                                      }}
                                    >
                                      <span>{m.name}</span>
                                      <Typography
                                        component="span"
                                        variant="caption"
                                        color="text.secondary"
                                        sx={{ overflow: 'hidden', textOverflow: 'ellipsis' }}
                                      >
                                        {m.tags.length ? m.tags.join(', ') : 'no tags'}
                                      </Typography>
                                    </Box>
                                  </MenuItem>,
                                ])}
                              </Select>
                            </FormControl>
                            {renames && (
                              <Typography
                                variant="caption"
                                component="div"
                                sx={{ mt: 0.25 }}
                                noWrap
                              >
                                {'renames to '}
                                <Box component="span" sx={strike(!willRename)}>
                                  {r.label}
                                </Box>
                              </Typography>
                            )}
                          </>
                        )}
                      </Box>

                      {/* col 4 — what it brings */}
                      <Stack
                        direction="row"
                        spacing={0.5}
                        sx={{ flexWrap: 'wrap', alignItems: 'center' }}
                      >
                        {r.hasImage && <Chip size="small" label="image" variant="outlined" />}
                        {r.hasDescription && (
                          <Chip size="small" label="description" variant="outlined" />
                        )}
                        {r.creatorRef && (
                          <Tooltip title="Creator ID — replaces what the model has">
                            <Chip size="small" label={r.creatorRef} variant="outlined" />
                          </Tooltip>
                        )}
                        {addTags(r).map((t) => (
                          <Chip
                            key={t}
                            size="small"
                            label={`+${t}`}
                            color="primary"
                            variant="outlined"
                          />
                        ))}
                        {id && !r.hasImage && !r.hasDescription && addTags(r).length === 0 && (
                          <Typography variant="caption" color="text.secondary">
                            nothing new
                          </Typography>
                        )}
                      </Stack>
                    </Box>
                  )
                })}
              </Box>
            </Box>

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
                  Model descriptions
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
