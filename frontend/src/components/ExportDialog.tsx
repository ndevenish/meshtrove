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
  Autocomplete,
  TextField,
  Box,
  Divider,
  CircularProgress,
} from '@mui/material'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import { api, type BundleDetail, type ModelDetail } from '../api'

/// Build an export archive. For a bundle: pick which members go in (filter the
/// list by model tag), and which of their variants (by variant tag, positive or
/// negative — "exclude: supported" is unsupported only). For a model: just the
/// variant filter. Building is async, so this queues a job and sends the user to
/// the Exports page.
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
  const [selected, setSelected] = useState<Set<string>>(
    () => new Set((bundle?.models ?? []).map((m) => m.id)),
  )
  const [tagFilter, setTagFilter] = useState<string[]>([])
  const [include, setInclude] = useState<string[]>([])
  const [exclude, setExclude] = useState<string[]>([])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  const { data: variantTags } = useQuery({
    queryKey: ['variant-tags'],
    queryFn: () => api.variantTags(),
    enabled: open,
  })
  const variantTagNames = (variantTags ?? []).map((t) => t.name)

  // Model-tag vocabulary for the member filter: what the members actually carry.
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

  const modelIds = bundle ? [...selected] : model ? [model.id] : []

  const submit = async () => {
    setError('')
    setBusy(true)
    try {
      await api.createExport({
        name: bundle?.name ?? model?.name,
        bundle_id: bundle?.id,
        model_ids: modelIds,
        variant_include: include,
        variant_exclude: exclude,
      })
      await queryClient.invalidateQueries({ queryKey: ['exports'] })
      onClose()
      navigate('/exports')
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
      setBusy(false)
    }
  }

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
                  Select {tagFilter.length ? 'shown' : 'all'}
                </Button>
                <Button size="small" onClick={() => setAllVisible(false)}>
                  Clear {tagFilter.length ? 'shown' : 'all'}
                </Button>
              </Stack>
              <Box
                sx={{
                  maxHeight: 260,
                  overflowY: 'auto',
                  border: 1,
                  borderColor: 'divider',
                  borderRadius: 1,
                  p: 1,
                }}
              >
                <Stack spacing={0}>
                  {visible.map((m) => (
                    <FormControlLabel
                      key={m.id}
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
                        </Stack>
                      }
                    />
                  ))}
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
