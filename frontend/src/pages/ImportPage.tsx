import { useCallback, useEffect, useState } from 'react'
import { useNavigate, useParams, useSearchParams } from 'react-router-dom'
import {
  Alert,
  Autocomplete,
  Box,
  Button,
  CircularProgress,
  Container,
  LinearProgress,
  Paper,
  Snackbar,
  Stack,
  TextField,
  ToggleButton,
  ToggleButtonGroup,
  Typography,
} from '@mui/material'
import DeleteIcon from '@mui/icons-material/Delete'
import ViewInArIcon from '@mui/icons-material/ViewInAr'
import Inventory2Icon from '@mui/icons-material/Inventory2'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import {
  api,
  type BundleSummary,
  type CommitTarget,
  type FileRecord,
  type LayoutPlan,
  type LayoutSpec,
} from '../api'
import { FileTree } from '../components/VariantSection'
import ImportLayoutPanel, { AnnotatedFileList } from '../components/ImportLayoutPanel'
import ImportRestorePanel from '../components/ImportRestorePanel'
import { useImportDraftState, clearImportDraft } from '../importDraft'
import { changeTags, pasteTags } from '../tags'

type Destination = 'new_model' | 'new_bundle' | 'bundle'

// A stable stand-in while the files query is loading: `files ?? []` would mint
// a fresh array every render and defeat the file list's memo.
const NO_FILES: FileRecord[] = []

/// The one place the model-vs-bundle question gets asked — after the archive has
/// unpacked and you can see what's actually in it. Committing moves every staged
/// file onto the destination and the import disappears.
///
/// `/imports/:id` reuses one instance across ids, so key the workbench on the id:
/// a fresh import gets fresh state (and clean seeding), and its persisted draft is
/// read once on mount rather than leaking between imports.
export default function ImportPage() {
  const { id } = useParams<{ id: string }>()
  return <ImportWorkbench key={id ?? 'new'} />
}

function ImportWorkbench() {
  const { id } = useParams<{ id: string }>()
  const [params] = useSearchParams()
  const navigate = useNavigate()
  const queryClient = useQueryClient()
  // These fields are the import's draft: persisted per id (see importDraft.ts) so
  // closing the page and reopening it restores what you were part-way through.
  const [name, setName] = useImportDraftState(id!, 'name', '')
  const [nameEdited, setNameEdited] = useImportDraftState(id!, 'nameEdited', false)
  const [dest, setDest] = useImportDraftState<Destination>(
    id!,
    'dest',
    params.get('bundle') ? 'bundle' : 'new_model',
  )
  // The chosen bundle is stored by id (a BundleSummary is not ours to persist) and
  // re-resolved from the loaded list below.
  const [targetId, setTargetId] = useImportDraftState<string | null>(id!, 'targetId', null)
  const [layout, setLayout] = useState<{ spec: LayoutSpec; plan: LayoutPlan } | null>(null)
  // Per member-model merge choices from the layout panel, index-aligned to the
  // plan's models (a member id or null=new); null when not merging into a bundle.
  const [mergeTargets, setMergeTargets] = useState<(string | null)[] | null>(null)
  const [committing, setCommitting] = useState(false)
  const [error, setError] = useState('')
  // Set after a partial ("keep unmatched files") commit, which stays on this
  // page: the import survived with the remainder, ready for another pass.
  const [notice, setNotice] = useState('')

  // The facts about the drop, typed once. A box set is bought once, from one
  // creator, under one licence — so on a bundle commit these land on every member
  // model the carve creates, not just the bundle.
  const [creatorName, setCreatorName] = useImportDraftState(id!, 'creatorName', '')
  const [tags, setTags] = useImportDraftState<string[]>(id!, 'tags', [])
  const [sourceUrl, setSourceUrl] = useImportDraftState(id!, 'sourceUrl', '')
  const [description, setDescription] = useImportDraftState(id!, 'description', '')

  const { data: staged, isLoading } = useQuery({
    queryKey: ['import', id],
    queryFn: () => api.import(id!),
    enabled: !!id,
    // While the archive is unpacking, the file list is still growing.
    refetchInterval: (query) => (query.state.data?.unpacking ? 1500 : false),
  })
  // Poll the *same* query while the archive unpacks, so arriving files are added
  // to a list that stays mounted. (Keying it on `file_count` made every tick a
  // different query with an empty cache: `files` blanked to undefined and
  // everything drawn from it tore down and rebuilt — a page-wide flicker.)
  //
  // Stop on what we're holding, not on the flag: the last files land *between*
  // the final poll and `unpacking` going false, so a poll that stops when the
  // flag clears stops one fetch too early and leaves the tail of the archive
  // off the page until a reload. Keep going until the list matches the count the
  // import reports.
  const { data: files } = useQuery({
    queryKey: ['import-files', id],
    queryFn: () => api.importFiles(id!),
    enabled: !!id,
    refetchInterval: (query) => {
      if (!staged) return false
      const held = query.state.data?.length ?? 0
      return staged.unpacking || held !== staged.file_count ? 1500 : false
    },
  })
  const { data: creators } = useQuery({ queryKey: ['creators'], queryFn: () => api.creators() })
  const { data: allTags } = useQuery({ queryKey: ['tags'], queryFn: () => api.tags() })
  const { data: bundles } = useQuery({
    queryKey: ['bundles-all'],
    queryFn: () => api.searchBundles(new URLSearchParams({ per_page: '100' })),
  })
  const target: BundleSummary | null = bundles?.bundles.find((b) => b.id === targetId) ?? null
  const fileList = files ?? NO_FILES

  // Referentially stable so the memoised layout panel isn't re-rendered by
  // every keystroke in the form around it.
  const handlePlan = useCallback(
    (spec: LayoutSpec | null, plan: LayoutPlan | null) =>
      setLayout(spec && plan ? { spec, plan } : null),
    [],
  )

  // Seed the editable name once the import loads, and preselect the bundle when
  // the drop happened on a bundle page (?bundle=…).
  useEffect(() => {
    if (staged && !name) setName(staged.name)
  }, [staged, name, setName])
  useEffect(() => {
    const wanted = params.get('bundle')
    if (wanted && !targetId && bundles) {
      // `?bundle=` may carry the id (the bundle page's inline merge box) or the
      // slug (the global page drop, whose URL is the canonical slug) — take either.
      setTargetId(bundles.bundles.find((b) => b.id === wanted || b.slug === wanted)?.id ?? null)
    }
  }, [params, bundles, targetId, setTargetId])

  // A one-model layout that captured a single model name suggests it — unless
  // the user has typed their own.
  const suggestedName = dest === 'new_model' ? layout?.plan.models[0]?.name : undefined
  useEffect(() => {
    if (suggestedName && !nameEdited) setName(suggestedName)
  }, [suggestedName, nameEdited, setName])

  if (isLoading || !staged) return null

  const commit = async () => {
    if (!name.trim()) return setError('Give it a name first')
    if (dest === 'bundle' && !target) return setError('Pick a bundle to add to')
    setCommitting(true)
    try {
      // A creator typed by hand is a creator that may not exist yet — same
      // get-or-create the model dialog does, so the two agree on one row.
      const typed = creatorName.trim()
      let creator_id: string | null = null
      if (typed) {
        const existing = (creators ?? []).find((c) => c.name.toLowerCase() === typed.toLowerCase())
        creator_id = existing ? existing.id : (await api.createCreator({ name: typed })).id
      }
      // Blank means "nothing to say": the backend coalesces, so an empty field
      // never erases what the carve worked out for itself.
      const meta = {
        creator_id,
        tags,
        source_url: sourceUrl.trim() || null,
        description_md: description.trim() || null,
      }
      const spec = layout?.spec
      const body: CommitTarget =
        dest === 'bundle'
          ? {
              target: 'bundle',
              bundle_id: target!.id,
              layout: spec,
              merge_targets: spec ? (mergeTargets ?? undefined) : undefined,
              ...meta,
            }
          : dest === 'new_bundle'
            ? {
                target: 'new_bundle',
                name: name.trim(),
                kind: 'purchased',
                name_autogenerated: !nameEdited,
                layout: spec,
                ...meta,
              }
            : { target: 'new_model', name: name.trim(), layout: spec, ...meta }
      const result = await api.commitImport(staged.id, body)
      // A "keep unmatched files" commit leaves the import alive with the
      // remainder, so stay here for the next pass — the draft (rules, choices)
      // stays too. The re-plan happens by itself as the file list refetches.
      if (spec?.keep_unmatched) {
        await queryClient.invalidateQueries()
        setCommitting(false)
        setNotice('Matched files imported — the rest are still staged here.')
        return
      }
      clearImportDraft(staged.id)
      await queryClient.invalidateQueries()
      navigate(result.type === 'model' ? `/models/${result.slug}` : `/bundles/${result.slug}`)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
      setCommitting(false)
    }
  }

  const discard = async () => {
    await api.deleteImport(staged.id)
    clearImportDraft(staged.id)
    await queryClient.invalidateQueries({ queryKey: ['imports'] })
    navigate('/imports')
  }

  // A dropped MeshTrove export is restored, not carved: skip the whole
  // model/bundle/layout question and show what the archive holds.
  if (staged.is_export) {
    return (
      <Container maxWidth="md" sx={{ py: 3 }}>
        <Stack direction="row" spacing={1} sx={{ alignItems: 'center', mb: 2 }}>
          <Box sx={{ flexGrow: 1 }}>
            <Typography variant="h5">Restore export</Typography>
            <Typography color="text.secondary" variant="body2">
              A MeshTrove export archive — restore the models and bundles it holds.
            </Typography>
          </Box>
          <Button color="error" startIcon={<DeleteIcon />} onClick={discard}>
            Discard
          </Button>
        </Stack>
        <Paper variant="outlined" sx={{ p: 2 }}>
          <ImportRestorePanel
            importId={staged.id}
            onImported={async () => {
              await queryClient.invalidateQueries()
              navigate('/')
            }}
          />
        </Paper>
        <Snackbar
          open={!!error}
          autoHideDuration={8000}
          onClose={() => setError('')}
          anchorOrigin={{ vertical: 'bottom', horizontal: 'center' }}
        >
          <Alert severity="error" onClose={() => setError('')}>
            {error}
          </Alert>
        </Snackbar>
      </Container>
    )
  }

  return (
    <Container maxWidth="xl" sx={{ py: 3 }}>
      <Stack direction="row" spacing={1} sx={{ alignItems: 'center', mb: 2 }}>
        <Box sx={{ flexGrow: 1 }}>
          <Typography variant="h5">Importing</Typography>
          <Typography color="text.secondary" variant="body2">
            {staged.file_count} file{staged.file_count === 1 ? '' : 's'} staged
            {staged.partial
              ? ' — the rest of this drop has already been placed; carve what remains below.'
              : ' — not in your library until you place it below.'}
          </Typography>
        </Box>
        <Button color="error" startIcon={<DeleteIcon />} onClick={discard} disabled={committing}>
          Discard
        </Button>
      </Stack>

      {/* Two columns: the data-entry form (left) beside the file list (right), so
          a layout pattern's captures preview next to the fields that set them.
          Stacks to one column below md; on md+ the file list sticks in view while
          the long form scrolls past it. */}
      <Box
        sx={{
          display: 'flex',
          flexDirection: { xs: 'column', md: 'row' },
          alignItems: 'flex-start',
          gap: 3,
        }}
      >
        <Box sx={{ width: '100%', flex: '1 1 0', minWidth: 0 }}>
          {staged.unpacking && (
            <Paper variant="outlined" sx={{ p: 2, mb: 2 }}>
              <Typography sx={{ mb: 1 }}>Unpacking the archive…</Typography>
              <LinearProgress />
            </Paper>
          )}

          <Paper variant="outlined" sx={{ p: 2, mb: 2 }}>
            <TextField
              fullWidth
              label="Name"
              value={name}
              onChange={(e) => {
                setName(e.target.value)
                setNameEdited(true)
              }}
              helperText="Used for the model or bundle this becomes"
              sx={{ mb: 2 }}
            />

            <Typography variant="subtitle2" sx={{ mb: 1 }}>
              What is this?
            </Typography>
            <ToggleButtonGroup
              exclusive
              value={dest}
              onChange={(_, value) => value && setDest(value as Destination)}
              sx={{ mb: 2, flexWrap: 'wrap' }}
            >
              <ToggleButton value="new_model" sx={{ gap: 1 }}>
                <ViewInArIcon fontSize="small" /> One model
              </ToggleButton>
              <ToggleButton value="new_bundle" sx={{ gap: 1 }}>
                <Inventory2Icon fontSize="small" /> A new bundle
              </ToggleButton>
              <ToggleButton value="bundle" sx={{ gap: 1 }}>
                <Inventory2Icon fontSize="small" /> Add to an existing bundle
              </ToggleButton>
            </ToggleButtonGroup>

            {dest === 'bundle' && (
              <Autocomplete
                options={bundles?.bundles ?? []}
                getOptionLabel={(b) => b.name}
                value={target}
                onChange={(_, value) => setTargetId(value?.id ?? null)}
                renderInput={(props) => <TextField {...props} label="Bundle" />}
                sx={{ mb: 2 }}
              />
            )}

            <Typography variant="subtitle2" sx={{ mb: 1 }}>
              Details (optional)
            </Typography>
            <Typography variant="body2" color="text.secondary" sx={{ mb: 1.5 }}>
              {dest === 'new_model'
                ? 'Recorded on the model this becomes.'
                : 'Recorded on the bundle and on every model carved out of it — a box set is bought once.'}
            </Typography>
            <Stack spacing={2} sx={{ mb: 2.5 }}>
              <Autocomplete
                freeSolo
                options={(creators ?? []).map((c) => c.name)}
                // The typed text *is* the state here (commit trims and get-or-creates
                // from it), so control inputValue, not value: driving the selection
                // from keystrokes makes the widget reconcile a changed selection
                // against its options on every character.
                inputValue={creatorName}
                onInputChange={(_, value) => setCreatorName(value)}
                renderInput={(props) => (
                  <TextField {...props} size="small" label="Creator (author / company / site)" />
                )}
              />
              <Autocomplete
                multiple
                freeSolo
                options={(allTags ?? []).map((t) => t.name)}
                value={tags}
                onChange={changeTags(setTags)}
                renderInput={(props) => (
                  <TextField
                    {...props}
                    size="small"
                    label="Tags"
                    placeholder="add tag…"
                    onPaste={pasteTags(tags, setTags)}
                  />
                )}
              />
              <TextField
                size="small"
                label="Source URL"
                value={sourceUrl}
                onChange={(e) => setSourceUrl(e.target.value)}
              />
              <TextField
                size="small"
                label="Description (markdown)"
                value={description}
                onChange={(e) => setDescription(e.target.value)}
                multiline
                minRows={3}
              />
            </Stack>

            <Typography variant="subtitle2" sx={{ mb: 1 }}>
              Carve it up? (optional)
            </Typography>
            <ImportLayoutPanel
              importId={staged.id}
              fileCount={fileList.filter((f) => f.kind !== 'archive').length}
              unpacking={staged.unpacking}
              target={dest === 'new_model' ? 'model' : 'bundle'}
              bundleId={dest === 'bundle' ? target?.id : undefined}
              onPlan={handlePlan}
              onMergeTargets={setMergeTargets}
            />

            {dest === 'new_model' && layout && layout.plan.model_names.length > 1 && (
              <Alert
                severity="info"
                sx={{ mb: 2 }}
                action={
                  <Button size="small" onClick={() => setDest('new_bundle')}>
                    Make it a bundle
                  </Button>
                }
              >
                This layout finds {layout.plan.model_names.length} different model names — it looks
                like a collection.
              </Alert>
            )}

            <Typography variant="body2" color="text.secondary" sx={{ mb: 2 }}>
              {layout
                ? dest === 'new_model'
                  ? `Matched files are carved into ${layout.plan.models[0]?.variants.length ?? 0} variant(s); the rest ${layout.spec.keep_unmatched ? 'stay staged here for another pass' : 'land in the model’s unsorted list'}.`
                  : `Matched files are carved into ${layout.plan.models.length} member model(s) with their variants; the rest ${layout.spec.keep_unmatched ? 'stay staged here for another pass' : 'land in the bundle’s unsorted list'}.`
                : dest === 'new_model'
                  ? 'Files land in the model’s unsorted list; sort them into variants on the model page.'
                  : 'Files land in the bundle’s unsorted list; carve them into member models on the bundle page.'}
            </Typography>

            <Stack direction="row" spacing={1}>
              <Button
                variant="contained"
                size="large"
                onClick={commit}
                disabled={staged.unpacking || committing || staged.file_count === 0}
                startIcon={committing ? <CircularProgress size={16} color="inherit" /> : undefined}
              >
                {staged.unpacking ? 'Waiting for unpack…' : 'Import'}
              </Button>
            </Stack>
          </Paper>
        </Box>

        <Box
          sx={{
            width: '100%',
            flex: '1 1 0',
            minWidth: 0,
            position: { md: 'sticky' },
            top: { md: 88 },
            maxHeight: { md: 'calc(100vh - 104px)' },
            overflowY: { md: 'auto' },
          }}
        >
          <Typography variant="h6" sx={{ mb: 1 }}>
            Contents
          </Typography>
          {layout ? (
            <AnnotatedFileList
              files={fileList}
              annotations={layout.plan.annotations}
              rules={layout.spec.rules}
            />
          ) : (
            <FileTree files={fileList} />
          )}
        </Box>
      </Box>

      <Snackbar
        open={!!error}
        autoHideDuration={8000}
        onClose={() => setError('')}
        anchorOrigin={{ vertical: 'bottom', horizontal: 'center' }}
      >
        <Alert severity="error" onClose={() => setError('')}>
          {error}
        </Alert>
      </Snackbar>
      <Snackbar
        open={!!notice}
        autoHideDuration={8000}
        onClose={() => setNotice('')}
        anchorOrigin={{ vertical: 'bottom', horizontal: 'center' }}
      >
        <Alert severity="success" onClose={() => setNotice('')}>
          {notice}
        </Alert>
      </Snackbar>
    </Container>
  )
}
