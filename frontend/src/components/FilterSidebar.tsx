import { Box, Typography, Chip, Divider } from '@mui/material'
import { useQuery } from '@tanstack/react-query'
import { useSearchParams } from 'react-router-dom'

import { api } from '../api'

/// Sidebar filters: two chip clouds over two vocabularies — what a model IS
/// (tags) and which edition of it you want (variant tags). Selecting several
/// variant tags requires ONE variant to carry them all, so 32mm + unsupported
/// will not match a model that has those tags on different variants.
export default function FilterSidebar() {
  const [params, setParams] = useSearchParams()

  const activeTags = (params.get('tags') ?? '').split(',').filter(Boolean)
  const activeVariantTags = (params.get('vtags') ?? '').split(',').filter(Boolean)
  const search = params.get('q') ?? ''

  // Counts reflect the current selection: each chip shows how many models would
  // remain if it were added, so narrowing filters the numbers down. The
  // selection is in the query key, so the clouds refetch as chips are toggled.
  const selection = { tags: activeTags, vtags: activeVariantTags, q: search }
  const { data: tags } = useQuery({
    queryKey: ['tags', activeTags, activeVariantTags, search],
    queryFn: () => api.tags(selection),
  })
  const { data: variantTags } = useQuery({
    queryKey: ['variant-tags', activeTags, activeVariantTags, search],
    queryFn: () => api.variantTags(selection),
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
      <Typography variant="subtitle2" sx={{ mb: 1.5, textTransform: 'uppercase', opacity: 0.7 }}>
        Tags
      </Typography>
      <Box sx={{ display: 'flex', flexWrap: 'wrap', gap: 0.75 }}>
        {(tags ?? []).map((tag) => {
          const active = activeTags.includes(tag.name)
          return (
            <Chip
              key={tag.id}
              label={`${tag.name} (${tag.model_count})`}
              size="small"
              color={active ? 'primary' : 'default'}
              variant={active ? 'filled' : 'outlined'}
              onClick={() => toggle('tags', activeTags, tag.name)}
              sx={{ opacity: tag.model_count === 0 && !active ? 0.1 : 1 }}
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
