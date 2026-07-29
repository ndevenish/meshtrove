import { useCallback, useMemo, useState } from 'react'
import { useParams, Link } from 'react-router-dom'
import { Alert, Box, Button, Container, Paper, Stack, Typography } from '@mui/material'
import ArrowBackIcon from '@mui/icons-material/ArrowBack'
import ContentCutIcon from '@mui/icons-material/ContentCut'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import {
  api,
  type BundleCarveResult,
  type LayoutPlan,
  type LayoutSpec,
  type PlanTarget,
} from '../api'
import ImportLayoutPanel, { AnnotatedFileList } from '../components/ImportLayoutPanel'
import { useDocumentTitle } from '../documentTitle'
import { clearImportDraft } from '../importDraft'
import { useAuth } from '../main'

/// Carving every member of a bundle with one layout.
///
/// A bundle is usually one purchase that arrived as one tree, so its members are
/// named the same way — and a mistake in how that naming was read is a mistake on
/// every one of them. The model carve fixes one member; this runs the same rules
/// over all of them, so a bundle whose folders were really variants (or really
/// separate models) can be put right in one pass instead of thirty.
///
/// Each member is carved exactly as it would be on its own, and what splits out
/// joins *this* bundle rather than making a bundle per member — so the page stays
/// where it is afterwards and simply refetches.
///
/// The preview is the members' plans merged, which is honest about two things:
/// every member's own share reads as one "stays where it is" row, and two members
/// that both capture the name "Head" list twice, because that is two new models.
export default function BundleCarvePage() {
  const { id } = useParams<{ id: string }>()
  const queryClient = useQueryClient()
  const { user } = useAuth()
  const [layout, setLayout] = useState<{ spec: LayoutSpec; plan: LayoutPlan } | null>(null)
  const [carving, setCarving] = useState(false)
  // Set once the carve has run. The page stays put rather than redirecting: a
  // bundle-wide carve lands back on the bundle you were already on, so a silent
  // return would read as "nothing happened" — and the counts are the one thing
  // worth reading afterwards. It also stops a spent layout being applied twice.
  const [done, setDone] = useState<BundleCarveResult | null>(null)
  const [error, setError] = useState('')

  const { data: bundle } = useQuery({
    queryKey: ['bundle', id],
    queryFn: () => api.bundle(id!),
    enabled: !!id,
  })
  // Every file the carve can touch, across every member: flat and in path order,
  // which is both what the rules match on and what the annotated list draws.
  const { data: files } = useQuery({
    queryKey: ['bundle-carve-files', bundle?.id],
    queryFn: () => api.bundleCarveFiles(bundle!.id),
    enabled: !!bundle,
  })

  useDocumentTitle(bundle ? `Carve ${bundle.name}` : undefined)

  const bundleId = bundle?.id
  // Referentially stable: the panel re-plans whenever this changes.
  const planner = useCallback(
    (spec: LayoutSpec, _target: PlanTarget, countsOnly?: boolean) =>
      api.planBundleCarve(bundleId!, spec, countsOnly),
    [bundleId],
  )
  const handlePlan = useCallback(
    (spec: LayoutSpec | null, plan: LayoutPlan | null) =>
      setLayout(spec && plan ? { spec, plan } : null),
    [],
  )

  // New models the carve would make: every planned model that captured a name.
  // The unnamed one is the members' own share and never counts as a split.
  const splits = useMemo(() => (layout?.plan.models ?? []).filter((m) => m.name), [layout])

  const canEdit =
    !!user && (user.role === 'admin' || (user.role === 'editor' && user.id === bundle?.created_by))

  if (!bundle) return null
  if (!canEdit) {
    return (
      <Container sx={{ py: 4 }}>
        <Alert severity="error">You can’t carve this bundle.</Alert>
      </Container>
    )
  }

  const run = async () => {
    if (!layout) return
    setCarving(true)
    setError('')
    try {
      const result = await api.carveBundle(bundle.id, layout.spec)
      // The half-built layout was about the bundle as it stood; the carve has
      // just re-shaped it, so the draft is spent.
      clearImportDraft(bundle.id)
      await queryClient.invalidateQueries()
      setDone(result)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    } finally {
      setCarving(false)
    }
  }

  return (
    <Container maxWidth="lg" sx={{ py: 3 }}>
      <Stack direction="row" spacing={1} sx={{ alignItems: 'center', mb: 2 }}>
        <Button
          component={Link}
          to={`/bundles/${bundle.slug}`}
          startIcon={<ArrowBackIcon />}
          size="small"
        >
          {bundle.name}
        </Button>
      </Stack>
      <Typography variant="h5" sx={{ mb: 0.5 }}>
        Carve the members of “{bundle.name}”
      </Typography>
      <Typography variant="body2" color="text.secondary" sx={{ mb: 3 }}>
        Match a pattern against the {files?.length ?? 0} file
        {files?.length === 1 ? '' : 's'} held by this bundle’s {bundle.models.length} member
        {bundle.models.length === 1 ? '' : 's'}. Every member is then carved with the same rules:
        what they recognise as a <strong>variant</strong> is re-sorted into variants on the member
        it is already on; what they recognise as a <strong>model name</strong> splits out into a new
        model, which joins this bundle. Anything unmatched stays exactly where it is, and a member
        the rules don’t recognise at all is left alone.
      </Typography>

      <Paper variant="outlined" sx={{ p: 2, mb: 2 }}>
        <ImportLayoutPanel
          subjectId={bundle.id}
          planner={planner}
          fileCount={files?.length ?? 0}
          unpacking={false}
          target="carve"
          homeLabel="Stays on its own member"
          onPlan={handlePlan}
        />
      </Paper>

      {splits.length > 0 && (
        <Alert severity="info" sx={{ mb: 2 }}>
          {splits.length} new model{splits.length === 1 ? '' : 's'} will split out of the members
          and join this bundle. Two members that capture the same name make two models — one each —
          so a name may appear twice in the list above.
        </Alert>
      )}

      {error && (
        <Alert severity="error" sx={{ mb: 2 }}>
          {error}
        </Alert>
      )}

      {done && (
        <Alert
          severity="success"
          sx={{ mb: 2 }}
          action={
            <Button component={Link} to={`/bundles/${bundle.slug}`} size="small">
              Back to bundle
            </Button>
          }
        >
          Carved {done.members_carved} of {bundle.models.length} member
          {bundle.models.length === 1 ? '' : 's'}
          {done.models_created > 0 &&
            `, ${done.models_created} new model${done.models_created === 1 ? '' : 's'} joined this bundle`}
          {done.variants_removed > 0 &&
            `, ${done.variants_removed} emptied variant${done.variants_removed === 1 ? '' : 's'} removed`}
          . The plan below has been re-run against the result.
        </Alert>
      )}

      <Stack direction="row" spacing={1} sx={{ mb: 3, alignItems: 'center' }}>
        <Button
          variant="contained"
          startIcon={<ContentCutIcon />}
          disabled={!layout || layout.plan.carved === 0 || carving || !!done}
          onClick={() => void run()}
        >
          Carve members
        </Button>
        <Button component={Link} to={`/bundles/${bundle.slug}`} disabled={carving}>
          {done ? 'Done' : 'Cancel'}
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
