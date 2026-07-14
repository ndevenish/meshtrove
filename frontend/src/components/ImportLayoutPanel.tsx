import { useEffect, useMemo, useRef, useState } from 'react'
import {
  Autocomplete,
  Box,
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
  TextField,
  Tooltip,
  Typography,
} from '@mui/material'
import SaveIcon from '@mui/icons-material/Save'
import ViewInArIcon from '@mui/icons-material/ViewInAr'
import InsertDriveFileIcon from '@mui/icons-material/InsertDriveFile'
import { useQuery, useQueryClient } from '@tanstack/react-query'

import {
  api,
  type FileAnnotation,
  type FileRecord,
  type GroupRole,
  type ImportLayout,
  type LayoutPlan,
  type LayoutSpec,
  type PlanTarget,
} from '../api'

/// Highlight colours per role — alpha overlays so they read on light and dark.
export const ROLE_STYLES: Record<GroupRole, { label: string; bg: string }> = {
  model_name: { label: 'Model name', bg: 'rgba(250, 104, 49, 0.30)' },
  model_tag: { label: 'Model tag', bg: 'rgba(142, 36, 170, 0.25)' },
  variant_tag: { label: 'Variant tag', bg: 'rgba(46, 125, 50, 0.28)' },
  ignore: { label: 'Ignore', bg: 'transparent' },
}
const ROLE_ORDER: GroupRole[] = ['model_name', 'model_tag', 'variant_tag', 'ignore']

/// The layout carve panel: pick or write a pattern, assign each capture group
/// a role, map raw captured values onto variant tags, and preview the result.
/// All matching happens server-side (`POST /api/imports/{id}/plan`) — the
/// pattern is an opaque string here, and the commit runs the same analysis,
/// so what this previews is exactly what commits.
export default function ImportLayoutPanel({
  importId,
  fileCount,
  unpacking,
  target,
  onPlan,
}: {
  importId: string
  /** re-plan when this grows */
  fileCount: number
  /** the archive is still extracting: plan against the finished tree, not a
      half-unpacked one that changes under us every second */
  unpacking: boolean
  target: PlanTarget
  onPlan: (spec: LayoutSpec | null, plan: LayoutPlan | null) => void
}) {
  const queryClient = useQueryClient()
  const [selected, setSelected] = useState<string>('')
  const [pattern, setPattern] = useState('')
  const [roles, setRoles] = useState<Record<string, GroupRole>>({})
  const [valueMap, setValueMap] = useState<Record<string, string[]>>({})
  const [plan, setPlan] = useState<LayoutPlan | null>(null)
  const [planError, setPlanError] = useState('')
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
    setPattern(layout.pattern)
    setRoles(layout.roles)
    setValueMap(layout.value_map)
  }

  const clear = () => {
    setPattern('')
    setRoles({})
    setValueMap({})
    setPlan(null)
    setPlanError('')
  }

  // Debounced server-side dry run; also the sole source of the parent's spec.
  const onPlanRef = useRef(onPlan)
  onPlanRef.current = onPlan
  useEffect(() => {
    if (!pattern.trim()) {
      setPlan(null)
      setPlanError('')
      onPlanRef.current(null, null)
      return
    }
    // Nothing to plan against yet: the tree is still arriving, and each arrival
    // would otherwise fire another dry run whose answer is obsolete on landing.
    if (unpacking) return
    const spec: LayoutSpec = { pattern, roles, value_map: valueMap }
    const timer = setTimeout(async () => {
      try {
        const result = await api.planImport(importId, spec, target)
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
  }, [importId, pattern, roles, valueMap, target, fileCount, unpacking])

  const roleOf = (group: number): GroupRole => roles[String(group)] ?? 'ignore'

  // A raw value's current mapping: explicit entry wins, else what the server
  // resolved (an existing tag of the same name), else unmapped.
  const mappingFor = (raw: string, resolved: string[] | null): string[] | null => {
    const key = raw.toLowerCase()
    if (key in valueMap) return valueMap[key]
    return resolved
  }

  const save = async () => {
    if (!saveName.trim() || !pattern.trim()) return
    try {
      await api.createImportLayout({
        name: saveName.trim(),
        pattern,
        roles,
        value_map: valueMap,
      })
      setSaveName('')
      setSaveError('')
      await queryClient.invalidateQueries({ queryKey: ['import-layouts'] })
    } catch (err) {
      setSaveError(err instanceof Error ? err.message : String(err))
    }
  }

  const variantValues = plan?.values ?? []

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
              if (value === 'custom') return
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
            {plan.matched < plan.total && ' — the rest land unsorted'}
          </Typography>
        )}
      </Stack>

      {(selected !== '' || pattern) && (
        <>
          <TextField
            fullWidth
            size="small"
            label="Pattern"
            value={pattern}
            onChange={(e) => {
              setPattern(e.target.value)
              setSelected('custom')
            }}
            error={!!planError}
            helperText={
              planError ||
              'Matched against each staged file’s full path (anchored). Assign the capture groups below.'
            }
            slotProps={{ input: { sx: { fontFamily: 'monospace', fontSize: 13 } } }}
            sx={{ mb: 1.5 }}
          />

          {plan && plan.groups.length > 0 && (
            <Box sx={{ mb: 1.5 }}>
              {plan.groups.map((group) => (
                <Stack
                  key={group.index}
                  direction="row"
                  spacing={1}
                  sx={{ alignItems: 'center', flexWrap: 'wrap', py: 0.25 }}
                >
                  <Chip
                    size="small"
                    label={`Group ${group.index}`}
                    sx={{ bgcolor: ROLE_STYLES[roleOf(group.index)].bg, minWidth: 76 }}
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
                    {group.examples.join(', ') || '(no captures)'}
                  </Typography>
                  <RadioGroup
                    row
                    value={roleOf(group.index)}
                    onChange={(e) =>
                      setRoles({ ...roles, [String(group.index)]: e.target.value as GroupRole })
                    }
                  >
                    {ROLE_ORDER.map((role) => (
                      <FormControlLabel
                        key={role}
                        value={role}
                        control={<Radio size="small" sx={{ py: 0.25 }} />}
                        label={<Typography variant="body2">{ROLE_STYLES[role].label}</Typography>}
                      />
                    ))}
                  </RadioGroup>
                </Stack>
              ))}
            </Box>
          )}

          {variantValues.length > 0 && (
            <Box sx={{ mb: 1.5 }}>
              <Typography variant="subtitle2" sx={{ mb: 0.5 }}>
                Captured values → variant tags
              </Typography>
              {variantValues.map((value) => {
                const mapped = mappingFor(value.raw, value.tags)
                return (
                  <Stack
                    key={value.raw.toLowerCase()}
                    direction="row"
                    spacing={1}
                    sx={{ alignItems: 'center', py: 0.5 }}
                  >
                    <Typography
                      variant="body2"
                      sx={{ fontFamily: 'monospace', fontSize: 12, minWidth: 180 }}
                      noWrap
                    >
                      {value.raw}
                    </Typography>
                    <Autocomplete
                      multiple
                      freeSolo
                      size="small"
                      options={(vocab ?? []).map((t) => t.name)}
                      value={mapped ?? []}
                      onChange={(_, tags) =>
                        setValueMap({ ...valueMap, [value.raw.toLowerCase()]: tags })
                      }
                      renderInput={(props) => (
                        <TextField
                          {...props}
                          error={mapped === null}
                          placeholder={mapped === null ? 'unmapped — pick tags' : undefined}
                        />
                      )}
                      sx={{ flexGrow: 1, maxWidth: 420 }}
                    />
                  </Stack>
                )
              })}
            </Box>
          )}

          {plan && plan.models.length > 0 && (
            <Box sx={{ mb: 1.5 }}>
              <Typography variant="subtitle2" sx={{ mb: 0.5 }}>
                Will become
              </Typography>
              {plan.models.slice(0, 30).map((model, i) => (
                <Box key={`${model.name}-${i}`} sx={{ mb: 0.5 }}>
                  <Stack direction="row" spacing={1} sx={{ alignItems: 'center' }}>
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
                        <Chip size="small" label="unsorted" variant="outlined" />
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
              <Tooltip title="Save this pattern, roles and value map for the next archive">
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
  roles,
}: {
  files: FileRecord[]
  annotations: FileAnnotation[]
  roles: Record<string, GroupRole>
}) {
  const byId = useMemo(() => new Map(annotations.map((a) => [a.id, a])), [annotations])
  const shown = files.slice(0, 400)
  return (
    <Box>
      {shown.map((file) => {
        const annotation = byId.get(file.id)
        return (
          <Stack
            key={file.id}
            direction="row"
            spacing={1}
            sx={{ alignItems: 'center', py: 0.25, opacity: annotation?.matched ? 1 : 0.6 }}
          >
            <InsertDriveFileIcon sx={{ fontSize: 14, opacity: 0.5, flexShrink: 0 }} />
            <Typography
              variant="body2"
              sx={{ fontFamily: 'monospace', fontSize: 12, flexGrow: 1, minWidth: 0 }}
              noWrap
            >
              {annotation ? (
                annotation.parts.map((part, i) =>
                  part.group !== undefined ? (
                    <Box
                      key={i}
                      component="span"
                      sx={{
                        bgcolor: ROLE_STYLES[roles[String(part.group)] ?? 'ignore'].bg,
                        borderRadius: 0.5,
                        px: 0.25,
                      }}
                    >
                      {part.text}
                    </Box>
                  ) : (
                    <span key={i}>{part.text}</span>
                  ),
                )
              ) : (
                <span>{file.path ? `${file.path}/${file.filename}` : file.filename}</span>
              )}
            </Typography>
            {annotation?.matched && (
              <Stack direction="row" spacing={0.5} sx={{ flexShrink: 0, alignItems: 'center' }}>
                {annotation.model_name && (
                  <Chip
                    size="small"
                    label={annotation.model_name}
                    sx={{ bgcolor: ROLE_STYLES.model_name.bg }}
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
              </Stack>
            )}
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
