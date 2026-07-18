import { Link } from 'react-router-dom'
import { Card, CardActionArea, CardContent, Typography, Box, Chip, Stack } from '@mui/material'
import Inventory2Icon from '@mui/icons-material/Inventory2'
import FavoriteIcon from '@mui/icons-material/Favorite'

import { type BundleSummary, imageUrl } from '../api'
import LikeButton from './LikeButton'

/// A bundle in the browse grid. Mirrors ModelCard but links to /bundles and
/// shows the member-model count; a "Bundle" chip makes the mixed grid scannable.
type BundleCardData = Pick<
  BundleSummary,
  'id' | 'slug' | 'name' | 'creator_name' | 'primary_image_id' | 'model_count' | 'tags'
> & { liked: boolean; like_count: number }

export default function BundleCard({ bundle }: { bundle: BundleCardData }) {
  return (
    // `position: relative` so the like button, which is not part of the link,
    // can be pinned to the corner of the artwork.
    <Card variant="outlined" sx={{ height: '100%', position: 'relative' }}>
      <CardActionArea
        component={Link}
        to={`/bundles/${bundle.slug}`}
        sx={{ height: '100%', display: 'flex', flexDirection: 'column', alignItems: 'stretch' }}
      >
        <Box
          sx={{
            aspectRatio: '1',
            backgroundColor: 'action.hover',
            display: 'flex',
            alignItems: 'center',
            justifyContent: 'center',
            overflow: 'hidden',
            position: 'relative',
          }}
        >
          {bundle.primary_image_id ? (
            <Box
              component="img"
              src={imageUrl(bundle.primary_image_id)}
              alt={bundle.name}
              loading="lazy"
              sx={{ width: '100%', height: '100%', objectFit: 'cover' }}
            />
          ) : (
            <Inventory2Icon sx={{ fontSize: 64, opacity: 0.25 }} />
          )}
          <Chip
            label="Bundle"
            size="small"
            color="primary"
            sx={{ position: 'absolute', top: 8, left: 8, height: 20 }}
          />
          {/* What the card is standing in for. On the front page a bundle takes
              the place of its members, so the one thing it owes the reader is how
              many models are behind it. Sits on the image, with its own scrim —
              a preview photo can be any colour underneath. */}
          <Typography
            variant="caption"
            sx={{
              position: 'absolute',
              bottom: 8,
              left: 8,
              px: 0.75,
              py: 0.25,
              borderRadius: 1,
              lineHeight: 1.4,
              color: 'common.white',
              backgroundColor: 'rgba(0, 0, 0, 0.6)',
            }}
          >
            {bundle.model_count} model{bundle.model_count === 1 ? '' : 's'}
          </Typography>
        </Box>
        <CardContent sx={{ flexGrow: 1, width: '100%', pb: '12px !important' }}>
          <Typography variant="subtitle1" sx={{ fontWeight: 600, lineHeight: 1.25 }} noWrap>
            {bundle.name}
          </Typography>
          <Typography variant="body2" color="text.secondary" noWrap>
            {bundle.creator_name ?? 'Unknown creator'}
          </Typography>
          <Stack direction="row" spacing={1} sx={{ alignItems: 'center', mt: 0.75 }}>
            {/* Matches ModelCard: the public tally, hidden at zero. */}
            {bundle.like_count > 0 && (
              <>
                <FavoriteIcon sx={{ fontSize: 16, color: 'primary.main' }} />
                <Typography variant="caption">{bundle.like_count}</Typography>
              </>
            )}
            <Box sx={{ flexGrow: 1 }} />
            {bundle.tags.slice(0, 2).map((tag) => (
              <Chip key={tag} label={tag} size="small" variant="outlined" />
            ))}
          </Stack>
        </CardContent>
      </CardActionArea>
      <LikeButton kind="bundle" id={bundle.id} liked={bundle.liked} />
    </Card>
  )
}
