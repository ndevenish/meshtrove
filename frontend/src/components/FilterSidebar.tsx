import { useState } from 'react'
import {
  Box,
  Typography,
  Chip,
  Divider,
  TextField,
  FormControlLabel,
  Switch,
  Checkbox,
} from '@mui/material'
import { useQuery } from '@tanstack/react-query'
import { useSearchParams } from 'react-router-dom'

import { api, decodeCustomFieldFilter, encodeCustomFieldFilter } from '../api'
import { useAuth } from '../main'

/// Sidebar filters: two chip clouds over two vocabularies — what a model IS
/// (tags) and which edition of it you want (variant tags). Selecting several
/// variant tags requires ONE variant to carry them all, so 32mm + unsupported
/// will not match a model that has those tags on different variants.
export default function FilterSidebar() {
  const [params, setParams] = useSearchParams()
  const { user } = useAuth()
  const isAdmin = user?.role === 'admin'
  // Client-side name filter over the tag cloud — the vocabulary can grow large,
  // and typing a few letters narrows the chips without touching the selection.
  const [tagFilter, setTagFilter] = useState('')

  const activeTags = (params.get('tags') ?? '').split(',').filter(Boolean)
  const activeVariantTags = (params.get('vtags') ?? '').split(',').filter(Boolean)
  const search = params.get('q') ?? ''
  // Admin escape hatch (URL-backed, so it also drives the browse grid): reveal
  // hidden-tagged items and the tags that only live on them. Only honoured for
  // admins — the toggle isn't even rendered otherwise, and the API re-checks.
  const showHidden = isAdmin && params.get('show_hidden') === '1'

  // The browse sidebar's custom-field filters, kept in one `cf` URL param so
  // they drive the grid (via the browse query) and the clouds alike.
  const cf = decodeCustomFieldFilter(params.get('cf'))
  const cfKey = params.get('cf') ?? ''

  // Counts reflect the current selection: each chip shows how many models would
  // remain if it were added, so narrowing filters the numbers down. The
  // selection is in the query key, so the clouds refetch as chips are toggled.
  const selection = { tags: activeTags, vtags: activeVariantTags, q: search, showHidden, cf }
  const { data: tags } = useQuery({
    queryKey: ['tags', activeTags, activeVariantTags, search, showHidden, cfKey],
    queryFn: () => api.tags(selection),
  })
  const { data: variantTags } = useQuery({
    queryKey: ['variant-tags', activeTags, activeVariantTags, search, cfKey],
    queryFn: () => api.variantTags(selection),
  })
  // The filterable custom-field vocabulary (visibility-gated server-side).
  const { data: fields } = useQuery({
    queryKey: ['filterable-cf'],
    queryFn: () => api.filterableCustomFields(),
  })

  const update = (mutate: (next: URLSearchParams) => void) => {
    const next = new URLSearchParams(params)
    mutate(next)
    next.delete('page')
    setParams(next)
  }

  const toggle = (key: 'tags' | 'vtags', active: string[], tag: string) =>
    update((next) => {
      const set = new Set(active)
      if (set.has(tag)) set.delete(tag)
      else set.add(tag)
      if (set.size) next.set(key, [...set].join(','))
      else next.delete(key)
    })

  // Replace one field's selected tokens, re-encoding the whole `cf` param (or
  // dropping it when nothing is left, so the URL and cache key stay clean).
  const setCf = (fieldKey: string, tokens: string[]) =>
    update((next) => {
      const nextCf = { ...cf, [fieldKey]: tokens }
      if (!tokens.length) delete nextCf[fieldKey]
      const encoded = encodeCustomFieldFilter(nextCf)
      if (encoded) next.set('cf', encoded)
      else next.delete('cf')
    })

  // Add/remove one token from a multi-select field (choice, rating).
  const toggleCf = (fieldKey: string, token: string) => {
    const set = new Set(cf[fieldKey] ?? [])
    if (set.has(token)) set.delete(token)
    else set.add(token)
    setCf(fieldKey, [...set])
  }

  return (
    <Box sx={{ width: 240, flexShrink: 0, pr: 3 }}>
      {variantTags && variantTags.length > 0 && (
        <>
          <Typography
            variant="subtitle2"
            sx={{ mb: 1.5, textTransform: 'uppercase', opacity: 0.7 }}
          >
            Variants
          </Typography>
          <Box sx={{ display: 'flex', flexWrap: 'wrap', gap: 0.75 }}>
            {variantTags.map((tag) => {
              const active = activeVariantTags.includes(tag.name)
              return (
                <Chip
                  key={tag.id}
                  label={`${tag.name} (${tag.variant_count})`}
                  size="small"
                  color={active ? 'primary' : 'default'}
                  variant={active ? 'filled' : 'outlined'}
                  onClick={() => toggle('vtags', activeVariantTags, tag.name)}
                  // A dead end under the current selection — adding it would empty
                  // the grid. Faded, not hidden, so the vocabulary stays put
                  // instead of reshuffling as you narrow.
                  sx={{ opacity: tag.variant_count === 0 && !active ? 0.4 : 1 }}
                />
              )
            })}
          </Box>
          <Divider sx={{ my: 2 }} />
        </>
      )}
      {(fields ?? []).map((field) => {
        const selected = cf[field.key] ?? []
        // Checkbox and file are a single on/off: a ticked custom checkbox, or
        // "has a file". Both carry the sentinel token ['1'] when on.
        if (field.kind === 'checkbox' || field.kind === 'file') {
          return (
            <FormControlLabel
              key={field.id}
              control={
                <Checkbox
                  size="small"
                  checked={selected.length > 0}
                  onChange={() => setCf(field.key, selected.length ? [] : ['1'])}
                />
              }
              label={field.kind === 'file' ? `Has ${field.name}` : field.name}
              slotProps={{ typography: { variant: 'body2' } }}
              sx={{ mb: 1, display: 'block' }}
            />
          )
        }
        // Choice and rating are chip clouds, multi-select (OR), with an explicit
        // unset option carried as the empty-string token.
        const options: { token: string; label: string }[] =
          field.kind === 'choice'
            ? [
                ...(field.options.choices ?? []).map((c) => ({ token: c, label: c })),
                { token: '', label: 'No choice' },
              ]
            : [
                ...Array.from({ length: field.options.max ?? 5 }, (_, i) => ({
                  token: String(i + 1),
                  label: `${i + 1}★`,
                })),
                { token: '', label: 'No rating' },
              ]
        return (
          <Box key={field.id} sx={{ mb: 2 }}>
            <Typography
              variant="subtitle2"
              sx={{ mb: 1, textTransform: 'uppercase', opacity: 0.7 }}
            >
              {field.name}
            </Typography>
            <Box sx={{ display: 'flex', flexWrap: 'wrap', gap: 0.75 }}>
              {options.map((o) => {
                const active = selected.includes(o.token)
                return (
                  <Chip
                    key={o.token || '∅'}
                    label={o.label}
                    size="small"
                    color={active ? 'primary' : 'default'}
                    variant={active ? 'filled' : 'outlined'}
                    onClick={() => toggleCf(field.key, o.token)}
                  />
                )
              })}
            </Box>
          </Box>
        )
      })}
      {(fields ?? []).length > 0 && <Divider sx={{ my: 2 }} />}
      <Typography variant="subtitle2" sx={{ mb: 1.5, textTransform: 'uppercase', opacity: 0.7 }}>
        Tags
      </Typography>
      {isAdmin && (
        <FormControlLabel
          control={
            <Switch
              size="small"
              checked={showHidden}
              onChange={() =>
                update((next) => {
                  if (showHidden) next.delete('show_hidden')
                  else next.set('show_hidden', '1')
                })
              }
            />
          }
          label="Show hidden"
          slotProps={{ typography: { variant: 'body2' } }}
          sx={{ mb: 1, display: 'block' }}
        />
      )}
      {(tags ?? []).length > 0 && (
        <TextField
          size="small"
          fullWidth
          placeholder="Filter tags…"
          value={tagFilter}
          onChange={(e) => setTagFilter(e.target.value)}
          sx={{ mb: 1.5 }}
        />
      )}
      <Box sx={{ display: 'flex', flexWrap: 'wrap', gap: 0.75 }}>
        {(tags ?? [])
          .filter((tag) => {
            const q = tagFilter.trim().toLowerCase()
            // Keep an active chip visible even when it falls outside the filter,
            // so a selection never silently disappears as you type.
            return !q || tag.name.toLowerCase().includes(q) || activeTags.includes(tag.name)
          })
          .map((tag) => {
            const active = activeTags.includes(tag.name)
            return (
              <Chip
                key={tag.id}
                label={`${tag.name} (${tag.model_count})`}
                size="small"
                color={active ? 'primary' : 'default'}
                variant={active ? 'filled' : 'outlined'}
                onClick={() => toggle('tags', activeTags, tag.name)}
                // Hidden tags only ever surface here for an admin with "Show
                // hidden" on; a dashed outline flags them as not publicly visible.
                sx={{
                  opacity: tag.model_count === 0 && !active ? 0.1 : 1,
                  ...(tag.hidden ? { borderStyle: 'dashed' } : {}),
                }}
              />
            )
          })}
        {tags?.length === 0 && (
          <Typography variant="body2" color="text.secondary">
            No tags yet
          </Typography>
        )}
      </Box>
    </Box>
  )
}
