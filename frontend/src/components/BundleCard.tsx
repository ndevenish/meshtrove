import { Link } from 'react-router-dom'
import { Card, CardActionArea, CardContent, Typography, Box, Chip, Stack } from '@mui/material'
import Inventory2Icon from '@mui/icons-material/Inventory2'

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
    <Card variant="outlined" sx={{ height: '100%', position: 'relative' }}>
      <CardActionArea
        component={Link}
        to={`/bundles/${bundle.slug}`}
        sx={{ position: 'relative', zIndex: 2 }}
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
        <CardContent sx={{ pb: 0 }}>
          <Typography variant="subtitle1" sx={{ fontWeight: 600, lineHeight: 1.25 }} noWrap>
            {bundle.name}
          </Typography>
        </CardContent>
      </CardActionArea>
      {/* The card's text foot, outside the link: the heart rides the end of the
          creator line rather than sitting in a band of its own, so the tally
          reads as a property of the thing, level with the name it belongs to.
          The creator takes the slack and truncates, leaving the heart pinned to
          the right edge. */}
      <Box sx={{ px: 2, pt: 0.25, pb: 1.5 }}>
        <Stack direction="row" spacing={1} sx={{ alignItems: 'center' }}>
          <Typography
            variant="body2"
            color="text.secondary"
            noWrap
            sx={{ flexGrow: 1, minWidth: 0 }}
          >
            {bundle.creator_name ?? 'Unknown creator'}
          </Typography>
          {/* Lifted above the overlay link below so the heart keeps its own
              clicks while the rest of the foot navigates. */}
          <Box sx={{ position: 'relative', zIndex: 2, display: 'flex' }}>
            <LikeButton
              kind="bundle"
              id={bundle.id}
              liked={bundle.liked}
              likeCount={bundle.like_count}
            />
          </Box>
        </Stack>
        {bundle.tags.length > 0 && (
          <Stack direction="row" spacing={1} sx={{ mt: 1 }}>
            {bundle.tags.slice(0, 2).map((tag) => (
              <Chip key={tag} label={tag} size="small" variant="outlined" />
            ))}
          </Stack>
        )}
      </Box>
      {/* A full-card click target. It sits beneath the CardActionArea (which
          keeps its own ripple over the image and name) but above the passive
          foot, so the creator line and its surrounding padding open the bundle
          too — everything but the heart, which is lifted back on top. */}
      <Box
        component={Link}
        to={`/bundles/${bundle.slug}`}
        aria-hidden
        tabIndex={-1}
        sx={{ position: 'absolute', inset: 0, zIndex: 1 }}
      />
    </Card>
  )
}
