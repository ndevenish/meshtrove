import { Box, Container, Typography, Pagination } from '@mui/material'
import { useQuery } from '@tanstack/react-query'
import { useSearchParams } from 'react-router-dom'

import { api } from '../api'
import ItemGrid from '../components/ItemGrid'
import { useAuth } from '../main'

/// Everything the logged-in user has liked, newest like first — what the front
/// page's "Liked" row is a single-row window onto.
export default function LikesPage() {
  const [params, setParams] = useSearchParams()
  const { user, loading } = useAuth()
  const page = Number(params.get('page') ?? '1')

  const { data, isLoading } = useQuery({
    queryKey: ['likes', 'page', page],
    queryFn: () => api.likes(new URLSearchParams({ page: String(page) })),
    enabled: !!user,
  })

  if (!loading && !user) {
    return (
      <Container maxWidth="xl" sx={{ py: 3 }}>
        <Typography color="text.secondary">Log in to keep a list of favourites.</Typography>
      </Container>
    )
  }

  const pageCount = data ? Math.max(1, Math.ceil(data.total / data.per_page)) : 1

  return (
    <Container maxWidth="xl" sx={{ py: 3 }}>
      <Typography variant="h5" sx={{ mb: 2 }}>
        Liked{' '}
        <Typography component="span" color="text.secondary">
          {data ? `(${data.total})` : ''}
        </Typography>
      </Typography>

      <ItemGrid items={data?.items ?? []} />

      {!isLoading && data?.items.length === 0 && (
        <Typography color="text.secondary" sx={{ mt: 6, textAlign: 'center' }}>
          Nothing liked yet. Tap the heart on a card to keep it here.
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
    </Container>
  )
}
