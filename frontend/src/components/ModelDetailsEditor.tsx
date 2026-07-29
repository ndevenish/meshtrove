import { forwardRef, useImperativeHandle, useState } from 'react'
import { createPortal } from 'react-dom'
import { useNavigate } from 'react-router-dom'
import { Alert, Autocomplete, Stack, TextField } from '@mui/material'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import { api, type ModelDetail } from '../api'
import { changeTags, pasteTags } from '../tags'
import { useSuppressGlobalDrop } from '../globalDrop'
import { CustomFieldControl, useCustomFieldDraft } from './CustomFieldControl'

/// What the page can ask of the editor. Save and Cancel live in the page header,
/// where the Edit button was — leaving the mode should be where entering it was,
/// not buried at the bottom of a form — so the buttons are up there and the state
/// is down here, and this is the seam between them.
export type DetailsEditorHandle = {
  save: () => Promise<void>
}

/// The model's fields, edited in place on the page they live on. The dialog that
/// used to do this still exists — it is how a model is *created*, where there is
/// no page to edit yet — but for one that already exists, editing it a modal away
/// from the thing you are editing was always a strange way round.
const ModelDetailsEditor = forwardRef<
  DetailsEditorHandle,
  {
    model: ModelDetail
    /** Saved, or cancelled: either way, edit mode is over. */
    onDone: () => void
    onBusyChange?: (busy: boolean) => void
    /** Where to put the tags control, if not here: the page lays the columns
        out, and on a model that means tags belong under the gallery, well away
        from the fields that say which model this is. The draft still lives here
        with the rest of the form — only the box it draws in is the page's. */
    tagsSlot?: HTMLElement | null
  }
>(function ModelDetailsEditor({ model, onDone, onBusyChange, tagsSlot }, ref) {
  const queryClient = useQueryClient()
  const navigate = useNavigate()
  // A file-kind field puts a drop target on this page; while one is up, the
  // app-wide "drop anywhere to import" overlay has to stand aside or it swallows
  // the drop and stages an import instead.
  useSuppressGlobalDrop(model.custom_fields.some((e) => e.field.kind === 'file'))
  const [name, setName] = useState(model.name)
  const [creatorName, setCreatorName] = useState(model.creator_name ?? '')
  const [creatorRef, setCreatorRef] = useState(model.creator_ref ?? '')
  const [version, setVersion] = useState(model.model_version ?? '')
  const [tags, setTags] = useState<string[]>(model.tags)
  const [sourceUrl, setSourceUrl] = useState(model.source_url ?? '')
  const [description, setDescription] = useState(model.description_md ?? '')
  const [error, setError] = useState('')
  const customFields = useCustomFieldDraft(model.custom_fields)

  const { data: creators } = useQuery({ queryKey: ['creators'], queryFn: () => api.creators() })
  const { data: allTags } = useQuery({ queryKey: ['tags'], queryFn: () => api.tags() })

  const busy = (value: boolean) => onBusyChange?.(value)

  const save = async () => {
    if (!name.trim()) {
      setError('A model needs a name')
      throw new Error('A model needs a name')
    }
    busy(true)
    setError('')
    try {
      // A creator typed by hand may not exist yet — get-or-create, so the library
      // ends up with one row per creator rather than one per spelling.
      const typed = creatorName.trim()
      let creator_id: string | null = null
      if (typed) {
        const existing = (creators ?? []).find((c) => c.name.toLowerCase() === typed.toLowerCase())
        creator_id = existing ? existing.id : (await api.createCreator({ name: typed })).id
      }
      const saved = await api.updateModel(model.id, {
        name: name.trim(),
        creator_id,
        creator_ref: creatorRef.trim() || null,
        model_version: version.trim() || null,
        source_url: sourceUrl.trim() || null,
        tags,
        custom_fields: customFields.payload(),
      })
      // Descriptions are immutable revisions: an edit inserts a new one, so only
      // write when it actually changed — otherwise every save grows the history
      // with a copy of what was already there.
      if (description !== (model.description_md ?? '')) {
        await api.updateDescription('models', model.id, description)
      }
      // A rename moves the slug, and with it the URL: go to the canonical slug
      // and refetch there. The old slug in the address bar no longer resolves,
      // so we must navigate rather than refetch it. When the slug is unchanged
      // this is a no-op navigation and the invalidate just refreshes in place.
      await queryClient.invalidateQueries({ queryKey: ['model', saved.slug] })
      await queryClient.invalidateQueries({ queryKey: ['creators'] })
      await queryClient.invalidateQueries({ queryKey: ['tags'] })
      navigate(`/models/${saved.slug}`, { replace: true })
      onDone()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
      throw err
    } finally {
      busy(false)
    }
  }

  useImperativeHandle(ref, () => ({ save }))

  const tagsField = (
    <Autocomplete
      multiple
      freeSolo
      options={(allTags ?? []).map((t) => t.name)}
      value={tags}
      onChange={changeTags(setTags)}
      renderInput={(props) => (
        <TextField
          {...props}
          onPaste={pasteTags(tags, setTags)}
          label="Tags"
          placeholder="add tag…"
        />
      )}
    />
  )

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
      <Autocomplete
        freeSolo
        options={(creators ?? []).map((c) => c.name)}
        value={creatorName}
        onInputChange={(_, value) => setCreatorName(value)}
        renderInput={(props) => <TextField {...props} label="Creator (author / company / site)" />}
      />
      {/* Both are the creator's own words for *which* release this is — short
          fields, and a line each of their own was a line wasted. */}
      <Stack direction={{ xs: 'column', sm: 'row' }} spacing={2}>
        <TextField
          label="Creator ID"
          value={creatorRef}
          onChange={(e) => setCreatorRef(e.target.value)}
          placeholder="the creator's own id / SKU"
          sx={{ flex: 1, minWidth: 0 }}
        />
        <TextField
          label="Version"
          value={version}
          onChange={(e) => setVersion(e.target.value)}
          placeholder="v2, 2024 rework"
          sx={{ flex: 1, minWidth: 0 }}
        />
      </Stack>
      {/* A portal, so the tags draft stays in this component's state while the
          control itself draws in the page's left column. */}
      {tagsSlot ? createPortal(tagsField, tagsSlot) : tagsField}
      <TextField
        label="Source URL"
        value={sourceUrl}
        onChange={(e) => setSourceUrl(e.target.value)}
      />
      {/* A file-kind field writes itself the moment something is dropped on it
          — there are no bytes to hold in a form — so it doesn't wait for save. */}
      {model.custom_fields.map((entry) => (
        <CustomFieldControl
          key={entry.field.id}
          entry={entry}
          value={customFields.valueOf(entry)}
          onChange={(value) => customFields.setValue(entry, value)}
          onUploadFile={async (file) => {
            const form = new FormData()
            form.append('file', file)
            await api.uploadCustomFieldFile('models', model.id, entry.field.id, form)
            await queryClient.invalidateQueries({ queryKey: ['model', model.slug] })
          }}
          onClearFile={async () => {
            await api.clearCustomField('models', model.id, entry.field.id)
            await queryClient.invalidateQueries({ queryKey: ['model', model.slug] })
          }}
        />
      ))}
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

export default ModelDetailsEditor
