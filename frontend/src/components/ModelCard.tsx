import { Link } from 'react-router-dom'
import { Card, CardActionArea, CardContent, Typography, Box, Chip, Stack } from '@mui/material'
import ViewInArIcon from '@mui/icons-material/ViewInAr'

import { type ModelSummary, imageUrl } from '../api'
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
    // A column so the foot row — tags and the like button, the parts that are
    // not the link — can sit below the CardActionArea and still be flush with
    // the bottom of a card stretched to match its neighbours.
    <Card variant="outlined" sx={{ height: '100%', display: 'flex', flexDirection: 'column' }}>
      <CardActionArea
        component={Link}
        to={`/models/${model.slug}`}
        sx={{ flexGrow: 1, display: 'flex', flexDirection: 'column', alignItems: 'stretch' }}
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
              src={imageUrl(model.primary_image_id)}
              alt={model.name}
              loading="lazy"
              sx={{ width: '100%', height: '100%', objectFit: 'cover' }}
            />
          ) : (
            <ViewInArIcon sx={{ fontSize: 64, opacity: 0.25 }} />
          )}
        </Box>
        <CardContent sx={{ flexGrow: 1, width: '100%', pb: 0 }}>
          <Typography variant="subtitle1" sx={{ fontWeight: 600, lineHeight: 1.25 }} noWrap>
            {model.name}
          </Typography>
          {!hideCreator && (
            <Typography variant="body2" color="text.secondary" noWrap>
              {model.creator_name ?? 'Unknown creator'}
            </Typography>
          )}
        </CardContent>
      </CardActionArea>
      {/* The card's base. `minHeight` holds the row open when a card has
          neither tags nor likes, so the feet of a grid row stay level. */}
      <Stack
        direction="row"
        spacing={1}
        sx={{ alignItems: 'center', px: 2, pt: 0.75, pb: 1.5, minHeight: 30 }}
      >
        {model.tags.slice(0, 2).map((tag) => (
          <Chip key={tag} label={tag} size="small" variant="outlined" />
        ))}
        <Box sx={{ flexGrow: 1 }} />
        <LikeButton kind="model" id={model.id} liked={model.liked} likeCount={model.like_count} />
      </Stack>
    </Card>
  )
}
