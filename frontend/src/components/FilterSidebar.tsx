import {
  Box,
  Typography,
  Chip,
  Stack,
  FormControl,
  Select,
  MenuItem,
  InputLabel,
  Divider,
} from '@mui/material'
import { useQuery } from '@tanstack/react-query'
import { useSearchParams } from 'react-router-dom'

import { api } from '../api'

/// Sidebar filters: tag chips plus one dropdown per declared variant axis —
/// entirely data-driven from the axis/option tables.
export default function FilterSidebar() {
  const [params, setParams] = useSearchParams()
  const { data: tags } = useQuery({ queryKey: ['tags'], queryFn: () => api.tags() })
  const { data: axes } = useQuery({ queryKey: ['axes'], queryFn: () => api.axes() })

  const activeTags = (params.get('tags') ?? '').split(',').filter(Boolean)
  const activeOpts = new Map(
    (params.get('opts') ?? '')
      .split(',')
      .filter(Boolean)
      .map((pair) => pair.split(':') as [string, string]),
  )

  const update = (mutate: (next: URLSearchParams) => void) => {
    const next = new URLSearchParams(params)
    mutate(next)
    next.delete('page')
    setParams(next)
  }

  const toggleTag = (tag: string) =>
    update((next) => {
      const set = new Set(activeTags)
      if (set.has(tag)) set.delete(tag)
      else set.add(tag)
      if (set.size) next.set('tags', [...set].join(','))
      else next.delete('tags')
    })

  const setOpt = (axis: string, value: string) =>
    update((next) => {
      const opts = new Map(activeOpts)
      if (value) opts.set(axis, value)
      else opts.delete(axis)
      if (opts.size) next.set('opts', [...opts.entries()].map(([a, v]) => `${a}:${v}`).join(','))
      else next.delete('opts')
    })

  return (
    <Box sx={{ width: 240, flexShrink: 0, pr: 3 }}>
      {axes && axes.length > 0 && (
        <>
          <Typography
            variant="subtitle2"
            sx={{ mb: 1.5, textTransform: 'uppercase', opacity: 0.7 }}
          >
            Variants
          </Typography>
          <Stack spacing={2}>
            {axes.map((axis) => (
              <FormControl key={axis.id} size="small" fullWidth>
                <InputLabel>{axis.name}</InputLabel>
                <Select
                  label={axis.name}
                  value={activeOpts.get(axis.name) ?? ''}
                  onChange={(e) => setOpt(axis.name, e.target.value)}
                >
                  <MenuItem value="">
                    <em>any</em>
                  </MenuItem>
                  {axis.options.map((option) => (
                    <MenuItem key={option.id} value={option.value}>
                      {option.value}
                    </MenuItem>
                  ))}
                </Select>
              </FormControl>
            ))}
          </Stack>
          <Divider sx={{ my: 2 }} />
        </>
      )}
      <Typography variant="subtitle2" sx={{ mb: 1.5, textTransform: 'uppercase', opacity: 0.7 }}>
        Tags
      </Typography>
      <Box sx={{ display: 'flex', flexWrap: 'wrap', gap: 0.75 }}>
        {(tags ?? []).map((tag) => (
          <Chip
            key={tag.id}
            label={`${tag.name} (${tag.model_count})`}
            size="small"
            color={activeTags.includes(tag.name) ? 'primary' : 'default'}
            variant={activeTags.includes(tag.name) ? 'filled' : 'outlined'}
            onClick={() => toggleTag(tag.name)}
          />
        ))}
        {tags?.length === 0 && (
          <Typography variant="body2" color="text.secondary">
            No tags yet
          </Typography>
        )}
      </Box>
    </Box>
  )
}
