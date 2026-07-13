import { Link } from 'react-router-dom'
import { Card, CardActionArea, CardContent, Typography, Box, Chip, Stack } from '@mui/material'
import FavoriteIcon from '@mui/icons-material/Favorite'
import ViewInArIcon from '@mui/icons-material/ViewInAr'

import { type ModelSummary, imageUrl } from '../api'

export default function ModelCard({ model }: { model: ModelSummary }) {
  return (
    <Card variant="outlined" sx={{ height: '100%' }}>
      <CardActionArea
        component={Link}
        to={`/models/${model.id}`}
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
        <CardContent sx={{ flexGrow: 1, width: '100%', pb: '12px !important' }}>
          <Typography variant="subtitle1" sx={{ fontWeight: 600, lineHeight: 1.25 }} noWrap>
            {model.name}
          </Typography>
          <Typography variant="body2" color="text.secondary" noWrap>
            {model.creator_name ?? 'Unknown creator'}
          </Typography>
          <Stack direction="row" spacing={1} sx={{ alignItems: 'center', mt: 0.75 }}>
            <FavoriteIcon sx={{ fontSize: 16, color: 'primary.main' }} />
            <Typography variant="caption">{model.like_count}</Typography>
            <Typography variant="caption" color="text.secondary">
              · {model.variant_count} variant{model.variant_count === 1 ? '' : 's'}
            </Typography>
            <Box sx={{ flexGrow: 1 }} />
            {model.tags.slice(0, 2).map((tag) => (
              <Chip key={tag} label={tag} size="small" variant="outlined" />
            ))}
          </Stack>
        </CardContent>
      </CardActionArea>
    </Card>
  )
}
