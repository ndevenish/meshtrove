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
  type ImportFolder,
  type LayoutPlan,
  type LayoutSpec,
  type ImportSummary,
} from '../api'
import { FileTree } from '../components/VariantSection'
import ImportLayoutPanel, { AnnotatedFileList } from '../components/ImportLayoutPanel'
import ImportRestorePanel from '../components/ImportRestorePanel'
import ImportStagingProgress from '../components/ImportStagingProgress'
import { useImportDraftState, clearImportDraft } from '../importDraft'
import { changeTags, pasteTags } from '../tags'
import { CustomFieldControl, type ScalarValue } from '../components/CustomFieldControl'

type Destination = 'new_model' | 'new_bundle' | 'bundle'

// A stable stand-in while the files query is loading: `files ?? []` would mint
// a fresh array every render and defeat the file list's memo.
const NO_FILES: FileRecord[] = []
const NO_FOLDERS: ImportFolder[] = []

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
  // The import a folder was just split out into — announced with a link to it,
  // since carving on here is the more likely next move.
  const [splitOff, setSplitOff] = useState<ImportSummary | null>(null)

  // The facts about the drop, typed once. A box set is bought once, from one
  // creator, under one licence — so on a bundle commit these land on every member
  // model the carve creates, not just the bundle.
  const [creatorName, setCreatorName] = useImportDraftState(id!, 'creatorName', '')
  const [tags, setTags] = useImportDraftState<string[]>(id!, 'tags', [])
  const [sourceUrl, setSourceUrl] = useImportDraftState(id!, 'sourceUrl', '')
  const [description, setDescription] = useImportDraftState(id!, 'description', '')
  // Custom fields, by field id. Each value goes wherever its own definition says
  // — see the backend's apply_custom_fields — so one form serves a drop that
  // creates a bundle and its members at once.
  const [customValues, setCustomValues] = useImportDraftState<Record<string, ScalarValue>>(
    id!,
    'customFields',
    {},
  )

  const { data: staged, isLoading } = useQuery({
    queryKey: ['import', id],
    queryFn: () => api.import(id!),
    enabled: !!id,
    // While the archive is unpacking, the file list is still growing.
    refetchInterval: (query) => (query.state.data?.unpacking ? 1500 : false),
  })
  // While the import is still filling up, watch it by folder counts instead of
  // by file. The full listing costs the server time proportional to the number
  // of staged files and hands the browser every row to rebuild a tree from,
  // which a multi-hour dropbox pickup of tens of thousands of files cannot
  // afford to be asked for on a timer. Nothing is lost: committing is refused
  // until the unpack clears, so there is nothing to do with an individual
  // staged file yet.
  const { data: folders } = useQuery({
    queryKey: ['import-folders', id],
    queryFn: () => api.importFileSummary(id!),
    enabled: !!id && !!staged?.unpacking,
    refetchInterval: 1500,
  })
  // The real listing, once there is a settled import to list. Held in one
  // mounted query rather than keyed on `file_count`: keying it that way made
  // every tick a different query with an empty cache, so `files` blanked to
  // undefined and everything drawn from it tore down and rebuilt — a page-wide
  // flicker.
  //
  // It still polls, and stops on what we're holding rather than on the flag:
  // the last files land *between* the final poll and `unpacking` going false,
  // so stopping when the flag clears stops one fetch too early and leaves the
  // tail of the archive off the page until a reload. With the fetch gated on
  // the flag this converges promptly — no new files are arriving by then.
  const { data: files, isLoading: filesLoading } = useQuery({
    queryKey: ['import-files', id],
    queryFn: () => api.importFiles(id!),
    enabled: !!id && !staged?.unpacking,
    refetchInterval: (query) => {
      if (!staged) return false
      const held = query.state.data?.length ?? 0
      return held === staged.file_count ? false : 1500
    },
  })
  const { data: creators } = useQuery({ queryKey: ['creators'], queryFn: () => api.creators() })
  const { data: allTags } = useQuery({ queryKey: ['tags'], queryFn: () => api.tags() })
  // Every field either side of the commit could want, with whatever this import
  // is already holding. A scalar waits in the draft above and is written when
  // the commit produces an owner; a file-kind one can't wait — its payload is
  // bytes — so it is uploaded onto the *import* and copied onto whatever the
  // commit carves out. Hence the values live here, not in the draft.
  const { data: stagedFields } = useQuery({
    queryKey: ['import-custom-fields', id],
    queryFn: () => api.importCustomFields(id!),
    enabled: !!id,
  })
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

  // Which fields this drop can carry. A model target takes the models-only ones;
  // a bundle target takes both, because a models-only field still reaches every
  // member the carve creates.
  const applicableFields = (stagedFields ?? []).filter(({ field: f }) =>
    dest === 'new_model' ? f.applies_to_models : f.applies_to_bundles || f.applies_to_models,
  )
  // Only the scalars ride along with the commit; a file was already stored on
  // the import when it was dropped.
  const scalarFields = applicableFields.filter((e) => e.field.kind !== 'file')

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
        custom_fields: scalarFields.map((e) => ({
          field_id: e.field.id,
          value: customValues[e.field.id] ?? null,
        })),
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

  // A file-kind field is stored the moment it is dropped, on the import itself:
  // there is no owner to hang it off until the commit runs, and the bytes can't
  // sit in a draft. It rides through to whatever the commit creates — and
  // through a split, into the import a folder is lifted into.
  const refreshFields = () =>
    queryClient.invalidateQueries({ queryKey: ['import-custom-fields', id] })
  const uploadFieldFile = (fieldId: string) => async (file: File) => {
    const form = new FormData()
    form.append('file', file)
    try {
      await api.uploadCustomFieldFile('imports', staged.id, fieldId, form)
      await refreshFields()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    }
  }
  const clearFieldFile = (fieldId: string) => async () => {
    try {
      await api.clearCustomField('imports', staged.id, fieldId)
      await refreshFields()
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    }
  }

  const discard = async () => {
    await api.deleteImport(staged.id)
    clearImportDraft(staged.id)
    await queryClient.invalidateQueries({ queryKey: ['imports'] })
    navigate('/imports')
  }

  // Drop a folder's staged files without importing them. There's no folder row
  // to delete — a folder is just its files' shared `path` — so delete each file
  // and refresh the list (and the summary's file_count) around them.
  //
  // A few at a time, not all at once: discarding a folder with its subfolders is
  // routinely thousands of files, and firing that many requests in one breath
  // buries the server under its own import.
  const discardFolder = async (fileIds: string[]) => {
    try {
      for (let i = 0; i < fileIds.length; i += 8) {
        await Promise.all(fileIds.slice(i, i + 8).map((fid) => api.deleteFile(fid)))
      }
      await queryClient.invalidateQueries({ queryKey: ['import', id] })
      await queryClient.invalidateQueries({ queryKey: ['import-files', id] })
      await queryClient.invalidateQueries({ queryKey: ['imports'] })
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    }
  }

  // Lift a folder out into an import of its own. One drop is often several
  // things — a dropbox pickup of a creator's back catalogue is a folder per
  // product — and an import commits to exactly one destination.
  //
  // Staying put is the point: the drop being carved up is right here, and
  // splitting the next folder out of it is the next thing you do. The new import
  // is one link away on the toast.
  const splitFolder = async (dir: string, name: string) => {
    try {
      const created = await api.splitImport(staged.id, dir, name)
      await queryClient.invalidateQueries({ queryKey: ['import', id] })
      await queryClient.invalidateQueries({ queryKey: ['import-files', id] })
      await queryClient.invalidateQueries({ queryKey: ['imports'] })
      setSplitOff(created)
    } catch (err) {
      setError(err instanceof Error ? err.message : String(err))
    }
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
              {applicableFields.map((entry) => (
                <CustomFieldControl
                  key={entry.field.id}
                  entry={entry}
                  value={customValues[entry.field.id] ?? null}
                  onChange={(value) =>
                    setCustomValues({ ...customValues, [entry.field.id]: value })
                  }
                  onUploadFile={uploadFieldFile(entry.field.id)}
                  onClearFile={clearFieldFile(entry.field.id)}
                />
              ))}
              <TextField
                size="small"
                // Name the destination: under a bundle target this describes the
                // bundle and is *not* copied onto each member model, which the
                // bare word "Description" left ambiguous.
                label={
                  dest === 'new_model'
                    ? 'Model description (markdown)'
                    : 'Bundle description (markdown)'
                }
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
          {staged.unpacking ? (
            <ImportStagingProgress folders={folders ?? NO_FOLDERS} />
          ) : filesLoading ? (
            // Until the first fetch lands we hold an empty list, which the file
            // tree would report as "No files yet" — a verdict on an import we
            // haven't read yet. Say we're still reading instead.
            <Stack direction="row" spacing={1} sx={{ alignItems: 'center' }}>
              <CircularProgress size={16} />
              <Typography variant="body2" color="text.secondary">
                Loading files…
              </Typography>
            </Stack>
          ) : layout ? (
            <AnnotatedFileList
              files={fileList}
              annotations={layout.plan.annotations}
              rules={layout.spec.rules}
            />
          ) : (
            <FileTree
              files={fileList}
              archivesExtracted
              onFolderDiscard={discardFolder}
              onFolderSplit={splitFolder}
            />
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
      <Snackbar
        open={!!splitOff}
        autoHideDuration={12000}
        onClose={() => setSplitOff(null)}
        anchorOrigin={{ vertical: 'bottom', horizontal: 'center' }}
      >
        <Alert
          severity="success"
          onClose={() => setSplitOff(null)}
          action={
            <Button
              color="inherit"
              size="small"
              onClick={() => {
                const target = splitOff
                setSplitOff(null)
                if (target) navigate(`/imports/${target.id}`)
              }}
            >
              Open it
            </Button>
          }
        >
          {splitOff?.file_count} file{splitOff?.file_count === 1 ? '' : 's'} split into “
          {splitOff?.name}”
        </Alert>
      </Snackbar>
    </Container>
  )
}
