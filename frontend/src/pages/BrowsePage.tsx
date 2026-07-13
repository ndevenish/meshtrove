import { useState } from 'react'
import { Box, Container, Typography, Pagination, Button, Stack } from '@mui/material'
import AddIcon from '@mui/icons-material/Add'
import { useQuery } from '@tanstack/react-query'
import { useSearchParams } from 'react-router-dom'

import { api } from '../api'
import ModelCard from '../components/ModelCard'
import FilterSidebar from '../components/FilterSidebar'
import ModelEditDialog from '../components/ModelEditDialog'
import { useAuth } from '../main'

export default function BrowsePage() {
  const [params, setParams] = useSearchParams()
  const { user } = useAuth()
  const [createOpen, setCreateOpen] = useState(false)

  const { data, isLoading } = useQuery({
    queryKey: ['models', params.toString()],
    queryFn: () => api.searchModels(new URLSearchParams(params)),
  })

  const page = Number(params.get('page') ?? '1')
  const pageCount = data ? Math.max(1, Math.ceil(data.total / data.per_page)) : 1
  const canEdit = user && user.role !== 'viewer'

  return (
    <Container maxWidth="xl" sx={{ py: 3, display: 'flex' }}>
      <FilterSidebar />
      <Box sx={{ flexGrow: 1 }}>
        <Stack direction="row" sx={{ alignItems: 'center', mb: 2 }}>
          <Typography variant="h5">
            Models{' '}
            <Typography component="span" color="text.secondary">
              {data ? `(${data.total})` : ''}
            </Typography>
          </Typography>
          <Box sx={{ flexGrow: 1 }} />
          {canEdit && (
            <Button variant="contained" startIcon={<AddIcon />} onClick={() => setCreateOpen(true)}>
              New model
            </Button>
          )}
        </Stack>

        <Box
          sx={{
            display: 'grid',
            gridTemplateColumns: 'repeat(auto-fill, minmax(210px, 1fr))',
            gap: 2,
          }}
        >
          {(data?.models ?? []).map((model) => (
            <ModelCard key={model.id} model={model} />
          ))}
        </Box>

        {!isLoading && data?.models.length === 0 && (
          <Typography color="text.secondary" sx={{ mt: 6, textAlign: 'center' }}>
            No models match. Try clearing filters{canEdit ? ' or add your first model' : ''}.
          </Typography>
        )}

        {pageCount > 1 && (
          <Box sx={{ display: 'flex', justifyContent: 'center', mt: 3 }}>
            <Pagination
              count={pageCount}
              page={page}
              color="primary"
              onChange={(_, value) => {
                const next = new URLSearchParams(params)
                next.set('page', String(value))
                setParams(next)
              }}
            />
          </Box>
        )}
      </Box>
      <ModelEditDialog open={createOpen} onClose={() => setCreateOpen(false)} />
    </Container>
  )
}
