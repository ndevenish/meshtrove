// Thin typed fetch layer over the backend API (same origin, cookie auth).

export type Role = 'admin' | 'editor' | 'viewer'

export interface User {
  id: string
  username: string
  role: Role
}

/// A user account as the admin user-management screen sees it.
export interface UserAccount {
  id: string
  username: string
  role: Role
  created_at: string
}

/** What a custom field *is*: how its value is entered and rendered. */
export type CustomFieldKind = 'text' | 'checkbox' | 'choice' | 'rating' | 'file'

/** Who a custom field is shown to at all — its value and its existence. */
export type CustomFieldVisibility = 'anonymous' | 'viewer' | 'editor' | 'admin'

/** Kind-specific settings: `choices` for a choice field, `max` for a rating. */
export interface CustomFieldOptions {
  choices?: string[]
  max?: number
}

/// An admin-defined extra metadata field, available meshtrove-wide on every
/// model and/or bundle.
export interface CustomFieldDef {
  id: string
  /** Stable slug; what scraped metadata keys are matched against. */
  key: string
  name: string
  kind: CustomFieldKind
  options: CustomFieldOptions
  applies_to_models: boolean
  applies_to_bundles: boolean
  /** Writing this field on a bundle copies the value down to member models. */
  bundle_persists_to_model: boolean
  /** ...even if the member model already had a value of its own. */
  bundle_persist_overwrites: boolean
  visibility: CustomFieldVisibility
  position: number
}

export type CustomFieldInput = Omit<CustomFieldDef, 'id'>

/** The file behind a file-kind value; downloaded through the usual file route. */
export interface CustomFieldFile {
  file_id: string
  filename: string
  mime: string | null
  size: number
}

/// One field as it appears on a model or bundle: the definition, plus whatever
/// this owner has stored under it. Every applicable and visible field is listed,
/// set or not, so an editor sees the blanks it could fill in.
export interface CustomFieldValue {
  field: CustomFieldDef
  /** null when unset; always null for a file field — see `file` */
  value: string | boolean | number | null
  file: CustomFieldFile | null
}

/** One scalar write, carried in the model/bundle edit. A null value clears. */
export interface CustomFieldValueInput {
  field_id: string
  value: string | boolean | number | null
}

/** What a custom field value hangs off. An import stages one until the commit
    knows whether it is a model's or a bundle's. */
export type CustomFieldOwner = 'models' | 'bundles' | 'imports'

export interface Creator {
  id: string
  name: string
  kind: 'author' | 'company' | 'site'
  url: string | null
  notes: string | null
  model_count: number
}

/** A tag from the flat variant vocabulary ("32mm", "supported"). */
export interface VariantTag {
  id: string
  name: string
  description: string | null
  variant_count: number
}

export interface Tag {
  id: string
  name: string
  model_count: number
}

export interface ModelSummary {
  id: string
  name: string
  slug: string
  creator_id: string | null
  creator_name: string | null
  primary_image_id: string | null
  tags: string[]
  like_count: number
  /** whether the *calling* user has liked it — what the heart button renders */
  liked: boolean
  variant_count: number
  matched_variant_ids: string[] | null
  updated_at: string
}

export interface SearchResults {
  models: ModelSummary[]
  total: number
  page: number
  per_page: number
}

export interface FileRecord {
  id: string
  blob_sha256: string
  path: string
  filename: string
  mime: string | null
  kind: 'model' | 'project' | 'raw' | 'document' | 'archive' | 'other'
  size: number
  created_at: string
  /** How this archive's unpack went. `null` when no unpack was ever queued —
      a non-archive, an export awaiting restore, or a format nothing here
      opens. Not the same as "unpacked fine", and must not be shown as such. */
  unpack: 'pending' | 'done' | 'failed' | null
}

export interface VariantDetail {
  id: string
  model_id: string
  /** Optional display label; null for the anonymous variant */
  name: string | null
  /** The tag set that identifies this variant; empty = anonymous */
  tags: string[]
  print_notes: string | null
  derived_from_variant_id: string | null
  file_count: number
  total_size: number
}

export interface ImageRecord {
  id: string
  kind: string
  is_primary: boolean
  /** set when the image belongs to a variant of the model, not the model itself */
  variant_id?: string | null
}

export interface ModelDetail {
  id: string
  name: string
  slug: string
  creator_id: string | null
  creator_name: string | null
  /** the creator's own id/SKU for the model — free text, not the creators FK */
  creator_ref: string | null
  /** the creator's version for the model — free text ("v2", "2024 rework") */
  model_version: string | null
  source_url: string | null
  license: string | null
  purchase_price: number | null
  purchase_date: string | null
  order_ref: string | null
  tags: string[]
  description_md: string | null
  /** every custom field applying to models that the caller may see, set or not */
  custom_fields: CustomFieldValue[]
  variants: VariantDetail[]
  images: ImageRecord[]
  /** bundles this model belongs to */
  bundles: BundleRef[]
  created_by: string
}

export interface BundleRef {
  id: string
  slug: string
  name: string
}

export interface Revision {
  id: string
  body_md: string
  label: string | null
  author: string
  created_at: string
}

export type DescOwner = 'models' | 'bundles'

export interface BundleSummary {
  id: string
  name: string
  slug: string
  creator_id: string | null
  creator_name: string | null
  primary_image_id: string | null
  tags: string[]
  model_count: number
  updated_at: string
}

export interface BundleDetail {
  id: string
  name: string
  slug: string
  creator_id: string | null
  creator_name: string | null
  source_url: string | null
  tags: string[]
  description_md: string | null
  /** every custom field applying to bundles that the caller may see, set or not */
  custom_fields: CustomFieldValue[]
  models: ModelSummary[]
  images: ImageRecord[]
  /** primary categories (import sections), in tab order; each is a model tag a
      member may carry */
  categories: string[]
  created_by: string
}

/// What a bulk member retag did. Counts are tag *assignments*, not distinct
/// tags — 3 tags across 12 models reads as 36, and re-running the same retag
/// reads as 0 because the adds already exist.
export interface MemberTagsResult {
  models_updated: number
  tags_added: number
  tags_removed: number
}

export interface BundleResults {
  bundles: BundleSummary[]
  total: number
  page: number
  per_page: number
}

/// What happens to a bundle's member models when the bundle is deleted:
/// `keep` unlinks them, `delete` deletes all, `delete_exclusive` deletes only the
/// ones not also in another bundle.
export type BundleMemberDisposition = 'keep' | 'delete' | 'delete_exclusive'

/// What becomes of the bundle merged *into* another: `keep` leaves it standing
/// (its models simply belong to both), `delete` moves everything it has across
/// and deletes it.
export type OtherBundleDisposition = 'keep' | 'delete'

/// One row in the unified browse (models + bundles mixed). `count` is
/// variant_count for models, model_count for bundles.
export interface BrowseItem {
  type: 'model' | 'bundle'
  id: string
  name: string
  slug: string
  creator_id: string | null
  creator_name: string | null
  primary_image_id: string | null
  tags: string[]
  like_count: number
  /** whether the *calling* user has liked it — what the heart button renders */
  liked: boolean
  count: number
  updated_at: string
}

export interface BrowseResults {
  items: BrowseItem[]
  total: number
  page: number
  per_page: number
}

/// One staged folder of an import, counted rather than listed.
export interface ImportFolder {
  path: string
  files: number
  bytes: number
}

/// A dropped archive, staged. Neither a model nor a bundle: it stays out of
/// browse until it is committed to one (see `commitImport`).
export interface ImportSummary {
  id: string
  name: string
  created_by: string
  created_at: string
  file_count: number
  /** its archive is still unpacking; committing is refused until this clears */
  unpacking: boolean
  /** archives still to open. Goes *up* when one of them turns out to hold more
      archives — a pack of packs has no knowable shape until it is opened */
  archives_left: number
  /** files staged by the jobs running right now, out of what those jobs have
      left to do. Not the import's eventual total, which nothing can know until
      the last archive is open. Both 0 when nothing has reported */
  staging_done: number
  staging_total: number
  /** the dropped archive is a MeshTrove export — restore it rather than carve it */
  is_export: boolean
  /** a "keep unmatched files" carve already placed some of this import; what's
      staged now is the remainder, awaiting another pass */
  partial: boolean
}

/// One entry sitting in the server-side dropbox (`<store>/imports`) — an archive
/// or a folder an admin put there directly, waiting to be staged with a button
/// instead of pushed back through the browser.
export interface DropboxEntry {
  /** its name in the dropbox; the handle `pickUpDropboxEntry` takes */
  name: string
  is_dir: boolean
  /** files a pickup would stage — a folder's whole tree */
  file_count: number
  size: number
  modified: string | null
  /** a pickup of this entry is already queued or running */
  importing: boolean
  /** when it was last picked up — a pickup leaves the dropbox untouched, so
      without this an imported entry looks identical to a new one */
  imported_at: string | null
  /** imported before, but its file count or size has changed since: same name,
      different contents */
  changed_since_import: boolean
}

export interface DropboxListing {
  /** absolute path of the dropbox on the server, so an admin knows where to copy to */
  path: string
  entries: DropboxEntry[]
}

// --- Import layout templates (regex-driven carve; see docs/plan.md) ---------

/// What a capture group means. No "variant" role: a variant IS its tag set,
/// so the union of a file's variant-tag captures is its variant.
export type GroupRole =
  | 'model_name'
  | 'creator_ref'
  | 'model_version'
  | 'model_tag'
  | 'variant_tag'
  /** the capture replaces the file's own folder — the general case of `flatten` */
  | 'folder'
  | 'ignore'

/// One regex of a layout. The pattern is a backend (fancy-regex) pattern — an
/// opaque string here; the frontend never runs it. Group numbers and the value
/// map are local to the rule, so two rules can both use group 1 for different
/// things.
export interface LayoutRule {
  /** optional label, shown on the rule's editor block ("scale", "supports") */
  name: string
  pattern: string
  /** capture group number (as a string key) -> role */
  roles: Record<string, GroupRole>
  /** lowercased raw capture -> variant tag names ([] = maps to nothing) */
  value_map: Record<string, string[]>
  /** off = contributes nothing, like a rule that never matches */
  enabled: boolean
}

/// The user-editable layout definition: several small patterns, each searched
/// across the path, whose captured model/variant tags merge.
export interface LayoutSpec {
  rules: LayoutRule[]
  /** drop the folders once the carve has read them: files land with no path */
  flatten?: boolean
  /** commit only what the rules matched: unmatched files stay staged in the
      import, which survives the commit (flagged partial) for another pass.
      Per-import working state — not saved into layout templates. */
  keep_unmatched?: boolean
}

export interface ImportLayout extends LayoutSpec {
  id: string
  name: string
  creator_id: string | null
}

/// A slice of a file's path; `role` set = highlight it in that role's colour.
/// The role travels with the slice because a group number is no longer unique
/// once a layout has several rules.
export interface PathPart {
  text: string
  role?: GroupRole
}

export interface FileAnnotation {
  id: string
  /** matched by at least one enabled rule */
  matched: boolean
  parts: PathPart[]
  /** indices of rules that captured two different values for one group here —
      their output was dropped for this file (a warning, never a blocker) */
  invalid_rules: number[]
  model_name?: string
  creator_ref?: string
  model_version?: string
  model_tags: string[]
  variant_tags: string[]
  /** raw variant-tag captures with no resolution — fill the value map in */
  unmapped: string[]
  /** the path this file will be stored under, replacing the one it came in
      with; absent = it keeps what it has */
  folder?: string
}

export interface PlanVariant {
  /** empty = the model's unsorted bucket */
  tags: string[]
  file_count: number
  example: string
}

export interface PlanModel {
  name: string
  /** the creator's own id/SKU, if a creator_ref group caught one */
  creator_ref?: string
  /** the creator's version, if a model_version group caught one */
  model_version?: string
  tags: string[]
  file_count: number
  variants: PlanVariant[]
  /** carving into an existing bundle: id of the member model this one merges
      onto by default (absent = a new member is created) */
  merge_target?: string | null
}

/// A member of the bundle being merged into — a retarget option for each
/// planned model.
export interface MemberCandidate {
  id: string
  name: string
  tags: string[]
}

export interface GroupInfo {
  index: number
  examples: string[]
}

export interface CapturedValue {
  /** the captured value, humanised (underscores/camelCase → spaces); its
      lowercase is the value-map key, so `Supported_LYCHEE` and `SupportedLychee`
      are one entry */
  raw: string
  /** resolved variant tag names; null = unmapped */
  tags: string[] | null
}

/// The dry run of a layout over an import's staged files. Commit executes the
/// same computation, so this preview is the result.
/// What one rule found, for its own editor block — index-aligned to the spec's
/// rules. Both halves are per-rule because both are read through that rule's
/// own roles and value map.
export interface RulePlan {
  groups: GroupInfo[]
  values: CapturedValue[]
}

export interface LayoutPlan {
  total: number
  matched: number
  carved: number
  rules: RulePlan[]
  models: PlanModel[]
  model_names: string[]
  annotations: FileAnnotation[]
  /** existing members of the bundle being merged into (empty otherwise), for
      the per-model retarget dropdowns */
  members?: MemberCandidate[]
}

/// One-model imports pool everything into variants; bundle imports split
/// member models by the model-name capture.
export type PlanTarget = 'model' | 'bundle'

/// The single decision an import exists to defer: what is this archive?
/// An attached `layout` carves the files into models/variants as it commits.
/// Metadata typed once on the import page. Flattened into the commit body; on a
/// bundle commit it lands on the bundle *and* on every member model the carve
/// creates. A null/absent field says nothing and overwrites nothing.
export interface ImportMeta {
  creator_id?: string | null
  source_url?: string | null
  license?: string | null
  purchase_price?: number | null
  purchase_date?: string | null
  order_ref?: string | null
  tags?: string[]
  description_md?: string | null
  /** admin-defined extra fields typed on the import page; each value goes
      wherever its own definition says it belongs */
  custom_fields?: CustomFieldValueInput[]
}

export type CommitTarget = ImportMeta &
  (
    | { target: 'new_model'; name?: string; layout?: LayoutSpec }
    | {
        target: 'new_bundle'
        name?: string
        /** name left at the archive's — a metadata import may replace it */
        name_autogenerated?: boolean
        layout?: LayoutSpec
      }
    | {
        target: 'bundle'
        bundle_id: string
        layout?: LayoutSpec
        /** per planned-model retarget choices, index-aligned to the plan's
            models: a member id merges onto it, null creates a new member */
        merge_targets?: (string | null)[]
      }
  )

export interface CommitResult {
  type: 'model' | 'bundle'
  id: string
  slug: string
}

export interface Job {
  id: number
  kind: string
  status: 'queued' | 'running' | 'succeeded' | 'failed' | 'cancelled'
  attempts: number
  last_error: string | null
  created_at: string
  payload?: unknown
}

export interface FileUpdate {
  kind?: FileRecord['kind']
  variant_id?: string
  /** carve a bundle-owned file into a member model */
  model_id?: string
  /** push a model-owned file up into a bundle it belongs to */
  bundle_id?: string
  unsorted?: boolean
  filename?: string
  path?: string
}

export interface RendererConfig {
  tool: string
  args: string[]
}

export interface StorageReport {
  path: string
  total_bytes: number
  used_bytes: number
  available_bytes: number
  blob_count: number
  blob_bytes: number
}

export interface CompressionReport {
  blobs: number
  apparent_bytes: number
  allocated_bytes: number
  ratio: number | null
}

export interface GcReport {
  dry_run: boolean
  db_orphans: number
  db_bytes: number
  disk_orphans: number
  disk_bytes: number
  skipped_recent: number
}

export class ApiError extends Error {
  status: number
  constructor(status: number, message: string) {
    super(message)
    this.status = status
  }
}

async function request<T>(path: string, init?: RequestInit): Promise<T> {
  const response = await fetch(path, init)
  if (!response.ok) {
    throw new ApiError(response.status, (await response.text()) || response.statusText)
  }
  if (response.status === 204) return undefined as T
  return response.json()
}

const json = (body: unknown): RequestInit => ({
  method: 'POST',
  headers: { 'content-type': 'application/json' },
  body: JSON.stringify(body),
})

/// Multipart upload with progress. `fetch` cannot report upload progress, so
/// big archive uploads go through XHR to drive a real percentage bar.
export function uploadWithProgress<T>(
  path: string,
  form: FormData,
  onProgress: (fraction: number) => void,
): Promise<T> {
  return new Promise((resolve, reject) => {
    const xhr = new XMLHttpRequest()
    xhr.open('POST', path)
    xhr.upload.onprogress = (e) => {
      if (e.lengthComputable && e.total > 0) onProgress(e.loaded / e.total)
    }
    xhr.onload = () => {
      if (xhr.status >= 200 && xhr.status < 300) {
        onProgress(1)
        resolve(xhr.responseText ? JSON.parse(xhr.responseText) : (undefined as T))
      } else {
        reject(new ApiError(xhr.status, xhr.responseText || xhr.statusText))
      }
    }
    xhr.onerror = () => reject(new ApiError(0, 'network error during upload'))
    xhr.send(form)
  })
}

/// The browse page's current selection, threaded into the tag/variant-tag
/// listings so their counts read as "how many models match this selection *and*
/// this tag" — the numbers filter down as you narrow. Omit it (the autocomplete
/// pickers do) for plain global counts.
export interface TagFilter {
  tags?: string[]
  vtags?: string[]
  q?: string
}

const tagSelectionQuery = (sel: TagFilter): string => {
  const p = new URLSearchParams()
  if (sel.tags?.length) p.set('sel_tags', sel.tags.join(','))
  if (sel.vtags?.length) p.set('sel_vtags', sel.vtags.join(','))
  if (sel.q?.trim()) p.set('sel_q', sel.q.trim())
  const qs = p.toString()
  return qs ? `?${qs}` : ''
}

export const api = {
  /** The *server's* build stamp. Compare against the SPA's own `__APP_VERSION__`
      to notice the server has been redeployed under a still-open page. */
  version: () => request<{ version: string }>('/api/version'),
  me: () => request<User>('/api/me'),
  login: (username: string, password: string) =>
    request<User>('/auth/login', json({ username, password })),
  register: (username: string, password: string) =>
    request<User>('/auth/register', json({ username, password })),
  logout: () => request<void>('/auth/logout', { method: 'POST' }),
  /** Self-service: verify the current password, then set a new one. */
  changePassword: (current_password: string, new_password: string) =>
    request<void>('/auth/password', json({ current_password, new_password })),

  // User administration (admin only).
  users: () => request<UserAccount[]>('/api/users'),
  setUserRole: (id: string, role: Role) =>
    request<UserAccount>(`/api/users/${id}`, { ...json({ role }), method: 'PATCH' }),
  /** Admin resets another user's password (no old-password check). */
  resetUserPassword: (id: string, new_password: string) =>
    request<void>(`/api/users/${id}/password`, json({ new_password })),
  deleteUser: (id: string) => request<void>(`/api/users/${id}`, { method: 'DELETE' }),

  // Custom field definitions: readable by editors (they drive the edit forms),
  // writable by admins only.
  customFields: () => request<CustomFieldDef[]>('/api/custom-fields'),
  createCustomField: (body: CustomFieldInput) =>
    request<CustomFieldDef>('/api/custom-fields', json(body)),
  updateCustomField: (id: string, body: CustomFieldInput) =>
    request<CustomFieldDef>(`/api/custom-fields/${id}`, { ...json(body), method: 'PUT' }),
  deleteCustomField: (id: string) =>
    request<void>(`/api/custom-fields/${id}`, { method: 'DELETE' }),

  /** What an import is holding: every field either side of a commit could
      want, with whatever has been staged against it. */
  importCustomFields: (id: string) =>
    request<CustomFieldValue[]>(`/api/imports/${id}/custom-fields`),

  /** Replace a file-kind value's file. The form carries one `file` part. */
  uploadCustomFieldFile: (owner: CustomFieldOwner, id: string, fieldId: string, form: FormData) =>
    request<CustomFieldValue>(`/api/${owner}/${id}/custom-fields/${fieldId}/file`, {
      method: 'POST',
      body: form,
    }),
  /** Unset one field on one owner, file and all. */
  clearCustomField: (owner: CustomFieldOwner, id: string, fieldId: string) =>
    request<void>(`/api/${owner}/${id}/custom-fields/${fieldId}`, { method: 'DELETE' }),

  searchModels: (params: URLSearchParams) => request<SearchResults>(`/api/models?${params}`),
  model: (id: string) => request<ModelDetail>(`/api/models/${id}`),
  createModel: (body: unknown) => request<ModelDetail>('/api/models', json(body)),
  updateModel: (id: string, body: unknown) =>
    request<ModelDetail>(`/api/models/${id}`, { ...json(body), method: 'PUT' }),
  deleteModel: (id: string) => request<void>(`/api/models/${id}`, { method: 'DELETE' }),

  // Description revisions work identically for models and bundles.
  updateDescription: (owner: DescOwner, id: string, body_md: string) =>
    request<Revision>(`/api/${owner}/${id}/description`, { ...json({ body_md }), method: 'PUT' }),
  revisions: (owner: DescOwner, id: string) =>
    request<Revision[]>(`/api/${owner}/${id}/description/revisions`),
  labelRevision: (owner: DescOwner, id: string, revId: string, label: string | null) =>
    request<void>(`/api/${owner}/${id}/description/revisions/${revId}/label`, {
      ...json({ label }),
      method: 'PUT',
    }),

  browse: (params: URLSearchParams) => request<BrowseResults>(`/api/browse?${params}`),

  /** Everything the caller has liked, models and bundles mixed, newest first. */
  likes: (params: URLSearchParams) => request<BrowseResults>(`/api/likes?${params}`),
  /** Idempotent: liking twice is one like, so a double click is harmless. */
  setLike: (kind: 'model' | 'bundle', id: string, liked: boolean) =>
    request<void>(`/api/${kind}s/${id}/like`, { method: liked ? 'PUT' : 'DELETE' }),

  searchBundles: (params: URLSearchParams) => request<BundleResults>(`/api/bundles?${params}`),
  bundle: (id: string) => request<BundleDetail>(`/api/bundles/${id}`),
  createBundle: (body: unknown) => request<BundleDetail>('/api/bundles', json(body)),
  updateBundle: (id: string, body: unknown) =>
    request<BundleDetail>(`/api/bundles/${id}`, { ...json(body), method: 'PUT' }),
  deleteBundle: (id: string, members: BundleMemberDisposition = 'keep') =>
    request<void>(`/api/bundles/${id}${members === 'keep' ? '' : `?members=${members}`}`, {
      method: 'DELETE',
    }),
  /** Move some members out into a bundle of their own; returns the new bundle. */
  splitBundle: (id: string, name: string, modelIds: string[]) =>
    request<BundleDetail>(`/api/bundles/${id}/split`, json({ name, model_ids: modelIds })),
  /** Absorb `from` into `id`; returns the bundle that did the absorbing. */
  mergeBundle: (id: string, from: string, other: OtherBundleDisposition) =>
    request<BundleDetail>(`/api/bundles/${id}/merge`, json({ from, other })),
  addModelToBundle: (bundleId: string, modelId: string) =>
    request<void>(`/api/bundles/${bundleId}/models`, json({ model_id: modelId })),
  removeModelFromBundle: (bundleId: string, modelId: string) =>
    request<void>(`/api/bundles/${bundleId}/models/${modelId}`, { method: 'DELETE' }),
  /** Add and/or remove tags across every member model of a bundle in one call.
      Additive/subtractive only — there is deliberately no bulk replace. */
  retagBundleMembers: (id: string, add: string[], remove: string[]) =>
    request<MemberTagsResult>(`/api/bundles/${id}/models/tags`, json({ add, remove })),
  /** replace a bundle's ordered category list (reorder/add/remove in one call) */
  setBundleCategories: (id: string, categories: string[]) =>
    request<BundleDetail>(`/api/bundles/${id}/categories`, {
      ...json({ categories }),
      method: 'PUT',
    }),
  bundleFiles: (id: string) => request<FileRecord[]>(`/api/bundles/${id}/files`),
  uploadBundleFiles: (id: string, form: FormData) =>
    request<FileRecord[]>(`/api/bundles/${id}/files`, { method: 'POST', body: form }),

  imports: () => request<ImportSummary[]>('/api/imports'),
  import: (id: string) => request<ImportSummary>(`/api/imports/${id}`),
  createImport: (name: string) => request<ImportSummary>('/api/imports', json({ name })),
  renameImport: (id: string, name: string) =>
    request<ImportSummary>(`/api/imports/${id}`, { ...json({ name }), method: 'PUT' }),
  deleteImport: (id: string) => request<void>(`/api/imports/${id}`, { method: 'DELETE' }),
  /** lift a staged folder (and everything under it) out into an import of its
      own; the folder becomes the new import's top directory */
  splitImport: (id: string, folder: string, name?: string) =>
    request<ImportSummary>(`/api/imports/${id}/split`, json({ folder, name })),
  /** every staged file, or — with `path` — just the one folder's worth. The
      import page opens folders against the narrow form; the whole listing is
      only worth asking for once a layout needs to annotate all of it. */
  importFiles: (id: string, path?: string) =>
    request<FileRecord[]>(
      path === undefined
        ? `/api/imports/${id}/files`
        : `/api/imports/${id}/files?${new URLSearchParams({ path })}`,
    ),
  /** what is staged so far, counted by folder — cheap enough to poll while an
      import is still filling up, which the full listing is not */
  importFileSummary: (id: string) => request<ImportFolder[]>(`/api/imports/${id}/files/summary`),
  commitImport: (id: string, target: CommitTarget) =>
    request<CommitResult>(`/api/imports/${id}/commit`, json(target)),
  planImport: (id: string, spec: LayoutSpec, target: PlanTarget, bundleId?: string) =>
    request<LayoutPlan>(`/api/imports/${id}/plan`, json({ ...spec, target, bundle_id: bundleId })),
  /** contents of the server-side dropbox (admin only) */
  dropbox: () => request<DropboxListing>('/api/dropbox'),
  /** stage one dropbox entry as an import; the copy itself runs as a job */
  pickUpDropboxEntry: (entry: string) =>
    request<ImportSummary>('/api/dropbox/import', json({ entry })),
  /** delete a dropbox entry off the server's disk (admin only) */
  deleteDropboxEntry: (entry: string) =>
    request<void>(`/api/dropbox?entry=${encodeURIComponent(entry)}`, { method: 'DELETE' }),
  importLayouts: () => request<ImportLayout[]>('/api/import-layouts'),
  createImportLayout: (body: { name: string; creator_id?: string | null } & LayoutSpec) =>
    request<ImportLayout>('/api/import-layouts', json(body)),
  deleteImportLayout: (id: string) =>
    request<void>(`/api/import-layouts/${id}`, { method: 'DELETE' }),

  createVariant: (modelId: string, body: unknown) =>
    request<VariantDetail>(`/api/models/${modelId}/variants`, json(body)),
  updateVariant: (id: string, body: unknown) =>
    request<VariantDetail>(`/api/variants/${id}`, { ...json(body), method: 'PUT' }),
  deleteVariant: (id: string) => request<void>(`/api/variants/${id}`, { method: 'DELETE' }),
  variantFiles: (id: string) => request<FileRecord[]>(`/api/variants/${id}/files`),
  uploadVariantFiles: (id: string, form: FormData) =>
    request<FileRecord[]>(`/api/variants/${id}/files`, { method: 'POST', body: form }),
  modelFiles: (id: string) => request<FileRecord[]>(`/api/models/${id}/files`),
  uploadModelFiles: (id: string, form: FormData) =>
    request<FileRecord[]>(`/api/models/${id}/files`, { method: 'POST', body: form }),
  updateFile: (id: string, body: FileUpdate) =>
    request<FileRecord>(`/api/files/${id}`, { ...json(body), method: 'PATCH' }),
  deleteFile: (id: string) => request<void>(`/api/files/${id}`, { method: 'DELETE' }),

  uploadImage: (owner: 'models' | 'variants' | 'bundles', id: string, form: FormData) =>
    request<ImageRecord>(`/api/${owner}/${id}/images`, { method: 'POST', body: form }),
  markPrimary: (imageId: string) =>
    request<void>(`/api/images/${imageId}/primary`, { method: 'PUT' }),
  /// Favourite a variant's picture *for the model*: the model takes a copy of the
  /// same blob as its own primary, and the variant keeps its thumbnail.
  promoteImage: (modelId: string, imageId: string) =>
    request<void>(`/api/models/${modelId}/images/${imageId}/promote`, { method: 'PUT' }),
  deleteImage: (imageId: string) => request<void>(`/api/images/${imageId}`, { method: 'DELETE' }),
  /// Force a render of this file, whatever the automatic pass chose. Returns the
  /// job, so the caller can wait for *its* picture rather than watching the queue.
  renderFile: (fileId: string) =>
    request<{ job_id: number }>(`/api/files/${fileId}/render`, { method: 'POST' }),
  job: (jobId: number) => request<Job>(`/api/jobs/${jobId}`),

  creators: (q = '') => request<Creator[]>(`/api/creators?q=${encodeURIComponent(q)}`),
  creator: (id: string) => request<Creator>(`/api/creators/${id}`),
  createCreator: (body: unknown) => request<Creator>('/api/creators', json(body)),
  updateCreator: (id: string, body: unknown) =>
    request<Creator>(`/api/creators/${id}`, { ...json(body), method: 'PUT' }),

  tags: (sel: TagFilter = {}) => request<Tag[]>(`/api/tags${tagSelectionQuery(sel)}`),
  variantTags: (sel: TagFilter = {}) =>
    request<VariantTag[]>(`/api/variant-tags${tagSelectionQuery(sel)}`),

  jobs: (status = '') => request<Job[]>(`/api/jobs?status=${status}`),
  retryJob: (id: number) => request<void>(`/api/jobs/${id}/retry`, { method: 'POST' }),

  rendererConfig: () => request<RendererConfig>('/api/admin/settings/renderer'),
  setRendererConfig: (config: RendererConfig) =>
    request<RendererConfig>('/api/admin/settings/renderer', { ...json(config), method: 'PUT' }),
  rerender: (scope: 'stale' | 'all', mode: 'add' | 'replace') =>
    request<{ jobs_queued: number }>('/api/admin/rerender', json({ scope, mode })),
  gcBlobs: (dryRun: boolean) => request<GcReport>('/api/admin/gc', json({ dry_run: dryRun })),
  storage: () => request<StorageReport>('/api/admin/storage'),
  /// Stats every blob in the store — on demand, not on page load.
  storageCompression: () => request<CompressionReport>('/api/admin/storage/compression'),

  /// What the current selection + filters would keep (per-model variant counts,
  /// a variant summary, and file counts by kind). Cheap; called as the dialog
  /// changes.
  exportPreview: (body: ExportRequest) =>
    request<ExportPreview>('/api/exports/preview', json(body)),
  /// Queue building an export archive; returns immediately with a building row.
  createExport: (body: ExportRequest) => request<ExportSummary>('/api/exports', json(body)),
  exports: () => request<ExportSummary[]>('/api/exports'),
  export: (id: string) => request<ExportSummary>(`/api/exports/${id}`),
  deleteExport: (id: string) => request<void>(`/api/exports/${id}`, { method: 'DELETE' }),

  /// What a dropped export archive holds (flagging entities already present).
  /// Reads only the manifest, so it is instant even for a huge archive.
  restorePreview: (importId: string) =>
    request<RestorePreview>(`/api/imports/${importId}/restore/preview`),
  /// Restore a previewed archive. `fresh` names the manifest-local ids of
  /// already-present entities to import as a fresh copy anyway.
  restoreCommit: (
    importId: string,
    fresh: string[],
    custom_fields: Record<string, CustomFieldMapping> = {},
  ) =>
    request<RestoreSummary>(
      `/api/imports/${importId}/restore/commit`,
      json({ fresh, custom_fields }),
    ),

  previewBundlePatch: (bundleId: string, zip: File) => {
    const form = new FormData()
    form.append('file', zip)
    return request<PatchPreview>(`/api/bundles/${bundleId}/patch/preview`, {
      method: 'POST',
      body: form,
    })
  },
  applyBundlePatch: (bundleId: string, zip: File, options: PatchApplyOptions) => {
    const form = new FormData()
    form.append('options', JSON.stringify(options))
    form.append('file', zip)
    return request<PatchApplyResult>(`/api/bundles/${bundleId}/patch`, {
      method: 'POST',
      body: form,
    })
  },
}

// --- Bundle metadata patch --------------------------------------------------

export interface PatchMember {
  id: string
  name: string
  tags: string[]
  /** other names this model already answers to (skip offering a rename to one) */
  aliases: string[]
}

export interface PatchUnresolvedRow {
  /** the patch model's index — its stable identity (names are not unique) */
  key: number
  patch_name: string
  patch_tags: string[]
  has_image: boolean
  has_description: boolean
  category: string | null
  /** non-empty for ambiguous rows; empty means "offer the whole member list" */
  candidates: PatchMember[]
}

export interface PatchPreview {
  /** the bundle description the patch carries (markdown), or null */
  bundle_description: string | null
  /** candidate covers as data: URLs, primary first */
  bundle_covers: string[]
  matched: {
    key: number
    patch_name: string
    model_id: string
    model_name: string
    add_tags: string[]
    has_image: boolean
    has_description: boolean
    category: string | null
  }[]
  ambiguous: PatchUnresolvedRow[]
  unmatched_patch: PatchUnresolvedRow[]
  unmatched_members: string[]
  members: PatchMember[]
  /** custom field values the patch carries that will be written */
  custom_fields_applied: number
  /** ...and the ones that won't, each with why — informational, never fatal */
  custom_field_warnings: PatchCustomFieldWarning[]
}

/** One scraped metadata key the apply will skip, and why. */
export interface PatchCustomFieldWarning {
  /** "the bundle", or the patch model's label */
  source: string
  key: string
  reason: string
}

export interface PatchApplyOptions {
  /** patch model labels to rename to the scraped name (per-model, not global) */
  rename: string[]
  model_tags: 'merge' | 'replace' | 'skip'
  model_images: 'replace_generated' | 'add' | 'skip'
  /** apply each patch model's description as a new revision */
  model_descriptions: boolean
  bundle_cover: boolean
  bundle_description: boolean
  /** patch model label -> chosen member id; resolves ambiguous / adopts unmatched */
  matches: Record<string, string>
}

export interface PatchApplyResult {
  models_updated: number
  images_added: number
  tags_added: number
  aliases_added: number
  descriptions_added: number
  /** custom field values written, bundle and members together */
  custom_fields_set: number
}

export const imageUrl = (id: string) => `/api/images/${id}`
export const downloadUrl = (fileId: string) => `/api/files/${fileId}/download`
/// An f3d-rendered PNG still of a single file, rendered on demand and not
/// persisted. The STL viewer shows this first for large meshes, before the user
/// opts into the heavier interactive preview.
export const renderPreviewUrl = (fileId: string) => `/api/files/${fileId}/render/preview`

// --- Export / import archives ----------------------------------------------

/// A finished export is downloaded by navigating to this URL (a GET that streams
/// the zip with a Content-Disposition attachment), rather than through fetch.
export const exportDownloadUrl = (id: string) => `/api/exports/${id}/download`

/// What to build. `model_ids` is the selected set; `variant_exclude` carries the
/// negative variant-tag filters (e.g. `["supported"]` = unsupported only).
export interface ExportRequest {
  name?: string
  bundle_id?: string
  model_ids: string[]
  variant_include?: string[]
  variant_exclude?: string[]
  /** file kinds to drop (e.g. ["project", "archive"]); empty keeps all */
  file_kinds_exclude?: string[]
}

/// What the current selection + filters would keep, for the export dialog.
export interface ExportPreview {
  models: { id: string; name: string; variants_total: number; variants_kept: number }[]
  variants: { label: string; count: number; kept: boolean }[]
  file_kinds: { kind: string; count: number }[]
}

export interface ExportSummary {
  id: string
  name: string
  /** building | ready | failed */
  status: string
  model_count: number
  size: number | null
  filename: string | null
  error: string | null
  /** absolute path of the built artifact in the store — sent only to admins, and
      only for a ready export (see routes/exports.rs) */
  path: string | null
  created_at: string
  updated_at: string
}

/// One model or bundle inside a dropped export archive.
export interface RestoreEntity {
  /** manifest-local id — pass to `fresh` to force a fresh copy of an existing one */
  id: string
  name: string
  slug: string
  /** an entity with this slug already exists here (skipped unless fresh-copied) */
  exists: boolean
  /** member count, for bundles */
  members?: number
  /** custom field values the archive carries for it — not written if it is
      skipped, since a skipped entity is left untouched */
  custom_field_values: number
}

/// A custom field definition the archive carries. The vocabulary is an
/// instance-wide admin setting, so the receiving instance may know nothing about
/// it — hence the per-field choice on import.
export interface RestoreCustomField {
  /** manifest-local id — the key of the `custom_fields` mapping on commit */
  id: string
  key: string
  name: string
  kind: CustomFieldKind
  applies_to_models: boolean
  applies_to_bundles: boolean
  visibility: CustomFieldVisibility
  /** how many values across the archive would be written under it */
  value_count: number
  /** the local field this instance would adopt by default, or null = create it */
  suggested_field_id: string | null
}

/** This instance's own vocabulary, to choose from when mapping. */
export interface RestoreLocalField {
  id: string
  key: string
  name: string
  kind: CustomFieldKind
  applies_to_models: boolean
  applies_to_bundles: boolean
}

/// What to do with one exported custom field: drop it, write its values onto a
/// field already here, or add the archive's definition to the vocabulary.
export type CustomFieldMapping =
  { action: 'skip' } | { action: 'existing'; field_id: string } | { action: 'create' }

export interface RestorePreview {
  schema: string
  exported_at: string
  models: RestoreEntity[]
  bundles: RestoreEntity[]
  custom_fields: RestoreCustomField[]
  local_custom_fields: RestoreLocalField[]
  blob_count: number
  total_size: number
}

export interface RestoreSummary {
  models_created: number
  models_skipped: number
  bundles_created: number
  bundles_skipped: number
  files: number
  images: number
  blobs: number
  /** custom field definitions added to this instance's vocabulary */
  custom_fields_created: number
  /** values written under them (and under the fields they were mapped onto) */
  custom_field_values: number
}

/// A short, human label for a source URL: just its origin (`https://host`), so
/// "from https://www.myminifactory.com" reads next to the creator rather than
/// dumping the whole object path. Falls back to the raw string if it won't parse.
export function sourceOrigin(url: string): string {
  try {
    return new URL(url).origin
  } catch {
    return url
  }
}

/// How to refer to a variant in prose: its label if it has one, else its tags,
/// else the fact that it is the model's default (tagless) bucket of files.
export function variantLabel(variant: VariantDetail): string {
  if (variant.name) return variant.name
  if (variant.tags.length) return variant.tags.join(' + ')
  return 'Default'
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`
  const units = ['KB', 'MB', 'GB', 'TB']
  let value = bytes
  let unit = ''
  for (const u of units) {
    value /= 1024
    unit = u
    if (value < 1024) break
  }
  return `${value.toFixed(value < 10 ? 1 : 0)} ${unit}`
}
