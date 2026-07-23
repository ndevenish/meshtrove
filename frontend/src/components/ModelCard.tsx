import { Link } from 'react-router-dom'
import { Card, CardActionArea, CardContent, Typography, Box, Chip, Stack } from '@mui/material'
import ViewInArIcon from '@mui/icons-material/ViewInAr'

import { type ModelSummary, squareImageUrl } from '../api'
import LikeButton from './LikeButton'

export default function ModelCard({
  model,
  hideCreator = false,
}: {
  model: ModelSummary
  /** Suppress the creator line — e.g. inside a bundle view whose members all
      share the bundle's creator, where repeating it on every card is noise. */
  hideCreator?: boolean
}) {
  return (
    <Card variant="outlined" sx={{ height: '100%', position: 'relative' }}>
      <CardActionArea
        component={Link}
        to={`/models/${model.slug}`}
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
          }}
        >
          {model.primary_image_id ? (
            <Box
              component="img"
              src={squareImageUrl(model.primary_image_id)}
              alt={model.name}
              loading="lazy"
              sx={{ width: '100%', height: '100%', objectFit: 'cover' }}
            />
          ) : (
            <ViewInArIcon sx={{ fontSize: 64, opacity: 0.25 }} />
          )}
        </Box>
        <CardContent sx={{ pb: 0 }}>
          <Typography variant="subtitle1" sx={{ fontWeight: 600, lineHeight: 1.25 }} noWrap>
            {model.name}
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
          {hideCreator ? (
            <Box sx={{ flexGrow: 1 }} />
          ) : (
            <Typography
              variant="body2"
              color="text.secondary"
              noWrap
              sx={{ flexGrow: 1, minWidth: 0 }}
            >
              {model.creator_name ?? 'Unknown creator'}
            </Typography>
          )}
          {/* Lifted above the overlay link below so the heart keeps its own
              clicks while the rest of the foot navigates. */}
          <Box sx={{ position: 'relative', zIndex: 2, display: 'flex' }}>
            <LikeButton
              kind="model"
              id={model.id}
              liked={model.liked}
              likeCount={model.like_count}
            />
          </Box>
        </Stack>
        {model.tags.length > 0 && (
          <Stack direction="row" spacing={1} sx={{ mt: 1 }}>
            {model.tags.slice(0, 2).map((tag) => (
              <Chip key={tag} label={tag} size="small" variant="outlined" />
            ))}
          </Stack>
        )}
      </Box>
      {/* A full-card click target. It sits beneath the CardActionArea (which
          keeps its own ripple over the image and name) but above the passive
          foot, so the creator line and its surrounding padding open the model
          too — everything but the heart, which is lifted back on top. */}
      <Box
        component={Link}
        to={`/models/${model.slug}`}
        aria-hidden
        tabIndex={-1}
        sx={{ position: 'absolute', inset: 0, zIndex: 1 }}
      />
    </Card>
  )
}
