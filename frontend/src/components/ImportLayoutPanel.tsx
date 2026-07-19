import { Fragment, useEffect, useMemo, useRef, useState } from 'react'
import {
  Autocomplete,
  Box,
  Button,
  Checkbox,
  Chip,
  FormControl,
  IconButton,
  InputLabel,
  MenuItem,
  Radio,
  RadioGroup,
  FormControlLabel,
  Select,
  Stack,
  Switch,
  TextField,
  Tooltip,
  Typography,
} from '@mui/material'
import AddIcon from '@mui/icons-material/Add'
import DeleteOutlineIcon from '@mui/icons-material/DeleteOutlined'
import SaveIcon from '@mui/icons-material/Save'
import ViewInArIcon from '@mui/icons-material/ViewInAr'
import WarningAmberIcon from '@mui/icons-material/WarningAmber'
import InsertDriveFileIcon from '@mui/icons-material/InsertDriveFile'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import {
  api,
  type FileAnnotation,
  type FileRecord,
  type GroupRole,
  type ImportLayout,
  type LayoutPlan,
  type LayoutRule,
  type LayoutSpec,
  type PlanModel,
  type PlanTarget,
} from '../api'
import { useImportDraftState } from '../importDraft'

/// A planned model's stable identity across re-plans (name + its tag set),
/// so a retarget choice survives an edit to the pattern that reorders the list.
const modelKey = (m: PlanModel): string =>
  `${m.name.toLowerCase()} ${[...m.tags]
    .map((t) => t.toLowerCase())
    .sort()
    .join(',')}`

/// Highlight colours per role — alpha overlays so they read on light and dark.
export const ROLE_STYLES: Record<GroupRole, { label: string; bg: string }> = {
  model_name: { label: 'Model name', bg: 'rgba(250, 104, 49, 0.30)' },
  creator_ref: { label: 'Creator ID', bg: 'rgba(2, 136, 209, 0.28)' },
  model_tag: { label: 'Model tag', bg: 'rgba(142, 36, 170, 0.25)' },
  variant_tag: { label: 'Variant tag', bg: 'rgba(46, 125, 50, 0.28)' },
  ignore: { label: 'Ignore', bg: 'transparent' },
}
const ROLE_ORDER: GroupRole[] = ['model_name', 'creator_ref', 'model_tag', 'variant_tag', 'ignore']

/// What a rule is called in a message when the user hasn't named it.
const ruleLabel = (rule: LayoutRule, index: number): string =>
  rule.name.trim() || `Rule ${index + 1}`

const blankRule = (): LayoutRule => ({
  name: '',
  pattern: '',
  roles: {},
  value_map: {},
  enabled: true,
})

/// The layout carve panel: compose the carve out of several small patterns,
/// assign each one's capture groups a role, map raw captured values onto variant
/// tags, and preview the merged result. All matching happens server-side
/// (`POST /api/imports/{id}/plan`) — patterns are opaque strings here, and the
/// commit runs the same analysis, so what this previews is exactly what commits.
export default function ImportLayoutPanel({
  importId,
  fileCount,
  unpacking,
  target,
  bundleId,
  onPlan,
  onMergeTargets,
}: {
  importId: string
  /** re-plan when this grows */
  fileCount: number
  /** the archive is still extracting: plan against the finished tree, not a
      half-unpacked one that changes under us every second */
  unpacking: boolean
  target: PlanTarget
  /** merging into this existing bundle: the plan then reports which member each
      planned model would land on, and the panel offers a retarget dropdown */
  bundleId?: string
  onPlan: (spec: LayoutSpec | null, plan: LayoutPlan | null) => void
  /** the resolved per-model merge choices, index-aligned to the plan's models
      (a member id or null=new); null when there's no existing bundle to merge
      into */
  onMergeTargets?: (targets: (string | null)[] | null) => void
}) {
  const queryClient = useQueryClient()
  // The carve is part of the import's draft: the rule list, flatten and merge
  // choices persist per import id (see importDraft.ts), so reopening the import
  // brings the half-built layout back with it — including which rules you had
  // switched off, which is per-import working state, not a property of the saved
  // template. The plan itself is not stored — it is re-derived server-side from
  // the restored spec on mount.
  const [selected, setSelected] = useImportDraftState(importId, 'layout.selected', '')
  const [rules, setRules] = useImportDraftState<LayoutRule[]>(importId, 'layout.rules', [])
  const [plan, setPlan] = useState<LayoutPlan | null>(null)
  const [planError, setPlanError] = useState('')
  const [flatten, setFlatten] = useImportDraftState(importId, 'layout.flatten', false)
  // Commit only the matched files and keep the rest staged here. Per-import
  // working state (like the enabled toggles' current values), never part of a
  // saved template — whether a drop is being split across targets is a fact
  // about this drop, not about the publisher's tree.
  const [keepUnmatched, setKeepUnmatched] = useImportDraftState(
    importId,
    'layout.keepUnmatched',
    false,
  )
  // Retarget choices, keyed by planned-model identity so they survive re-plans:
  // a member id, or 'new' to force a fresh member. An unset model rides on the
  // plan's auto-matched `merge_target`.
  const [mergeChoices, setMergeChoices] = useImportDraftState<Record<string, string>>(
    importId,
    'layout.mergeChoices',
    {},
  )
  const [saveName, setSaveName] = useState('')
  const [saveError, setSaveError] = useState('')

  const { data: layouts } = useQuery({
    queryKey: ['import-layouts'],
    queryFn: api.importLayouts,
  })
  const { data: vocab } = useQuery({
    queryKey: ['variant-tags'],
    queryFn: () => api.variantTags(),
  })

  // Coverage ranking for the picker: dry-run every saved layout once the file
  // tree stops moving. A layout that errors (or matches nothing) still lists —
  // it just can't recommend itself. Waiting for the unpack costs a few seconds
  // of an empty column and saves a full dry-run of every layout, every second,
  // against numbers that are stale before they render.
  const { data: coverage } = useQuery({
    queryKey: ['layout-coverage', importId, fileCount, layouts?.map((l) => l.id).join()],
    enabled: !!layouts && fileCount > 0 && !unpacking,
    queryFn: async () => {
      const entries = await Promise.all(
        layouts!.map(async (layout) => {
          try {
            const p = await api.planImport(importId, layout, 'bundle')
            return [layout.id, p.matched] as const
          } catch {
            return [layout.id, -1] as const
          }
        }),
      )
      return Object.fromEntries(entries) as Record<string, number>
    },
  })

  const applyLayout = (layout: ImportLayout) => {
    setRules(layout.rules.map((rule) => ({ ...rule })))
    setFlatten(layout.flatten ?? false)
  }

  const clear = () => {
    setRules([])
    setFlatten(false)
    setKeepUnmatched(false)
    setPlan(null)
    setPlanError('')
  }

  /// Patch one rule in place; every editor field goes through this.
  const updateRule = (index: number, patch: Partial<LayoutRule>) =>
    setRules((prev) => prev.map((rule, i) => (i === index ? { ...rule, ...patch } : rule)))

  // A rule with no pattern yet is inert server-side (it is not "matches
  // everything"), so an empty new block never floods the preview.
  const live = rules.filter((rule) => rule.pattern.trim())

  // Debounced server-side dry run; also the sole source of the parent's spec.
  const onPlanRef = useRef(onPlan)
  onPlanRef.current = onPlan
  useEffect(() => {
    if (live.length === 0) {
      setPlan(null)
      setPlanError('')
      onPlanRef.current(null, null)
      return
    }
    // Nothing to plan against yet: the tree is still arriving, and each arrival
    // would otherwise fire another dry run whose answer is obsolete on landing.
    if (unpacking) return
    // Every rule is sent, blanks included, so `plan.rules` stays index-aligned
    // with the editor blocks on screen.
    const spec: LayoutSpec = { rules, flatten, keep_unmatched: keepUnmatched }
    const timer = setTimeout(async () => {
      try {
        const result = await api.planImport(importId, spec, target, bundleId)
        setPlan(result)
        setPlanError('')
        onPlanRef.current(spec, result)
      } catch (err) {
        setPlan(null)
        setPlanError(err instanceof Error ? err.message : String(err))
        onPlanRef.current(null, null)
      }
    }, 400)
    return () => clearTimeout(timer)
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [importId, rules, target, fileCount, unpacking, flatten, keepUnmatched, bundleId, live.length])

  // The member each planned model resolves to: an explicit dropdown choice, else
  // the plan's auto-match. Reported up so the commit sends the same array.
  const choiceFor = (model: PlanModel): string =>
    mergeChoices[modelKey(model)] ?? model.merge_target ?? 'new'

  const onMergeTargetsRef = useRef(onMergeTargets)
  onMergeTargetsRef.current = onMergeTargets
  useEffect(() => {
    const report = onMergeTargetsRef.current
    if (!report) return
    if (!plan || !bundleId) return report(null)
    report(plan.models.map((m) => (choiceFor(m) === 'new' ? null : choiceFor(m))))
    // choiceFor closes over mergeChoices and plan; both are deps below.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [plan, mergeChoices, bundleId])

  // A raw value's current mapping *for this rule*: explicit entry wins, else what
  // the server resolved (an existing tag of the same name), else unmapped.
  const mappingFor = (
    rule: LayoutRule,
    raw: string,
    resolved: string[] | null,
  ): string[] | null => {
    const key = raw.toLowerCase()
    if (key in rule.value_map) return rule.value_map[key]
    return resolved
  }

  // Set a value's mapping, or `null` to clear the explicit entry and fall back
  // to unmapped. An explicit empty list is "recognised, no tags" — distinct from
  // an absent entry, which the commit refuses; the "No tags" checkbox is the one
  // affordance that reaches the empty state without adding then deleting a chip.
  const setMapping = (index: number, key: string, tags: string[] | null) =>
    setRules((prev) =>
      prev.map((rule, i) => {
        if (i !== index) return rule
        if (tags === null) {
          const { [key]: _drop, ...rest } = rule.value_map
          return { ...rule, value_map: rest }
        }
        return { ...rule, value_map: { ...rule.value_map, [key]: tags } }
      }),
    )

  const save = async () => {
    if (!saveName.trim() || live.length === 0) return
    try {
      // Blank rules are scratch, not template: they'd restore as empty blocks in
      // every future import. The enabled toggles *are* saved — they become the
      // template's defaults.
      await api.createImportLayout({ name: saveName.trim(), rules: live, flatten })
      setSaveName('')
      setSaveError('')
      await queryClient.invalidateQueries({ queryKey: ['import-layouts'] })
    } catch (err) {
      setSaveError(err instanceof Error ? err.message : String(err))
    }
  }

  // One value→tags row, shared by the live captures and the disabled stale ones.
  // `mapKey` (lowercased) is both the React key and the value-map key setMapping
  // writes; `label` is what's shown (the humanised capture, or the raw key).
  const valueRow = (
    index: number,
    mapKey: string,
    label: string,
    mapped: string[] | null,
    disabled: boolean,
  ) => (
    <Stack
      key={mapKey}
      direction="row"
      spacing={1}
      sx={{ alignItems: 'center', py: 0.5, opacity: disabled ? 0.5 : 1 }}
    >
      <Typography
        variant="body2"
        sx={{ fontFamily: 'monospace', fontSize: 12, minWidth: 180 }}
        noWrap
      >
        {label}
      </Typography>
      <Autocomplete
        multiple
        freeSolo
        size="small"
        disabled={disabled}
        options={(vocab ?? []).map((t) => t.name)}
        value={mapped ?? []}
        onChange={(_, tags) => setMapping(index, mapKey, tags)}
        renderInput={(props) => (
          <TextField
            {...props}
            error={!disabled && mapped === null}
            placeholder={!disabled && mapped === null ? 'pick tags, or tick “No tags”' : undefined}
          />
        )}
        sx={{ flexGrow: 1, maxWidth: 420 }}
      />
      <Tooltip title="Recognise this value but add no variant tags — its files land in a plain, untagged variant. Use this to confirm you meant to leave it blank.">
        <FormControlLabel
          control={
            <Checkbox
              size="small"
              disabled={disabled}
              checked={mapped !== null && mapped.length === 0}
              onChange={(e) => setMapping(index, mapKey, e.target.checked ? [] : null)}
            />
          }
          label={<Typography variant="body2">No tags</Typography>}
          sx={{ ml: 0, whiteSpace: 'nowrap' }}
        />
      </Tooltip>
    </Stack>
  )

  /// One rule's editor block: its label and switch, its pattern, its group roles
  /// and its value map. Everything inside is scoped to the rule — group 1 here
  /// has nothing to do with group 1 next door.
  const ruleBlock = (rule: LayoutRule, index: number) => {
    const found = plan?.rules?.[index]
    // The group-role rows to show: every group this pattern captures, plus any
    // group the rule assigned a role to that the edited pattern no longer
    // produces. The latter render disabled — while you edit a pattern down to too
    // few groups, its role assignments stay visible (and preserved in state) rather
    // than vanishing, so they come straight back when the group returns.
    const planGroups = new Map((found?.groups ?? []).map((g) => [g.index, g] as const))
    const groupIndices = [
      ...new Set<number>([...planGroups.keys(), ...Object.keys(rule.roles).map(Number)]),
    ].sort((a, b) => a - b)

    // Likewise for value-map rows: the values this pattern captures now, plus any
    // mapping carried on the rule whose value isn't captured now — shown disabled.
    const values = found?.values ?? []
    const activeValueKeys = new Set(values.map((v) => v.raw.toLowerCase()))
    const staleValueKeys = Object.keys(rule.value_map)
      .filter((k) => !activeValueKeys.has(k))
      .sort()

    const roleOf = (group: number): GroupRole => rule.roles[String(group)] ?? 'ignore'

    return (
      <Box
        key={index}
        sx={{
          border: 1,
          borderColor: 'divider',
          borderRadius: 1,
          p: 1.5,
          mb: 1.5,
        }}
      >
        <Stack direction="row" spacing={1} sx={{ alignItems: 'center', mb: 1 }}>
          <Tooltip
            title={
              rule.enabled
                ? 'Switch this rule off for this import — it then contributes nothing'
                : 'Switch this rule back on'
            }
          >
            <Switch
              size="small"
              checked={rule.enabled}
              onChange={(e) => updateRule(index, { enabled: e.target.checked })}
            />
          </Tooltip>
          <TextField
            size="small"
            variant="standard"
            placeholder={`Rule ${index + 1}`}
            value={rule.name}
            onChange={(e) => updateRule(index, { name: e.target.value })}
            sx={{ width: 200 }}
          />
          <Box sx={{ flexGrow: 1 }} />
          <Tooltip title="Remove this rule">
            <IconButton
              size="small"
              onClick={() => {
                setRules((prev) => prev.filter((_, i) => i !== index))
                setSelected('custom')
              }}
            >
              <DeleteOutlineIcon fontSize="small" />
            </IconButton>
          </Tooltip>
        </Stack>

        {/* The body greys out when the rule is off, but stays editable: you
            switch a rule off to see what it was contributing, then tune it. */}
        <Box sx={{ opacity: rule.enabled ? 1 : 0.5 }}>
          <TextField
            fullWidth
            size="small"
            label="Pattern"
            // A long pattern is unreadable on one clipped line — grow with its
            // content instead, capping at maxRows so it scrolls rather than
            // swallowing the panel.
            multiline
            maxRows={10}
            value={rule.pattern}
            onChange={(e) => {
              updateRule(index, { pattern: e.target.value })
              setSelected('custom')
            }}
            helperText="Searched anywhere in each staged file’s full path. Assign its capture groups below."
            slotProps={{ input: { sx: { fontFamily: 'monospace', fontSize: 13 } } }}
            sx={{ mb: 1 }}
          />

          {groupIndices.map((group) => {
            const info = planGroups.get(group)
            return (
              <Stack
                key={group}
                direction="row"
                spacing={1}
                sx={{ alignItems: 'center', flexWrap: 'wrap', py: 0.25, opacity: info ? 1 : 0.5 }}
              >
                <Chip
                  size="small"
                  label={`Group ${group}`}
                  sx={{ bgcolor: ROLE_STYLES[roleOf(group)].bg, minWidth: 76 }}
                />
                <Typography
                  variant="body2"
                  sx={{
                    fontFamily: 'monospace',
                    fontSize: 12,
                    color: 'text.secondary',
                    minWidth: 180,
                    flexShrink: 1,
                  }}
                  noWrap
                >
                  {info
                    ? info.examples.join(', ') || '(no captures)'
                    : '(not captured by the current pattern)'}
                </Typography>
                <RadioGroup
                  row
                  value={roleOf(group)}
                  onChange={(e) =>
                    updateRule(index, {
                      roles: { ...rule.roles, [String(group)]: e.target.value as GroupRole },
                    })
                  }
                >
                  {ROLE_ORDER.map((role) => (
                    <FormControlLabel
                      key={role}
                      value={role}
                      disabled={!info}
                      control={<Radio size="small" sx={{ py: 0.25 }} />}
                      label={<Typography variant="body2">{ROLE_STYLES[role].label}</Typography>}
                    />
                  ))}
                </RadioGroup>
              </Stack>
            )
          })}

          {(values.length > 0 || staleValueKeys.length > 0) && (
            <Box sx={{ mt: 1 }}>
              <Typography variant="subtitle2" sx={{ mb: 0.5 }}>
                Captured values → variant tags
              </Typography>
              {values.map((value) =>
                valueRow(
                  index,
                  value.raw.toLowerCase(),
                  value.raw,
                  mappingFor(rule, value.raw, value.tags),
                  false,
                ),
              )}
              {staleValueKeys.map((key) => valueRow(index, key, key, rule.value_map[key], true))}
            </Box>
          )}
        </Box>
      </Box>
    )
  }

  return (
    <Box sx={{ mb: 2 }}>
      <Stack direction="row" spacing={1} sx={{ alignItems: 'center', mb: 1 }}>
        <FormControl size="small" sx={{ minWidth: 240 }}>
          <InputLabel id="layout-select">Layout</InputLabel>
          <Select
            labelId="layout-select"
            label="Layout"
            value={selected}
            onChange={(e) => {
              const value = e.target.value
              setSelected(value)
              if (value === '') return clear()
              // "Custom" from nothing needs somewhere to type.
              if (value === 'custom') {
                if (rules.length === 0) setRules([blankRule()])
                return
              }
              const layout = layouts?.find((l) => l.id === value)
              if (layout) applyLayout(layout)
            }}
          >
            <MenuItem value="">
              <em>None — everything lands unsorted</em>
            </MenuItem>
            {(layouts ?? []).map((layout) => (
              <MenuItem key={layout.id} value={layout.id}>
                {layout.name}
                {coverage && coverage[layout.id] >= 0 && (
                  <Typography
                    component="span"
                    variant="body2"
                    color="text.secondary"
                    sx={{ ml: 1 }}
                  >
                    {coverage[layout.id]}/{fileCount} files
                  </Typography>
                )}
              </MenuItem>
            ))}
            <MenuItem value="custom">Custom pattern…</MenuItem>
          </Select>
        </FormControl>
        {plan && (
          <Typography variant="body2" color="text.secondary">
            matches {plan.matched} of {plan.total} files
            {plan.carved !== plan.matched && ` (${plan.carved} carved)`}
            {plan.matched < plan.total &&
              (keepUnmatched ? ' — the rest stay staged here' : ' — the rest land unsorted')}
          </Typography>
        )}
      </Stack>

      {(selected !== '' || rules.length > 0) && (
        <>
          {planError && (
            <Typography variant="body2" color="error" sx={{ mb: 1 }}>
              {planError}
            </Typography>
          )}

          {/* display:flex stacks the two option rows; inline-flex would let them
              sit side by side and ragged. */}
          <FormControlLabel
            sx={{ mb: 1, display: 'flex' }}
            control={
              <Checkbox
                size="small"
                checked={flatten}
                onChange={(e) => setFlatten(e.target.checked)}
              />
            }
            label={
              <Box>
                <Typography variant="body2">Discard folders</Typography>
                <Typography variant="caption" color="text.secondary">
                  The carve has already read them — keep the files flat inside the model instead of
                  nested under the folders they came in.
                </Typography>
              </Box>
            }
          />

          <FormControlLabel
            sx={{ mb: 1, display: 'flex' }}
            control={
              <Checkbox
                size="small"
                checked={keepUnmatched}
                onChange={(e) => setKeepUnmatched(e.target.checked)}
              />
            }
            label={
              <Box>
                <Typography variant="body2">Keep unmatched files staged</Typography>
                <Typography variant="caption" color="text.secondary">
                  Import only what the rules match — matched files leave the import; the rest stay
                  here (instead of landing unsorted), so you can carve them at a different target in
                  another pass.
                </Typography>
              </Box>
            }
          />

          {rules.map(ruleBlock)}

          <Button
            size="small"
            startIcon={<AddIcon />}
            onClick={() => {
              setRules((prev) => [...prev, blankRule()])
              setSelected('custom')
            }}
            sx={{ mb: 1.5 }}
          >
            Add rule
          </Button>

          {plan && plan.models.length > 0 && (
            <Box sx={{ mb: 1.5 }}>
              <Typography variant="subtitle2" sx={{ mb: 0.5 }}>
                Will become
              </Typography>
              {plan.models.slice(0, 30).map((model, i) => (
                <Box key={`${model.name}-${i}`} sx={{ mb: 0.5 }}>
                  <Stack
                    direction="row"
                    spacing={1}
                    sx={{ alignItems: 'center', flexWrap: 'wrap' }}
                  >
                    <ViewInArIcon sx={{ fontSize: 16, opacity: 0.6 }} />
                    <Typography variant="body2" sx={{ fontWeight: 600 }}>
                      {model.name || 'This model'}
                    </Typography>
                    {model.tags.map((tag) => (
                      <Chip
                        key={tag}
                        size="small"
                        label={tag}
                        sx={{ bgcolor: ROLE_STYLES.model_tag.bg }}
                      />
                    ))}
                    <Typography variant="body2" color="text.secondary">
                      {model.file_count} file{model.file_count === 1 ? '' : 's'}
                    </Typography>
                    {bundleId && plan.members && plan.members.length > 0 && (
                      <FormControl size="small" sx={{ minWidth: 200 }}>
                        <Select
                          value={choiceFor(model)}
                          onChange={(e) =>
                            setMergeChoices((prev) => ({
                              ...prev,
                              [modelKey(model)]: e.target.value,
                            }))
                          }
                          sx={{ fontSize: 13 }}
                        >
                          <MenuItem value="new">
                            <em>New model</em>
                          </MenuItem>
                          {plan.members.map((member) => (
                            <MenuItem key={member.id} value={member.id}>
                              Merge into “{member.name}”
                              {member.tags.length ? ` (${member.tags.join(', ')})` : ''}
                            </MenuItem>
                          ))}
                        </Select>
                      </FormControl>
                    )}
                  </Stack>
                  {model.variants.map((variant, j) => (
                    <Stack
                      key={j}
                      direction="row"
                      spacing={0.75}
                      sx={{ alignItems: 'center', pl: 3.5, py: 0.125 }}
                    >
                      {variant.tags.length ? (
                        variant.tags.map((tag) => (
                          <Chip
                            key={tag}
                            size="small"
                            label={tag}
                            sx={{ bgcolor: ROLE_STYLES.variant_tag.bg }}
                          />
                        ))
                      ) : (
                        <Chip
                          size="small"
                          label={target === 'bundle' ? 'anonymous variant' : 'unsorted'}
                          variant="outlined"
                        />
                      )}
                      <Typography variant="body2" color="text.secondary">
                        {variant.file_count}
                      </Typography>
                    </Stack>
                  ))}
                </Box>
              ))}
              {plan.models.length > 30 && (
                <Typography variant="body2" color="text.secondary">
                  …and {plan.models.length - 30} more models
                </Typography>
              )}
            </Box>
          )}

          {plan && (
            <Stack direction="row" spacing={1} sx={{ alignItems: 'center' }}>
              <TextField
                size="small"
                label="Save as layout"
                value={saveName}
                onChange={(e) => setSaveName(e.target.value)}
                error={!!saveError}
                helperText={saveError || undefined}
              />
              <Tooltip title="Save these rules, roles and value maps for the next archive">
                <span>
                  <IconButton onClick={save} disabled={!saveName.trim()}>
                    <SaveIcon />
                  </IconButton>
                </span>
              </Tooltip>
            </Stack>
          )}
        </>
      )}
    </Box>
  )
}

/// The staged file list, annotated by the active plan: capture groups
/// highlighted in role colours, resolved model/tags chips trailing each row.
export function AnnotatedFileList({
  files,
  annotations,
  rules,
}: {
  files: FileRecord[]
  annotations: FileAnnotation[]
  /** the active rules, only so a self-contradicting one can be named in the
      warning marker */
  rules?: LayoutRule[]
}) {
  const byId = useMemo(() => new Map(annotations.map((a) => [a.id, a])), [annotations])
  const shown = files.slice(0, 400)
  return (
    // The full path must always be legible and never clipped, so it wraps rather
    // than scrolling. Wrapping breaks *only* after a path divider ('/'): each
    // segment is an unbreakable unit (so a filename with spaces never splits mid-
    // word) and a <wbr> after each slash is the sole break opportunity — see
    // WrappedPath. Continuation lines hang-indent (paddingLeft + negative
    // textIndent) so a wrapped path reads as one path, not a new row.
    <Box>
      {shown.map((file) => {
        const annotation = byId.get(file.id)
        const parts = annotation?.parts ?? [
          { text: file.path ? `${file.path}/${file.filename}` : file.filename },
        ]
        const invalid = annotation?.invalid_rules ?? []
        return (
          <Stack
            key={file.id}
            direction="row"
            spacing={1}
            sx={{ alignItems: 'flex-start', py: 0.25, opacity: annotation?.matched ? 1 : 0.6 }}
          >
            <InsertDriveFileIcon sx={{ fontSize: 14, opacity: 0.5, flexShrink: 0, mt: '2px' }} />
            <Typography
              variant="body2"
              sx={{
                fontFamily: 'monospace',
                fontSize: 12,
                flexGrow: 1,
                minWidth: 0,
                whiteSpace: 'normal',
                pl: '1.5em',
                textIndent: '-1.5em',
              }}
            >
              <WrappedPath parts={parts} />
            </Typography>
            <Stack direction="row" spacing={0.5} sx={{ flexShrink: 0, alignItems: 'center' }}>
              {invalid.length > 0 && (
                // Not a blocker: the rule found two different answers here, so it
                // was dropped for this file and the others carried on.
                <Tooltip
                  title={`${invalid
                    .map((i) => (rules?.[i] ? ruleLabel(rules[i], i) : `Rule ${i + 1}`))
                    .join(', ')} captured two different values in this path, so ${
                    invalid.length === 1 ? 'it was' : 'they were'
                  } ignored for this file.`}
                >
                  <WarningAmberIcon color="warning" sx={{ fontSize: 16 }} />
                </Tooltip>
              )}
              {annotation?.matched && (
                <>
                  {annotation.model_name && (
                    <Chip
                      size="small"
                      label={annotation.model_name}
                      sx={{ bgcolor: ROLE_STYLES.model_name.bg }}
                    />
                  )}
                  {annotation.creator_ref && (
                    <Chip
                      size="small"
                      label={annotation.creator_ref}
                      sx={{ bgcolor: ROLE_STYLES.creator_ref.bg }}
                    />
                  )}
                  {annotation.model_tags.map((tag) => (
                    <Chip
                      key={tag}
                      size="small"
                      label={tag}
                      sx={{ bgcolor: ROLE_STYLES.model_tag.bg }}
                    />
                  ))}
                  {annotation.variant_tags.map((tag) => (
                    <Chip
                      key={tag}
                      size="small"
                      label={tag}
                      sx={{ bgcolor: ROLE_STYLES.variant_tag.bg }}
                    />
                  ))}
                  {annotation.unmapped.map((raw) => (
                    <Chip
                      key={raw}
                      size="small"
                      label={`${raw}?`}
                      color="warning"
                      variant="outlined"
                    />
                  ))}
                </>
              )}
            </Stack>
          </Stack>
        )
      })}
      {files.length > shown.length && (
        <Typography variant="body2" color="text.secondary" sx={{ mt: 0.5 }}>
          …and {files.length - shown.length} more files
        </Typography>
      )}
    </Box>
  )
}

/// Render a file's path so it wraps only after a slash. The path is split into
/// '/'-delimited segments (the slash kept on the end of its segment, so the break
/// lands *after* the divider); each segment is an inline `white-space: nowrap`
/// box that never breaks internally, and a `<wbr>` between segments is the only
/// break opportunity CSS is offered. Capture highlighting survives the split —
/// each part carries its role through, so a capture that straddles a slash still
/// colours on both sides.
function WrappedPath({ parts }: { parts: FileAnnotation['parts'] }) {
  const segments: FileAnnotation['parts'][] = []
  let current: FileAnnotation['parts'] = []
  for (const part of parts) {
    let buf = ''
    for (const ch of part.text) {
      buf += ch
      if (ch === '/') {
        current.push({ text: buf, role: part.role })
        segments.push(current)
        current = []
        buf = ''
      }
    }
    if (buf) current.push({ text: buf, role: part.role })
  }
  if (current.length) segments.push(current)

  return (
    <>
      {segments.map((seg, si) => (
        <Fragment key={si}>
          <Box component="span" sx={{ whiteSpace: 'nowrap' }}>
            {seg.map((run, ri) =>
              run.role !== undefined ? (
                <Box
                  key={ri}
                  component="span"
                  sx={{
                    bgcolor: ROLE_STYLES[run.role].bg,
                    borderRadius: 0.5,
                    px: 0.25,
                  }}
                >
                  {run.text}
                </Box>
              ) : (
                <span key={ri}>{run.text}</span>
              ),
            )}
          </Box>
          {si < segments.length - 1 && <wbr />}
        </Fragment>
      ))}
    </>
  )
}
