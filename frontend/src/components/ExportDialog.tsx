import { useMemo, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import {
  Dialog,
  DialogTitle,
  DialogContent,
  DialogActions,
  Button,
  Typography,
  Stack,
  Alert,
  Chip,
  Checkbox,
  FormControlLabel,
  FormGroup,
  Autocomplete,
  TextField,
  Box,
  Divider,
  CircularProgress,
} from '@mui/material'
import { keepPreviousData, useQuery, useQueryClient } from '@tanstack/react-query'

import { api, type BundleDetail, type ModelDetail, type ExportRequest } from '../api'

/// Build an export archive. For a bundle: pick which members go in (filter the
/// list by model tag). For both: filter which variants by variant tag (positive
/// or negative — exclude "supported" is unsupported only) and which file kinds.
/// A live preview shows how many variants each choice keeps and how many files
/// each kind governs. Building is async, so this queues a job and sends the user
/// to the Exports page.
export default function ExportDialog({
  open,
  onClose,
  bundle,
  model,
}: {
  open: boolean
  onClose: () => void
  bundle?: BundleDetail
  model?: ModelDetail
}) {
  const navigate = useNavigate()
  const queryClient = useQueryClient()

  const members = bundle?.models ?? []
  // Default to nothing selected: the count then always matches what's ticked,
  // with no hidden selections lurking behind the tag filter.
  const [selected, setSelected] = useState<Set<string>>(new Set())
  const [tagFilter, setTagFilter] = useState<string[]>([])
  const [include, setInclude] = useState<string[]>([])
  const [exclude, setExclude] = useState<string[]>([])
  const [excludedKinds, setExcludedKinds] = useState<Set<string>>(new Set())
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  const { data: variantTags } = useQuery({
    queryKey: ['variant-tags'],
    queryFn: () => api.variantTags(),
    enabled: open,
  })
  const variantTagNames = (variantTags ?? []).map((t) => t.name)

  const memberTags = useMemo(() => {
    const set = new Set<string>()
    for (const m of members) m.tags.forEach((t) => set.add(t))
    return [...set].sort()
  }, [members])

  const visible = useMemo(
    () =>
      tagFilter.length === 0
        ? members
        : members.filter((m) => m.tags.some((t) => tagFilter.includes(t))),
    [members, tagFilter],
  )

  const modelIds = bundle ? [...selected].sort() : model ? [model.id] : []

  // Live preview of what the selection + variant filter keeps. File-kind counts
  // are pre-exclusion, so toggling a kind doesn't need a refetch.
  const previewBody: ExportRequest = {
    bundle_id: bundle?.id,
    model_ids: modelIds,
    variant_include: include,
    variant_exclude: exclude,
  }
  const { data: preview } = useQuery({
    queryKey: ['export-preview', previewBody],
    queryFn: () => api.exportPreview(previewBody),
    enabled: open && modelIds.length > 0,
    placeholderData: keepPreviousData,
  })
  const variantCount = useMemo(() => {
    const m = new Map<string, { kept: number; total: number }>()
    for (const r of preview?.models ?? [])
      m.set(r.id, { kept: r.variants_kept, total: r.variants_total })
    return m
  }, [preview])

  const toggle = (id: string) =>
    setSelected((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  const setAllVisible = (on: boolean) =>
    setSelected((prev) => {
      const next = new Set(prev)
      for (const m of visible) {
        if (on) next.add(m.id)
        else next.delete(m.id)
      }
      return next
    })
  const toggleKind = (kind: string) =>
    setExcludedKinds((prev) => {
      const next = new Set(prev)
      if (next.has(kind)) next.delete(kind)
      else next.add(kind)
      return next
    })

  const submit = async () => {
    setError('')
    setBusy(true)
    try {
      await api.createExport({
        ...previewBody,
        name: bundle?.name ?? model?.name,
        file_kinds_exclude: [...excludedKinds],
      })
      await queryClient.invalidateQueries({ queryKey: ['exports'] })
      onClose()
      navigate('/exports')
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
      setBusy(false)
    }
  }

  const scope = tagFilter.length ? 'shown' : 'all'

  return (
    <Dialog open={open} onClose={onClose} maxWidth="sm" fullWidth>
      <DialogTitle>Export {bundle ? 'bundle' : 'model'}</DialogTitle>
      <DialogContent>
        <Stack spacing={2} sx={{ mt: 1 }}>
          {error && <Alert severity="error">{error}</Alert>}

          {bundle && (
            <>
              <Typography variant="body2" color="text.secondary">
                Choose which member models to include.
              </Typography>
              {memberTags.length > 0 && (
                <Autocomplete
                  multiple
                  size="small"
                  options={memberTags}
                  value={tagFilter}
                  onChange={(_, v) => setTagFilter(v)}
                  renderInput={(p) => (
                    <TextField {...p} label="Filter members by tag" placeholder="tag…" />
                  )}
                />
              )}
              <Stack direction="row" spacing={1} sx={{ alignItems: 'center' }}>
                <Typography variant="body2" sx={{ flexGrow: 1 }}>
                  {selected.size} of {members.length} selected
                </Typography>
                <Button size="small" onClick={() => setAllVisible(true)}>
                  Select {scope}
                </Button>
                <Button size="small" onClick={() => setAllVisible(false)}>
                  Clear {scope}
                </Button>
              </Stack>
              <Box
                sx={{
                  maxHeight: 240,
                  overflowY: 'auto',
                  border: 1,
                  borderColor: 'divider',
                  borderRadius: 1,
                  p: 1,
                }}
              >
                <Stack spacing={0}>
                  {visible.map((m) => {
                    const vc = variantCount.get(m.id)
                    return (
                      <FormControlLabel
                        key={m.id}
                        sx={{ alignItems: 'center' }}
                        control={
                          <Checkbox
                            size="small"
                            checked={selected.has(m.id)}
                            onChange={() => toggle(m.id)}
                          />
                        }
                        label={
                          <Stack
                            direction="row"
                            spacing={0.5}
                            sx={{ alignItems: 'center', flexWrap: 'wrap' }}
                          >
                            <span>{m.name}</span>
                            {m.tags.map((t) => (
                              <Chip
                                key={t}
                                label={t}
                                size="small"
                                variant="outlined"
                                sx={{ height: 18 }}
                              />
                            ))}
                            {selected.has(m.id) && vc && (
                              <Typography variant="caption" color="text.secondary">
                                · {vc.kept === vc.total ? `${vc.total}` : `${vc.kept}/${vc.total}`}{' '}
                                variant
                                {vc.total === 1 ? '' : 's'}
                              </Typography>
                            )}
                          </Stack>
                        }
                      />
                    )
                  })}
                  {visible.length === 0 && (
                    <Typography variant="body2" color="text.secondary" sx={{ p: 1 }}>
                      No members match that tag.
                    </Typography>
                  )}
                </Stack>
              </Box>
              <Divider />
            </>
          )}

          <Typography variant="body2" color="text.secondary">
            Which variants to include. Leave blank for all; use <em>exclude</em> for negatives (e.g.
            exclude <code>supported</code> to export the unsupported variants only).
          </Typography>
          <Autocomplete
            multiple
            freeSolo
            size="small"
            options={variantTagNames}
            value={include}
            onChange={(_, v) => setInclude(v)}
            renderInput={(p) => (
              <TextField {...p} label="Only variants tagged" placeholder="variant tag…" />
            )}
          />
          <Autocomplete
            multiple
            freeSolo
            size="small"
            options={variantTagNames}
            value={exclude}
            onChange={(_, v) => setExclude(v)}
            renderInput={(p) => (
              <TextField {...p} label="Exclude variants tagged" placeholder="variant tag…" />
            )}
          />

          {/* Variant summary: what's there, and what the filter keeps. */}
          {preview && preview.variants.length > 0 && (
            <Box>
              <Typography variant="subtitle2" sx={{ mb: 0.5 }}>
                Variants in selection
              </Typography>
              <Stack direction="row" spacing={0.5} sx={{ flexWrap: 'wrap', gap: 0.5 }}>
                {preview.variants.map((v) => (
                  <Chip
                    key={v.label}
                    size="small"
                    variant={v.kept ? 'filled' : 'outlined'}
                    color={v.kept ? 'primary' : 'default'}
                    label={`${v.label} × ${v.count}`}
                    sx={v.kept ? undefined : { textDecoration: 'line-through', opacity: 0.6 }}
                  />
                ))}
              </Stack>
            </Box>
          )}

          {/* File-kind filter. */}
          {preview && preview.file_kinds.length > 0 && (
            <Box>
              <Typography variant="subtitle2" sx={{ mb: 0.5 }}>
                File types to include
              </Typography>
              <FormGroup row>
                {preview.file_kinds.map((k) => (
                  <FormControlLabel
                    key={k.kind}
                    control={
                      <Checkbox
                        size="small"
                        checked={!excludedKinds.has(k.kind)}
                        onChange={() => toggleKind(k.kind)}
                      />
                    }
                    label={
                      <Typography variant="body2">
                        {k.kind} ({k.count})
                      </Typography>
                    }
                  />
                ))}
              </FormGroup>
            </Box>
          )}
        </Stack>
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose} disabled={busy}>
          Cancel
        </Button>
        <Button
          variant="contained"
          onClick={() => void submit()}
          disabled={busy || modelIds.length === 0}
          startIcon={busy ? <CircularProgress size={16} color="inherit" /> : undefined}
        >
          Export
        </Button>
      </DialogActions>
    </Dialog>
  )
}
