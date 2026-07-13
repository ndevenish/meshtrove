import { useState } from 'react'
import {
  Box,
  Container,
  Typography,
  Pagination,
  Button,
  Stack,
  Menu,
  MenuItem,
} from '@mui/material'
import AddIcon from '@mui/icons-material/Add'
import { useQuery } from '@tanstack/react-query'
import { useSearchParams } from 'react-router-dom'

import { api } from '../api'
import ModelCard from '../components/ModelCard'
import BundleCard from '../components/BundleCard'
import FilterSidebar from '../components/FilterSidebar'
import ModelEditDialog from '../components/ModelEditDialog'
import BundleEditDialog from '../components/BundleEditDialog'
import { useAuth } from '../main'

export default function BrowsePage() {
  const [params, setParams] = useSearchParams()
  const { user } = useAuth()
  const [createModelOpen, setCreateModelOpen] = useState(false)
  const [createBundleOpen, setCreateBundleOpen] = useState(false)
  const [createAnchor, setCreateAnchor] = useState<HTMLElement | null>(null)

  const { data, isLoading } = useQuery({
    queryKey: ['browse', params.toString()],
    queryFn: () => api.browse(new URLSearchParams(params)),
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
            Browse{' '}
            <Typography component="span" color="text.secondary">
              {data ? `(${data.total})` : ''}
            </Typography>
          </Typography>
          <Box sx={{ flexGrow: 1 }} />
          {canEdit && (
            <>
              <Button
                variant="contained"
                startIcon={<AddIcon />}
                onClick={(e) => setCreateAnchor(e.currentTarget)}
              >
                New
              </Button>
              <Menu
                anchorEl={createAnchor}
                open={createAnchor !== null}
                onClose={() => setCreateAnchor(null)}
              >
                <MenuItem
                  onClick={() => {
                    setCreateAnchor(null)
                    setCreateModelOpen(true)
                  }}
                >
                  New model
                </MenuItem>
                <MenuItem
                  onClick={() => {
                    setCreateAnchor(null)
                    setCreateBundleOpen(true)
                  }}
                >
                  New bundle
                </MenuItem>
              </Menu>
            </>
          )}
        </Stack>

        <Box
          sx={{
            display: 'grid',
            gridTemplateColumns: 'repeat(auto-fill, minmax(210px, 1fr))',
            gap: 2,
          }}
        >
          {(data?.items ?? []).map((item) =>
            item.type === 'bundle' ? (
              <BundleCard key={item.id} bundle={{ ...item, model_count: item.count }} />
            ) : (
              <ModelCard
                key={item.id}
                model={{
                  ...item,
                  variant_count: item.count,
                  like_count: item.like_count ?? 0,
                  matched_variant_ids: null,
                }}
              />
            ),
          )}
        </Box>

        {!isLoading && data?.items.length === 0 && (
          <Typography color="text.secondary" sx={{ mt: 6, textAlign: 'center' }}>
            Nothing matches. Try clearing filters{canEdit ? ' or add your first model' : ''}.
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
      <ModelEditDialog open={createModelOpen} onClose={() => setCreateModelOpen(false)} />
      <BundleEditDialog open={createBundleOpen} onClose={() => setCreateBundleOpen(false)} />
    </Container>
  )
}
