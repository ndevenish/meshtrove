import { forwardRef, useImperativeHandle, useState } from 'react'
import { useNavigate } from 'react-router-dom'
import { Alert, Autocomplete, MenuItem, Stack, TextField } from '@mui/material'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import { api, type BundleDetail } from '../api'
import type { DetailsEditorHandle } from './ModelDetailsEditor'

/// The bundle's fields, edited in place. Mirrors ModelDetailsEditor — a bundle
/// has a `kind` and no variants, and is otherwise the same handful of facts.
const BundleDetailsEditor = forwardRef<
  DetailsEditorHandle,
  {
    bundle: BundleDetail
    onDone: () => void
    onBusyChange?: (busy: boolean) => void
  }
>(function BundleDetailsEditor({ bundle, onDone, onBusyChange }, ref) {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  const [name, setName] = useState(bundle.name)
  const [kind, setKind] = useState(bundle.kind)
  const [creatorName, setCreatorName] = useState(bundle.creator_name ?? '')
  const [tags, setTags] = useState<string[]>(bundle.tags)
  const [sourceUrl, setSourceUrl] = useState(bundle.source_url ?? '')
  const [description, setDescription] = useState(bundle.description_md ?? '')
  const [error, setError] = useState('')

  const { data: creators } = useQuery({ queryKey: ['creators'], queryFn: () => api.creators() })
  const { data: allTags } = useQuery({ queryKey: ['tags'], queryFn: () => api.tags() })

  const save = async () => {
    if (!name.trim()) {
      setError('A bundle needs a name')
      throw new Error('A bundle needs a name')
    }
    onBusyChange?.(true)
    setError('')
    try {
      const typed = creatorName.trim()
      let creator_id: string | null = null
      if (typed) {
        const existing = (creators ?? []).find((c) => c.name.toLowerCase() === typed.toLowerCase())
        creator_id = existing ? existing.id : (await api.createCreator({ name: typed })).id
      }
      const saved = await api.updateBundle(bundle.id, {
        name: name.trim(),
        creator_id,
        kind,
        source_url: sourceUrl.trim() || null,
        tags,
      })
      // A description edit inserts a revision; only write when it changed.
      if (description !== (bundle.description_md ?? '')) {
        await api.updateDescription('bundles', bundle.id, description)
      }
      // A rename moves the slug and the URL — go to the canonical slug and
      // refetch there, since the old slug no longer resolves (see
      // ModelDetailsEditor). A no-op navigation when the slug is unchanged.
      await queryClient.invalidateQueries({ queryKey: ['bundle', saved.slug] })
      await queryClient.invalidateQueries({ queryKey: ['creators'] })
      await queryClient.invalidateQueries({ queryKey: ['tags'] })
      navigate(`/bundles/${saved.slug}`, { replace: true })
      onDone()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
      throw err
    } finally {
      onBusyChange?.(false)
    }
  }

  useImperativeHandle(ref, () => ({ save }))

  return (
    <Stack spacing={2} sx={{ mb: 2 }}>
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
        <MenuItem value="collection">Collection (models grouped by hand)</MenuItem>
      </TextField>
      <Autocomplete
        freeSolo
        options={(creators ?? []).map((c) => c.name)}
        value={creatorName}
        onInputChange={(_, value) => setCreatorName(value)}
        renderInput={(props) => <TextField {...props} label="Creator (author / company / site)" />}
      />
      <Autocomplete
        multiple
        freeSolo
        options={(allTags ?? []).map((t) => t.name)}
        value={tags}
        onChange={(_, value) => setTags(value)}
        renderInput={(props) => <TextField {...props} label="Tags" placeholder="add tag…" />}
      />
      <TextField
        label="Source URL"
        value={sourceUrl}
        onChange={(e) => setSourceUrl(e.target.value)}
      />
      <TextField
        label="Description (markdown)"
        value={description}
        onChange={(e) => setDescription(e.target.value)}
        multiline
        minRows={4}
      />
    </Stack>
  )
})

export default BundleDetailsEditor
