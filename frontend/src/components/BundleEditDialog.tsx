import { useEffect, useState } from 'react'
import {
  Dialog,
  DialogTitle,
  DialogContent,
  DialogActions,
  Button,
  TextField,
  Stack,
  Autocomplete,
  Alert,
  MenuItem,
} from '@mui/material'
import { useQuery, useQueryClient } from '@tanstack/react-query'
import { useNavigate } from 'react-router-dom'

import { api, type BundleDetail } from '../api'

/// Create (no `bundle`) or edit a bundle's metadata and tags. Trimmed clone of
/// ModelEditDialog — bundles have a `kind` but no price/variant/archive fields.
export default function BundleEditDialog({
  open,
  onClose,
  bundle,
}: {
  open: boolean
  onClose: () => void
  bundle?: BundleDetail
}) {
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const [name, setName] = useState('')
  const [creatorName, setCreatorName] = useState('')
  const [kind, setKind] = useState('purchased')
  const [sourceUrl, setSourceUrl] = useState('')
  const [tags, setTags] = useState<string[]>([])
  const [description, setDescription] = useState('')
  const [error, setError] = useState('')
  const [busy, setBusy] = useState(false)

  const { data: creators } = useQuery({ queryKey: ['creators'], queryFn: () => api.creators() })
  const { data: allTags } = useQuery({ queryKey: ['tags'], queryFn: () => api.tags() })

  useEffect(() => {
    if (open) {
      setName(bundle?.name ?? '')
      setCreatorName(bundle?.creator_name ?? '')
      setKind(bundle?.kind ?? 'purchased')
      setSourceUrl(bundle?.source_url ?? '')
      setTags(bundle?.tags ?? [])
      setDescription(bundle?.description_md ?? '')
      setError('')
    }
  }, [open, bundle])

  const submit = async () => {
    setBusy(true)
    setError('')
    try {
      let creator_id: string | null = bundle?.creator_id ?? null
      const trimmed = creatorName.trim()
      if (!trimmed) {
        creator_id = null
      } else {
        const existing = creators?.find((c) => c.name.toLowerCase() === trimmed.toLowerCase())
        creator_id = existing ? existing.id : (await api.createCreator({ name: trimmed })).id
      }

      const body = {
        name,
        creator_id,
        kind,
        source_url: sourceUrl || null,
        tags,
        description_md: bundle ? undefined : description || null,
      }
      let saved: BundleDetail
      if (bundle) {
        saved = await api.updateBundle(bundle.id, body)
        if (description !== (bundle.description_md ?? '')) {
          await api.updateDescription('bundles', bundle.id, description)
        }
      } else {
        saved = await api.createBundle(body)
      }
      await queryClient.invalidateQueries()
      onClose()
      navigate(`/bundles/${saved.id}`)
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e))
    } finally {
      setBusy(false)
    }
  }

  return (
    <Dialog open={open} onClose={onClose} maxWidth="sm" fullWidth>
      <DialogTitle>{bundle ? 'Edit bundle' : 'New bundle'}</DialogTitle>
      <DialogContent>
        <Stack spacing={2} sx={{ mt: 1 }}>
          {error && <Alert severity="error">{error}</Alert>}
          <TextField
            label="Name"
            value={name}
            onChange={(e) => setName(e.target.value)}
            autoFocus
            required
          />
          <TextField select label="Kind" value={kind} onChange={(e) => setKind(e.target.value)}>
            <MenuItem value="purchased">Purchased (a bought pack)</MenuItem>
            <MenuItem value="collection">Collection (a personal grouping)</MenuItem>
          </TextField>
          <Autocomplete
            freeSolo
            options={(creators ?? []).map((c) => c.name)}
            value={creatorName}
            onInputChange={(_, value) => setCreatorName(value)}
            renderInput={(params) => (
              <TextField {...params} label="Creator (author / company / site)" />
            )}
          />
          <Autocomplete
            multiple
            freeSolo
            options={(allTags ?? []).map((t) => t.name)}
            value={tags}
            onChange={(_, value) => setTags(value)}
            renderInput={(params) => <TextField {...params} label="Tags" placeholder="add tag…" />}
          />
          <TextField
            label="Source URL"
            value={sourceUrl}
            onChange={(e) => setSourceUrl(e.target.value)}
          />
          {!bundle && (
            <TextField
              label="Description (markdown)"
              value={description}
              onChange={(e) => setDescription(e.target.value)}
              multiline
              minRows={4}
            />
          )}
        </Stack>
      </DialogContent>
      <DialogActions>
        <Button onClick={onClose}>Cancel</Button>
        <Button variant="contained" onClick={submit} disabled={busy || !name.trim()}>
          {bundle ? 'Save' : 'Create'}
        </Button>
      </DialogActions>
    </Dialog>
  )
}
