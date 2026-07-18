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
  Link as MuiLink,
} from '@mui/material'
import AddIcon from '@mui/icons-material/Add'
import ChevronRightIcon from '@mui/icons-material/ChevronRight'
import { useQuery } from '@tanstack/react-query'
import { useSearchParams, Link } from 'react-router-dom'

import { api } from '../api'
import ItemGrid from '../components/ItemGrid'
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

  // The liked row is a *shortcut back to your own things*, which only makes
  // sense on the plain front page: once there is a query or a filter, the page
  // is answering a question, and a fixed row of favourites on top of the answer
  // is noise. Same reason it stays on page 1 only.
  const showLiked =
    !!user && page === 1 && !params.get('q') && !params.get('tags') && !params.get('vtags')

  // One row's worth is a viewport-dependent number; ask for enough to fill the
  // widest plausible one and let the grid clip the rest.
  const { data: liked } = useQuery({
    queryKey: ['likes', 'row'],
    queryFn: () => api.likes(new URLSearchParams({ per_page: '12' })),
    enabled: showLiked,
  })

  const likedItems = showLiked && liked ? liked.items : []

  const likedHeading = (
    <MuiLink
      component={Link}
      to="/likes"
      underline="none"
      color="inherit"
      sx={{ display: 'inline-flex', alignItems: 'center' }}
    >
      <Typography variant="h5">Liked</Typography>
      <Typography component="span" color="text.secondary" sx={{ ml: 1 }}>
        ({liked?.total})
      </Typography>
      <ChevronRightIcon sx={{ color: 'text.secondary' }} />
    </MuiLink>
  )

  const browseHeading = (
    <Typography variant="h5">
      Browse{' '}
      <Typography component="span" color="text.secondary">
        {data ? `(${data.total})` : ''}
      </Typography>
    </Typography>
  )

  // "New" belongs to the page, not to the Browse section it used to sit in:
  // when a liked row appears it would ride the Browse heading halfway down the
  // page. So it shares a line with whichever heading comes first instead.
  const newButton = canEdit && (
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
  )

  return (
    <Container maxWidth="xl" sx={{ py: 3, display: 'flex' }}>
      <FilterSidebar />
      <Box sx={{ flexGrow: 1 }}>
        <Stack direction="row" sx={{ alignItems: 'center', mb: 2 }}>
          {likedItems.length > 0 ? likedHeading : browseHeading}
          <Box sx={{ flexGrow: 1 }} />
          {newButton}
        </Stack>

        {likedItems.length > 0 && (
          <>
            <Box sx={{ mb: 4 }}>
              <ItemGrid items={likedItems} singleRow />
            </Box>
            <Box sx={{ mb: 2 }}>{browseHeading}</Box>
          </>
        )}

        <ItemGrid items={data?.items ?? []} />

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
