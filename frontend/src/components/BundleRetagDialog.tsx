import { useMemo, useState } from 'react'
import {
  Alert,
  Autocomplete,
  Button,
  Chip,
  Dialog,
  DialogActions,
  DialogContent,
  DialogTitle,
  Stack,
  TextField,
  Typography,
} from '@mui/material'
import { useQuery } from '@tanstack/react-query'
import { api, type BundleDetail } from '../api'
import { changeTags, pasteTags } from '../tags'

/// Add and/or remove model tags across every member of a bundle at once.
///
/// Deliberately not a "set the tags on all members" editor. Members arrive
/// carrying their own tags from imports and scraped patches, so a wholesale
/// overwrite would quietly destroy per-model detail; add and remove compose to
/// any state the user actually wants, one visible step at a time.
export default function BundleRetagDialog({
  open,
  onClose,
  bundle,
  onDone,
}: {
  open: boolean
  onClose: () => void
  bundle: BundleDetail
  onDone: (message: string) => void
}) {
  const [add, setAdd] = useState<string[]>([])
  const [remove, setRemove] = useState<string[]>([])
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')

  const { data: allTags } = useQuery({ queryKey: ['tags'], queryFn: () => api.tags() })

  // Removal only offers tags the members actually carry — the global tag list
  // is mostly noise here, and a tag no member has would be a no-op anyway.
  const memberTags = useMemo(() => {
    const seen = new Map<string, string>()
    for (const model of bundle.models) {
      for (const tag of model.tags) {
        if (!seen.has(tag.toLowerCase())) seen.set(tag.toLowerCase(), tag)
      }
    }
    return [...seen.values()].sort((a, b) => a.localeCompare(b))
  }, [bundle.models])

  // Preview the effect before writing: members already carrying an added tag are
  // untouched, so "add supported" across a bundle where half have it reads
  // honestly as half. Counts assignments, matching what the server reports back.
  const preview = useMemo(() => {
    const lower = (tags: string[]) => new Set(tags.map((t) => t.toLowerCase()))
    const adding = lower(add)
    // A tag in both lists is an add — mirrors the server, which lets the add win.
    const removing = new Set([...lower(remove)].filter((t) => !adding.has(t)))
    let added = 0
    let removed = 0
    const touched = new Set<string>()
    for (const model of bundle.models) {
      const has = lower(model.tags)
      const gains = [...adding].filter((t) => !has.has(t)).length
      const loses = [...removing].filter((t) => has.has(t)).length
      added += gains
      removed += loses
      if (gains || loses) touched.add(model.id)
    }
    return { added, removed, models: touched.size }
  }, [add, remove, bundle.models])

  const apply = async () => {
    setBusy(true)
    setError('')
    try {
      const result = await api.retagBundleMembers(bundle.id, add, remove)
      onDone(
        `${result.tags_added} added, ${result.tags_removed} removed across ` +
          `${result.models_updated} model${result.models_updated === 1 ? '' : 's'}`,
      )
      setAdd([])
      setRemove([])
      onClose()
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }

  const nothingToDo = preview.added === 0 && preview.removed === 0

  return (
    <Dialog open={open} onClose={onClose} fullWidth maxWidth="sm">
      <DialogTitle>Tag all models in this bundle</DialogTitle>
      <DialogContent>
        <Stack spacing={2} sx={{ mt: 1 }}>
          <Typography variant="body2" color="text.secondary">
            Applies to all {bundle.models.length} member model
            {bundle.models.length === 1 ? '' : 's'}. Existing tags are kept — this adds and removes,
            it never replaces.
          </Typography>
          <Autocomplete
            multiple
            freeSolo
            options={(allTags ?? []).map((t) => t.name)}
            value={add}
            onChange={changeTags(setAdd)}
            renderInput={(params) => (
              <TextField
                {...params}
                onPaste={pasteTags(add, setAdd)}
                label="Add to every model"
                placeholder="add tag…"
              />
            )}
          />
          <Autocomplete
            multiple
            freeSolo
            options={memberTags}
            value={remove}
            onChange={changeTags(setRemove)}
            renderInput={(params) => (
              <TextField
                {...params}
                onPaste={pasteTags(remove, setRemove)}
                label="Remove from every model"
                placeholder="remove tag…"
              />
            )}
          />
          {!nothingToDo && (
            <Stack direction="row" spacing={1} sx={{ alignItems: 'center', flexWrap: 'wrap' }}>
              {/* A zero side is left out rather than shown as "+0": a pure
                  removal shouldn't display a green chip claiming an addition. */}
              {preview.added > 0 && (
                <Chip size="small" color="success" label={`+${preview.added}`} />
              )}
              {preview.removed > 0 && (
                <Chip size="small" color="warning" label={`−${preview.removed}`} />
              )}
              <Typography variant="body2" color="text.secondary">
                across {preview.models} model{preview.models === 1 ? '' : 's'}
              </Typography>
            </Stack>
          )}
          {nothingToDo && (add.length > 0 || remove.length > 0) && (
            <Typography variant="body2" color="text.secondary">
              Every member is already in that state — nothing to change.
            </Typography>
          )}
          {error && <Alert severity="error">{error}</Alert>}
        </Stack>
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose} disabled={busy}>
          Cancel
        </Button>
        <Button variant="contained" onClick={() => void apply()} disabled={busy || nothingToDo}>
          Apply
        </Button>
      </DialogActions>
    </Dialog>
  )
}
