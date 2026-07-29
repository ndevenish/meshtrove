import { useCallback, useMemo, useState } from 'react'
import { useNavigate, useParams, Link } from 'react-router-dom'
import { Alert, Box, Button, Container, Paper, Stack, TextField, Typography } from '@mui/material'
import ArrowBackIcon from '@mui/icons-material/ArrowBack'
import ContentCutIcon from '@mui/icons-material/ContentCut'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import { api, type LayoutPlan, type LayoutSpec, type PlanTarget } from '../api'
import ImportLayoutPanel, { AnnotatedFileList } from '../components/ImportLayoutPanel'
import { useDocumentTitle } from '../documentTitle'
import { clearImportDraft } from '../importDraft'
import { useAuth } from '../main'

/// Carving a model that is already in the library: run an import layout over
/// the files it is holding and re-shape them in place.
///
/// Its own page rather than a corner of the model editor, because it is the
/// import page's carve panel doing the work — the rule list, the role table, the
/// value maps and the annotated file list are a workbench, not a form field, and
/// they want the width.
///
/// The layout does two things at once, which is the whole point:
///
/// * a file that captures **no model name** stays on this model, sorted into the
///   variant its variant tags describe — the plain "split this into variants" case;
/// * a file that **does** capture one was never this model, and leaves as a model
///   of its own.
///
/// Split models are pieces of one purchase, so the carve gathers every model it
/// produced — this one included — into a bundle, and that is where the page then
/// goes. Carve nothing out and it goes back to the model.
export default function CarvePage() {
  const { id } = useParams<{ id: string }>()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  const { user } = useAuth()
  const [layout, setLayout] = useState<{ spec: LayoutSpec; plan: LayoutPlan } | null>(null)
  const [bundleName, setBundleName] = useState('')
  const [bundleNameEdited, setBundleNameEdited] = useState(false)
  const [carving, setCarving] = useState(false)
  const [error, setError] = useState('')

  const { data: model } = useQuery({
    queryKey: ['model', id],
    queryFn: () => api.model(id!),
    enabled: !!id,
  })
  // Every file the carve can touch — the unsorted bucket and every variant's,
  // flat and in path order, which is both what the rules match on and what the
  // annotated list draws.
  const { data: files } = useQuery({
    queryKey: ['model-carve-files', model?.id],
    queryFn: () => api.modelCarveFiles(model!.id),
    enabled: !!model,
  })

  useDocumentTitle(model ? `Carve ${model.name}` : undefined)

  const modelId = model?.id
  // Referentially stable: the panel re-plans whenever this changes.
  const planner = useCallback(
    (spec: LayoutSpec, _target: PlanTarget, countsOnly?: boolean) =>
      api.planModelCarve(modelId!, spec, countsOnly),
    [modelId],
  )
  const handlePlan = useCallback(
    (spec: LayoutSpec | null, plan: LayoutPlan | null) =>
      setLayout(spec && plan ? { spec, plan } : null),
    [],
  )

  // Models the carve would split out: every planned model that captured a name.
  // The unnamed one (if any) is this model's own share and never counts as a split.
  const splits = useMemo(() => (layout?.plan.models ?? []).filter((m) => m.name), [layout])

  const canEdit =
    !!user && (user.role === 'admin' || (user.role === 'editor' && user.id === model?.created_by))

  if (!model) return null
  if (!canEdit) {
    return (
      <Container sx={{ py: 4 }}>
        <Alert severity="error">You can’t carve this model.</Alert>
      </Container>
    )
  }

  const run = async () => {
    if (!layout) return
    setCarving(true)
    setError('')
    try {
      const result = await api.carveModel(model.id, layout.spec, bundleName)
      // The half-built layout was about the model as it stood; the carve has
      // just re-shaped it, so the draft is spent.
      clearImportDraft(model.id)
      await queryClient.invalidateQueries()
      navigate(result.kind === 'bundle' ? `/bundles/${result.slug}` : `/models/${result.slug}`)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
      setCarving(false)
    }
  }

  return (
    <Container maxWidth="lg" sx={{ py: 3 }}>
      <Stack direction="row" spacing={1} sx={{ alignItems: 'center', mb: 2 }}>
        <Button
          component={Link}
          to={`/models/${model.slug}`}
          startIcon={<ArrowBackIcon />}
          size="small"
        >
          {model.name}
        </Button>
      </Stack>
      <Typography variant="h5" sx={{ mb: 0.5 }}>
        Carve “{model.name}”
      </Typography>
      <Typography variant="body2" color="text.secondary" sx={{ mb: 3 }}>
        Match a pattern against the {files?.length ?? 0} file
        {files?.length === 1 ? '' : 's'} this model holds. What the rules recognise as a{' '}
        <strong>variant</strong> is re-sorted into variants here; what they recognise as a{' '}
        <strong>model name</strong> is a piece that was never this model, and splits out into one of
        its own. Anything unmatched stays exactly where it is — unless you ask for the leftovers to
        be swept into the unnamed variant.
      </Typography>

      <Paper variant="outlined" sx={{ p: 2, mb: 2 }}>
        <ImportLayoutPanel
          subjectId={model.id}
          planner={planner}
          fileCount={files?.length ?? 0}
          unpacking={false}
          target="carve"
          onPlan={handlePlan}
        />
      </Paper>

      {splits.length > 0 && (
        <Alert severity="info" sx={{ mb: 2 }}>
          <Typography variant="body2" sx={{ mb: 1.5 }}>
            {splits.length} model{splits.length === 1 ? '' : 's'} will split out of this one. They
            are pieces of the same thing, so a bundle is made to hold them — “{model.name}” included
            — and you’ll land on it afterwards.
          </Typography>
          <TextField
            size="small"
            label="Bundle name"
            value={bundleNameEdited ? bundleName : model.name}
            onChange={(e) => {
              setBundleNameEdited(true)
              setBundleName(e.target.value)
            }}
            sx={{ minWidth: 280 }}
          />
        </Alert>
      )}

      {error && (
        <Alert severity="error" sx={{ mb: 2 }}>
          {error}
        </Alert>
      )}

      <Stack direction="row" spacing={1} sx={{ mb: 3, alignItems: 'center' }}>
        <Button
          variant="contained"
          startIcon={<ContentCutIcon />}
          disabled={!layout || layout.plan.carved === 0 || carving}
          onClick={() => void run()}
        >
          Carve
        </Button>
        <Button component={Link} to={`/models/${model.slug}`} disabled={carving}>
          Cancel
        </Button>
        {layout && layout.plan.carved > 0 && (
          <Typography variant="body2" color="text.secondary">
            {layout.plan.carved} file{layout.plan.carved === 1 ? '' : 's'} will move
            {splits.length > 0
              ? `, ${splits.length} model${splits.length === 1 ? '' : 's'} will split out`
              : ''}
            .
          </Typography>
        )}
      </Stack>

      <Box>
        <Typography variant="subtitle2" sx={{ mb: 1 }}>
          Files
        </Typography>
        <AnnotatedFileList
          files={files ?? []}
          annotations={layout?.plan.annotations ?? []}
          rules={layout?.spec.rules}
        />
      </Box>
    </Container>
  )
}
