//! Import layout templates: a list of regex rules whose capture groups carve a
//! staged import into models and variants (docs/plan.md, "Import layout
//! templates").
//!
//! A layout is several small patterns rather than one big one. Each is
//! *searched* across the file's path, contributes its own captures through its
//! own roles and value map, and the results merge — model tags and variant tags
//! union, and one designated group names the model. One regex that has to
//! capture the name, the category, the scale and the support state at once is
//! fragile to write and impossible to reuse; a rule that only knows about
//! scales composes with a rule that only knows about supports.
//!
//! The whole feature hangs off one pure function, [`analyze`]: given a spec, a
//! file list, and the variant-tag vocabulary, it produces both the preview the
//! UI shows (per-file highlight spans + resolved chips, the grouped tree) and
//! the carve the commit executes (file ids per model/variant). One function
//! feeding both is what guarantees preview == result.
//!
//! The dialect is fancy-regex (lookaround and backreferences allowed) and the
//! backend is its only interpreter — the frontend treats patterns as opaque
//! strings and renders the `parts` this module computes, so there is no
//! Rust-vs-JS regex drift and no byte-vs-UTF-16 offset mismatch.

use std::collections::{BTreeMap, HashMap, HashSet};

use fancy_regex::Regex;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;
use uuid::Uuid;

use crate::error::ApiError;

/// What a capture group means. There is deliberately no "variant" role: a
/// variant IS its tag set, so the union of a file's variant-tag captures
/// (after value mapping) simply is its variant.
#[derive(Clone, Copy, PartialEq, Eq, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    ModelName,
    ModelTag,
    VariantTag,
    Ignore,
}

fn default_true() -> bool {
    true
}

/// One regex of a layout: a pattern, what each of *its* groups means, and how
/// its raw captures translate into variant tags. A group number only ever means
/// something inside its own rule, so two rules can both use group 1 for
/// different things and carry different value maps.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct LayoutRule {
    /// An optional label, shown on the rule's editor block ("scale", "supports").
    #[serde(default)]
    pub name: String,
    /// fancy-regex, *searched* across the file's full logical path
    /// (`path/filename`) — not anchored, so a small rule can find its fragment
    /// wherever it sits, and in more than one place.
    pub pattern: String,
    /// Capture group number (a string, because JSON keys) -> role.
    /// Unlisted groups are ignored.
    #[serde(default)]
    pub roles: HashMap<String, Role>,
    /// Lowercased raw capture -> variant tag names ("supported_lychee" ->
    /// supported + lychee_project). An empty list means "recognised, maps to
    /// no tags". Values with no entry fall back to an existing variant tag of
    /// the same name, else count as unmapped.
    #[serde(default)]
    pub value_map: HashMap<String, Vec<String>>,
    /// Off = contributes nothing, exactly like a rule that never matches. The
    /// panel's rule list is per-import working state, so a toggle scopes to this
    /// import; saving the layout captures the toggles as the template's defaults.
    #[serde(default = "default_true")]
    pub enabled: bool,
}

/// The user-editable definition: a list of rules whose captures merge. One
/// regex that has to capture the name, the category, the scale *and* the
/// support state at once is fragile to write; several small ones, each saying
/// one thing, compose.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct LayoutSpec {
    pub rules: Vec<LayoutRule>,
    /// Drop the folders on the way in: the carved files land in the model with
    /// no `path` at all. Once a tree has been carved, its folders have usually
    /// *said* everything they had to say — `32mm/supported/` becomes the variant,
    /// and repeating it as a folder inside that variant only buries the files.
    #[serde(default)]
    pub flatten: bool,
}

/// Whether the carve targets one model (variants only; model-name captures
/// just suggest the name) or a bundle (model-name captures split member
/// models).
#[derive(Clone, Copy, PartialEq, Eq, Debug, Default, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum CarveTarget {
    Model,
    #[default]
    Bundle,
}

/// A file to be planned. `path`/`filename` as stored on the `files` row.
pub struct PlanFile {
    pub id: Uuid,
    pub path: String,
    pub filename: String,
}

impl PlanFile {
    fn full_path(&self) -> String {
        if self.path.is_empty() {
            self.filename.clone()
        } else {
            format!("{}/{}", self.path, self.filename)
        }
    }
}

/// A slice of a file's path: plain text, or the text a capture group matched.
/// The *role* travels with the slice rather than a group number, because with
/// several rules in play a group number is no longer unique across the layout.
#[derive(Serialize, ToSchema)]
pub struct PathPart {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub role: Option<Role>,
}

/// Everything the UI needs to draw one file row under the active layout.
#[derive(Serialize, ToSchema)]
pub struct FileAnnotation {
    pub id: Uuid,
    /// Matched by at least one enabled rule.
    pub matched: bool,
    pub parts: Vec<PathPart>,
    /// Indices of rules that contradicted themselves on this file — one of their
    /// groups captured two different values here, so there is no single answer
    /// to take. Their output is dropped and the file is flagged; the other rules
    /// still apply and the commit is *not* blocked.
    pub invalid_rules: Vec<usize>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    pub model_tags: Vec<String>,
    pub variant_tags: Vec<String>,
    /// Raw variant-tag captures with no resolution — the mapping table's todo.
    pub unmapped: Vec<String>,
}

/// One planned variant of a planned model. Empty `tags` = matched files that
/// resolved no variant tags; the commit puts them in the model's unsorted
/// bucket (one-model carve) or its anonymous variant (bundle carve).
#[derive(Serialize, ToSchema)]
pub struct PlanVariant {
    pub tags: Vec<String>,
    pub file_count: usize,
    /// One example path, to anchor the preview.
    pub example: String,
    #[serde(skip)]
    pub files: Vec<Uuid>,
}

/// One planned (member) model with its variants.
#[derive(Serialize, ToSchema)]
pub struct PlanModel {
    /// Empty under [`CarveTarget::Model`] when no single name was captured.
    pub name: String,
    pub tags: Vec<String>,
    pub file_count: usize,
    pub variants: Vec<PlanVariant>,
    /// Carving into an *existing* bundle: the member model this planned model
    /// would merge onto by default (null = a new member is created). `analyze`
    /// never sets this — it has no DB; the plan endpoint fills it in.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub merge_target: Option<Uuid>,
}

/// A member model of the bundle being merged into, offered as a retarget option
/// for each planned model. Filled by the plan endpoint (see imports.rs).
#[derive(Serialize, ToSchema, Clone)]
pub struct MemberCandidate {
    pub id: Uuid,
    pub name: String,
    pub tags: Vec<String>,
}

/// A capture group, with example captures for the role-assignment table.
#[derive(Serialize, ToSchema)]
pub struct GroupInfo {
    pub index: u32,
    /// Up to three distinct example captures.
    pub examples: Vec<String>,
}

/// A distinct raw value captured by a variant-tag group, and what it resolved
/// to. `tags: null` = unmapped (prompts the mapping table).
#[derive(Serialize, ToSchema)]
pub struct CapturedValue {
    /// The captured value, humanised (underscores and camelCase turned into
    /// spaces). The same normalisation feeds its identity, so `Supported_LYCHEE`,
    /// `Supported LYCHEE` and `SupportedLychee` are one value shown once; the
    /// value-map key is its lowercase.
    pub raw: String,
    pub tags: Option<Vec<String>>,
}

/// What one rule found, for its own editor block: the groups it captures (with
/// examples) and the distinct raw values its variant-tag groups produced. Both
/// are per-rule because both are read through that rule's own roles/value map.
#[derive(Serialize, ToSchema)]
pub struct RulePlan {
    pub groups: Vec<GroupInfo>,
    pub values: Vec<CapturedValue>,
}

/// The dry-run result: preview for the UI, carve for the commit.
#[derive(Serialize, ToSchema)]
pub struct Plan {
    pub total: usize,
    pub matched: usize,
    /// Files that actually land on a model/variant (a matched file without a
    /// model name still falls to unsorted under a bundle target).
    pub carved: usize,
    /// Index-aligned to the spec's rules.
    pub rules: Vec<RulePlan>,
    pub models: Vec<PlanModel>,
    /// Distinct model names captured — >1 under a one-model target is the
    /// "this is really a bundle" signal.
    pub model_names: Vec<String>,
    pub annotations: Vec<FileAnnotation>,
    /// Existing members of the bundle being merged into, so the UI can offer
    /// each planned model a retarget dropdown. Empty unless the plan endpoint
    /// is carving into an existing bundle.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub members: Vec<MemberCandidate>,
    /// Distinct model-tag captures in the order they first appear in the tree
    /// (files arrive in path order), i.e. the bundle's section/category order as
    /// the folders present it — `1 - Heroes` before `2 - Enemies`. The carve
    /// records these as the bundle's ordered categories.
    #[serde(skip_serializing_if = "Vec::is_empty")]
    pub model_tag_order: Vec<String>,
}

impl Plan {
    /// Raw values the commit must refuse on: mapping them is the user's call,
    /// not something to guess at.
    pub fn unmapped_values(&self) -> Vec<&str> {
        let mut out: Vec<&str> = Vec::new();
        for value in self.rules.iter().flat_map(|r| r.values.iter()) {
            // Two rules can capture the same raw value; the user only needs
            // telling about it once.
            if value.tags.is_none() && !out.contains(&value.raw.as_str()) {
                out.push(value.raw.as_str());
            }
        }
        out
    }
}

fn fold(s: &str) -> String {
    s.trim().to_lowercase()
}

/// Identity of a captured variant-tag value. Humanise first, so the folder
/// spelled `Supported_LYCHEE`, `Supported LYCHEE` and `SupportedLychee` all
/// name one value — one row in the mapping table, one value-map key.
fn value_key(raw: &str) -> String {
    fold(&crate::util::humanize_token(raw))
}

/// Collapse a value map onto canonical keys so two spellings of one value can't
/// both survive. The UI keys an edit by the humanised value (`supported lychee`),
/// but a template saved earlier may still carry the legacy spelling
/// (`supported_lychee`); both fold to one [`value_key`], and left as-is which one
/// wins is down to hash order — so a tag the user *removed* can quietly come back
/// through the stale key. Canonicalise, and let the already-canonical spelling
/// (the UI's edit) win over a legacy one, so a removal sticks.
pub fn canonical_value_map(map: &HashMap<String, Vec<String>>) -> BTreeMap<String, Vec<String>> {
    let mut out: BTreeMap<String, Vec<String>> = BTreeMap::new();
    // Legacy spellings first, then canonical keys overwrite them.
    for (k, v) in map {
        if value_key(k) != *k {
            out.insert(value_key(k), v.clone());
        }
    }
    for (k, v) in map {
        if value_key(k) == *k {
            out.insert(k.clone(), v.clone());
        }
    }
    out
}

/// What one capture group of one rule found in one file: the folded value (its
/// identity — two spellings of it are the same answer), the raw text as
/// captured, and every span it matched.
type GroupCapture = (String, String, Vec<(usize, usize)>);

/// One rule, compiled and ready to run over paths.
struct CompiledRule {
    regex: Regex,
    /// includes group 0
    group_count: usize,
    roles: Vec<Role>,
    value_map: BTreeMap<String, Vec<String>>,
    enabled: bool,
}

/// What a rule's variant-tag capture resolves to: its explicit mapping, else an
/// existing variant tag of the same name, else nothing (unmapped).
fn resolve(
    value_map: &BTreeMap<String, Vec<String>>,
    vocab: &HashSet<String>,
    raw: &str,
) -> Option<Vec<String>> {
    if let Some(tags) = value_map.get(&value_key(raw)) {
        return Some(
            tags.iter()
                .map(|t| fold(t))
                .filter(|t| !t.is_empty())
                .collect(),
        );
    }
    // The vocabulary is matched on the exact fold, not the humanised form: a tag
    // whose name legitimately carries an underscore (lychee_project) must still
    // resolve when a capture names it outright.
    let folded = fold(raw);
    vocab.contains(&folded).then(|| vec![folded])
}

/// Run a layout over a file list. Pure — `vocab` is the lowercased variant-tag
/// vocabulary, fetched by the caller — so the same call backs the plan
/// endpoint, the commit, and the tests.
///
/// Every enabled rule is searched across each path independently and their
/// captures merge: model tags and variant tags union, and the one rule allowed
/// to carry the model name names the model. A rule that finds nothing simply
/// says nothing.
pub fn analyze(
    spec: &LayoutSpec,
    target: CarveTarget,
    files: &[PlanFile],
    vocab: &HashSet<String>,
) -> Result<Plan, ApiError> {
    let mut compiled: Vec<CompiledRule> = Vec::with_capacity(spec.rules.len());
    let mut name_groups = 0usize;
    for rule in &spec.rules {
        // Searched, not anchored — a small rule finds its fragment wherever it
        // sits. The seeded patterns anchor themselves (`^…$`), so they still
        // match exactly once and carve identically.
        let regex = Regex::new(&rule.pattern)
            .map_err(|e| ApiError::BadRequest(format!("bad pattern: {e}")))?;
        let group_count = regex.captures_len(); // includes group 0

        let mut roles: Vec<Role> = vec![Role::Ignore; group_count];
        for (key, role) in &rule.roles {
            // A saved layout carries a role per group it was built for, but an
            // in-progress edit may have cut the pattern down to fewer groups. Skip a
            // role whose group the current pattern no longer has — the carve simply
            // uses the groups that exist — rather than failing the whole plan, which
            // would drop the preview back to the plain file tree the moment you delete
            // a group. (The panel keeps those stale assignments, greyed out, and they
            // reactivate when the group returns.)
            if let Some(index) = key
                .parse::<usize>()
                .ok()
                .filter(|i| (1..group_count).contains(i))
            {
                roles[index] = *role;
            }
        }
        name_groups += roles.iter().filter(|r| **r == Role::ModelName).count();

        compiled.push(CompiledRule {
            regex,
            group_count,
            roles,
            // Value-map keys are humanised the same way the captures are, so a saved
            // "supported_lychee" still matches the folder that spelled it any which way —
            // and a legacy spelling can't shadow the UI's re-mapping of the same value.
            value_map: canonical_value_map(&rule.value_map),
            // A blank pattern is a rule the user has only just added, not one
            // that matches everywhere: inert, exactly like a disabled one, so
            // pressing "Add rule" doesn't flood the preview before you type.
            enabled: rule.enabled && !rule.pattern.trim().is_empty(),
        });
    }
    // The model name is singular across the whole layout: two rules each
    // claiming it would leave the carve picking one by rule order.
    if name_groups > 1 {
        return Err(ApiError::BadRequest(
            "at most one capture group, in one rule, can be the model name".into(),
        ));
    }

    let mut annotations = Vec::with_capacity(files.len());
    // Per-rule preview state: the groups each pattern captures (examples filled
    // in as files match) and the distinct raw values its variant-tag groups saw.
    let mut rule_groups: Vec<Vec<GroupInfo>> = compiled
        .iter()
        .map(|rule| {
            (1..rule.group_count)
                .map(|i| GroupInfo {
                    index: i as u32,
                    examples: Vec::new(),
                })
                .collect()
        })
        .collect();
    let mut rule_values: Vec<BTreeMap<String, CapturedValue>> =
        compiled.iter().map(|_| BTreeMap::new()).collect();
    // Model tags in first-seen (file/path) order — the bundle's category order.
    let mut model_tag_order: Vec<String> = Vec::new();
    let mut seen_model_tags: HashSet<String> = HashSet::new();
    // (folded name, folded tag set) -> model; BTreeMaps keep the output stable.
    type VariantKey = Vec<String>;
    struct ModelAcc {
        name: String,
        tags: Vec<String>,
        variants: BTreeMap<VariantKey, PlanVariant>,
    }
    let mut models: BTreeMap<(String, Vec<String>), ModelAcc> = BTreeMap::new();
    let mut model_names: BTreeMap<String, String> = BTreeMap::new();
    let mut matched = 0usize;
    let mut carved = 0usize;

    for file in files {
        let full = file.full_path();
        // Merged across every rule that had something to say about this file.
        let mut matched_any = false;
        let mut invalid_rules: Vec<usize> = Vec::new();
        let mut spans: Vec<(usize, usize, Role)> = Vec::new();
        let mut model_name: Option<String> = None;
        let mut model_tags: Vec<String> = Vec::new();
        let mut variant_tags: Vec<String> = Vec::new();
        let mut unmapped: Vec<String> = Vec::new();

        for (rule_index, rule) in compiled.iter().enumerate() {
            if !rule.enabled {
                continue;
            }
            // Group index -> the group's one answer. A rule is searched, so one
            // group can fire several times down the path; consistency is what
            // makes that a single answer.
            let mut captured: BTreeMap<usize, GroupCapture> = BTreeMap::new();
            let mut contradicted = false;
            let mut hit = false;
            for caps in rule.regex.captures_iter(&full) {
                let caps =
                    caps.map_err(|e| ApiError::BadRequest(format!("pattern failed to run: {e}")))?;
                hit = true;
                for index in 1..rule.group_count {
                    let Some(m) = caps.get(index) else { continue };
                    let raw = m.as_str().trim();
                    if raw.is_empty() {
                        continue;
                    }
                    // Examples are collected even when the rule turns out to
                    // contradict itself — seeing both values side by side in the
                    // role table is how you spot why it was dropped.
                    if let Some(info) = rule_groups[rule_index].get_mut(index - 1)
                        && !info.examples.iter().any(|e| e == raw)
                        && info.examples.len() < 3
                    {
                        info.examples.push(raw.to_string());
                    }
                    let folded = fold(raw);
                    match captured.entry(index) {
                        std::collections::btree_map::Entry::Vacant(slot) => {
                            slot.insert((folded, raw.to_string(), vec![(m.start(), m.end())]));
                        }
                        std::collections::btree_map::Entry::Occupied(mut slot) => {
                            let seen = slot.get_mut();
                            if seen.0 == folded {
                                seen.2.push((m.start(), m.end()));
                            } else {
                                // 32mm here and 75mm there, from one group: this
                                // rule has no single answer for this file.
                                contradicted = true;
                            }
                        }
                    }
                }
            }
            if !hit {
                continue; // a rule that doesn't match just says nothing
            }
            matched_any = true;
            if contradicted {
                // Drop this rule's whole contribution (and its highlights) for
                // this file, warn, and let the other rules carry on.
                invalid_rules.push(rule_index);
                continue;
            }

            for (index, (_folded, raw, group_spans)) in &captured {
                let role = rule.roles[*index];
                if role != Role::Ignore {
                    spans.extend(
                        group_spans
                            .iter()
                            .filter(|(s, e)| s < e)
                            .map(|(s, e)| (*s, *e, role)),
                    );
                }
                match role {
                    // Archives name their folders in camel case constantly, and the
                    // capture becomes the model's name verbatim — so `DwarfBerserker`
                    // would be the name in the library. Put the spaces back.
                    Role::ModelName => model_name = Some(crate::util::expand_camel_case(raw)),
                    Role::ModelTag => {
                        if !model_tags.iter().any(|t| fold(t) == fold(raw)) {
                            model_tags.push(raw.clone());
                        }
                        if seen_model_tags.insert(fold(raw)) {
                            model_tag_order.push(raw.clone());
                        }
                    }
                    Role::VariantTag => {
                        let resolved = resolve(&rule.value_map, vocab, raw);
                        let humanised = crate::util::humanize_token(raw);
                        rule_values[rule_index]
                            .entry(value_key(raw))
                            .or_insert_with(|| CapturedValue {
                                raw: humanised.clone(),
                                tags: resolved.clone(),
                            });
                        match resolved {
                            Some(tags) => variant_tags.extend(tags),
                            None => {
                                if !unmapped.contains(&humanised) {
                                    unmapped.push(humanised);
                                }
                            }
                        }
                    }
                    Role::Ignore => {}
                }
            }
        }

        variant_tags.sort_unstable();
        variant_tags.dedup();

        // Highlight spans: non-overlapping capture ranges pooled from every valid
        // rule, first-come wins on nesting so the UI never has to deal with
        // overlapping marks.
        spans.sort_by_key(|(start, end, _)| (*start, *end));
        let mut parts = Vec::new();
        let mut cursor = 0usize;
        for (start, end, role) in spans {
            if start < cursor {
                continue;
            }
            if start > cursor {
                parts.push(PathPart {
                    text: full[cursor..start].into(),
                    role: None,
                });
            }
            parts.push(PathPart {
                text: full[start..end].into(),
                role: Some(role),
            });
            cursor = end;
        }
        if cursor < full.len() || parts.is_empty() {
            parts.push(PathPart {
                text: full[cursor..].into(),
                role: None,
            });
        }

        if !matched_any {
            annotations.push(FileAnnotation {
                id: file.id,
                matched: false,
                parts,
                invalid_rules,
                model_name: None,
                model_tags: vec![],
                variant_tags: vec![],
                unmapped: vec![],
            });
            continue;
        }
        matched += 1;

        if let Some(name) = &model_name {
            model_names
                .entry(fold(name))
                .or_insert_with(|| name.clone());
        }

        // Group into the carve tree. Under a one-model target everything
        // matched lands on the single model; under a bundle target a file
        // needs a model name to be carvable.
        let key = match target {
            CarveTarget::Model => Some((String::new(), Vec::new())),
            CarveTarget::Bundle => model_name
                .as_ref()
                .map(|n| (fold(n), model_tags.iter().map(|t| fold(t)).collect())),
        };
        if let Some(key) = key {
            carved += 1;
            let acc = models.entry(key).or_insert_with(|| ModelAcc {
                name: match target {
                    CarveTarget::Model => String::new(),
                    CarveTarget::Bundle => model_name.clone().unwrap_or_default(),
                },
                tags: Vec::new(),
                variants: BTreeMap::new(),
            });
            for tag in &model_tags {
                if !acc.tags.iter().any(|t| fold(t) == fold(tag)) {
                    acc.tags.push(tag.clone());
                }
            }
            let variant = acc
                .variants
                .entry(variant_tags.clone())
                .or_insert_with(|| PlanVariant {
                    tags: variant_tags.clone(),
                    file_count: 0,
                    example: full.clone(),
                    files: Vec::new(),
                });
            variant.file_count += 1;
            variant.files.push(file.id);
        }

        annotations.push(FileAnnotation {
            id: file.id,
            matched: true,
            parts,
            invalid_rules,
            model_name,
            model_tags,
            variant_tags,
            unmapped,
        });
    }

    // Under a one-model target, a single captured name is the name suggestion.
    let mut models: Vec<PlanModel> = models
        .into_values()
        .map(|acc| PlanModel {
            file_count: acc.variants.values().map(|v| v.file_count).sum(),
            name: acc.name,
            tags: acc.tags,
            variants: acc.variants.into_values().collect(),
            merge_target: None,
        })
        .collect();
    if target == CarveTarget::Model
        && model_names.len() == 1
        && let Some(model) = models.first_mut()
    {
        model.name = model_names.values().next().expect("just checked").clone();
    }

    Ok(Plan {
        total: files.len(),
        matched,
        carved,
        // A disabled rule still reports its groups (they come from the pattern,
        // not from the files), so its role table stays editable while it's off —
        // it just captured nothing.
        rules: rule_groups
            .into_iter()
            .zip(rule_values)
            .map(|(groups, values)| RulePlan {
                groups,
                values: values.into_values().collect(),
            })
            .collect(),
        models,
        model_names: model_names.into_values().collect(),
        annotations,
        members: Vec::new(),
        model_tag_order,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOOT_PATTERN: &str =
        r"(?i)^(?:[^/]+/)*?\d+ - ([^/]+)/[^/]+/([^/]+?)_(\d+mm)_([^/]+)/[^/]+\.stl$";

    /// A rule with no value map — the common shape for the small composable
    /// rules these tests are about.
    fn rule(pattern: &str, roles: &[(&str, Role)]) -> LayoutRule {
        LayoutRule {
            name: String::new(),
            pattern: pattern.into(),
            roles: roles.iter().map(|(k, r)| ((*k).to_string(), *r)).collect(),
            value_map: HashMap::new(),
            enabled: true,
        }
    }

    fn spec_of(rules: Vec<LayoutRule>) -> LayoutSpec {
        LayoutSpec {
            rules,
            flatten: false,
        }
    }

    fn loot_spec() -> LayoutSpec {
        spec_of(vec![LayoutRule {
            name: "loot".into(),
            pattern: LOOT_PATTERN.into(),
            roles: HashMap::from([
                ("1".into(), Role::ModelTag),
                ("2".into(), Role::ModelName),
                ("3".into(), Role::VariantTag),
                ("4".into(), Role::VariantTag),
            ]),
            value_map: HashMap::from([
                (
                    "supported_lychee".into(),
                    vec!["supported".into(), "lychee_project".into()],
                ),
                // Keyed the way the folder spells it: humanising splits the
                // camelCase capture, so a "nosupports" key would never match.
                ("NoSupports".into(), vec!["unsupported".into()]),
            ]),
            enabled: true,
        }])
    }

    fn vocab() -> HashSet<String> {
        ["32mm", "75mm", "supported", "unsupported", "lychee_project"]
            .into_iter()
            .map(String::from)
            .collect()
    }

    fn file(id: u128, path: &str, filename: &str) -> PlanFile {
        PlanFile {
            id: Uuid::from_u128(id),
            path: path.into(),
            filename: filename.into(),
        }
    }

    fn loot_files() -> Vec<PlanFile> {
        vec![
            file(
                1,
                "DownloadAll_32mm/1 - Heroes/BuriedTombHeroes_32mm_Supported_LYCHEE/Gold_32mm_Supported_Lychee",
                "Gold.stl",
            ),
            file(
                2,
                "DownloadAll_32mm/1 - Heroes/BuriedTombHeroes_32mm_Supported_LYCHEE/Sanjay_32mm_Supported_Lychee",
                "Sanjay.stl",
            ),
            file(
                3,
                "DownloadAll_32mm/1 - Heroes/BuriedTombHeroes_32mm_NoSupports/Gold_32mm_NoSupports",
                "Gold.stl",
            ),
            file(4, "DownloadAll_32mm", "Magazine.pdf"),
        ]
    }

    #[test]
    fn loot_carve_groups_models_and_variants() {
        let plan = analyze(&loot_spec(), CarveTarget::Bundle, &loot_files(), &vocab()).unwrap();
        assert_eq!((plan.total, plan.matched, plan.carved), (4, 3, 3));

        // Gold appears in two support states: one model, two variants.
        let gold = plan.models.iter().find(|m| m.name == "Gold").expect("gold");
        assert_eq!(gold.tags, vec!["Heroes"]);
        let mut sets: Vec<Vec<String>> = gold.variants.iter().map(|v| v.tags.clone()).collect();
        sets.sort();
        assert_eq!(
            sets,
            vec![
                vec![
                    "32mm".to_string(),
                    "lychee_project".into(),
                    "supported".into()
                ],
                vec!["32mm".to_string(), "unsupported".into()],
            ]
        );
        assert!(plan.models.iter().any(|m| m.name == "Sanjay"));
        assert!(plan.unmapped_values().is_empty());
    }

    #[test]
    fn separate_rules_union_onto_one_variant() {
        // The point of the whole feature: one rule finds the scale, another the
        // support state, and the file lands on the variant that is both.
        let spec = spec_of(vec![
            rule(r"/(\d+mm)/", &[("1", Role::VariantTag)]),
            rule(r"/(supported|unsupported)/", &[("1", Role::VariantTag)]),
        ]);
        let files = vec![file(1, "Gold/32mm/supported", "a.stl")];
        let plan = analyze(&spec, CarveTarget::Model, &files, &vocab()).unwrap();
        assert_eq!(plan.matched, 1);
        assert_eq!(plan.models[0].variants.len(), 1);
        assert_eq!(plan.models[0].variants[0].tags, vec!["32mm", "supported"]);
        // Each rule reports its own captures for its own editor block.
        assert_eq!(plan.rules.len(), 2);
        assert_eq!(plan.rules[0].groups[0].examples, vec!["32mm"]);
        assert_eq!(plan.rules[1].groups[0].examples, vec!["supported"]);
    }

    #[test]
    fn a_rule_that_contradicts_itself_is_dropped_for_that_file_only() {
        // The scale rule finds 32mm in one folder and 75mm in the next: there is
        // no single answer, so that rule contributes nothing here and the file is
        // flagged. The name rule is unaffected, and nothing blocks.
        let spec = spec_of(vec![
            rule(r"(\d+mm)", &[("1", Role::VariantTag)]),
            rule(r"^([^/]+?)_", &[("1", Role::ModelName)]),
        ]);
        let files = vec![file(1, "Gold_32mm/extra_75mm", "a.stl")];
        let plan = analyze(&spec, CarveTarget::Bundle, &files, &vocab()).unwrap();
        let annotation = &plan.annotations[0];
        assert!(
            annotation.matched,
            "the file did match — it just has a bad rule"
        );
        assert_eq!(annotation.invalid_rules, vec![0]);
        assert!(
            annotation.variant_tags.is_empty(),
            "{:?}",
            annotation.variant_tags
        );
        assert_eq!(annotation.model_name.as_deref(), Some("Gold"));
        assert!(
            plan.unmapped_values().is_empty(),
            "a dropped rule never blocks the commit"
        );
        // Its highlight spans go too — only the name is marked.
        let marked: Vec<Role> = annotation.parts.iter().filter_map(|p| p.role).collect();
        assert_eq!(marked, vec![Role::ModelName]);
    }

    #[test]
    fn one_value_found_twice_is_one_tag_not_a_contradiction() {
        let spec = spec_of(vec![rule(r"(\d+mm)", &[("1", Role::VariantTag)])]);
        let files = vec![file(1, "Set_32mm/Gold_32mm", "a.stl")];
        let plan = analyze(&spec, CarveTarget::Model, &files, &vocab()).unwrap();
        assert!(plan.annotations[0].invalid_rules.is_empty());
        assert_eq!(plan.annotations[0].variant_tags, vec!["32mm"]);
        // Both occurrences still highlight.
        let marked: Vec<&str> = plan.annotations[0]
            .parts
            .iter()
            .filter(|p| p.role == Some(Role::VariantTag))
            .map(|p| p.text.as_str())
            .collect();
        assert_eq!(marked, vec!["32mm", "32mm"]);
    }

    #[test]
    fn a_disabled_rule_contributes_nothing() {
        let mut spec = spec_of(vec![
            rule(r"/(\d+mm)/", &[("1", Role::VariantTag)]),
            rule(r"/(supported|unsupported)/", &[("1", Role::VariantTag)]),
        ]);
        spec.rules[1].enabled = false;
        let files = vec![file(1, "Gold/32mm/supported", "a.stl")];
        let plan = analyze(&spec, CarveTarget::Model, &files, &vocab()).unwrap();
        assert_eq!(plan.models[0].variants[0].tags, vec!["32mm"]);
        // Its groups still list (so its role table stays editable) but it saw
        // nothing, so it neither highlights nor captures a value.
        assert_eq!(plan.rules[1].groups.len(), 1);
        assert!(plan.rules[1].groups[0].examples.is_empty());
        assert!(plan.rules[1].values.is_empty());
        let marked: Vec<&str> = plan.annotations[0]
            .parts
            .iter()
            .filter(|p| p.role.is_some())
            .map(|p| p.text.as_str())
            .collect();
        assert_eq!(marked, vec!["32mm"]);
    }

    #[test]
    fn each_rule_resolves_through_its_own_value_map() {
        let mut spec = spec_of(vec![
            rule(r"/(\d+mm)/", &[("1", Role::VariantTag)]),
            rule(r"/(lychee)/", &[("1", Role::VariantTag)]),
        ]);
        spec.rules[1].value_map = HashMap::from([(
            "lychee".into(),
            vec!["supported".into(), "lychee_project".into()],
        )]);
        let files = vec![file(1, "Gold/32mm/lychee", "a.stl")];
        let plan = analyze(&spec, CarveTarget::Model, &files, &vocab()).unwrap();
        assert_eq!(
            plan.annotations[0].variant_tags,
            vec!["32mm", "lychee_project", "supported"]
        );
        // The mapping row belongs to the rule that captured it, not the layout.
        assert!(plan.rules[0].values.iter().all(|v| v.raw != "Lychee"));
        assert_eq!(plan.rules[1].values.len(), 1);
    }

    #[test]
    fn two_rules_cannot_both_name_the_model() {
        let spec = spec_of(vec![
            rule(r"^([^/]+)/", &[("1", Role::ModelName)]),
            rule(r"/([^/]+)\.stl$", &[("1", Role::ModelName)]),
        ]);
        assert!(analyze(&spec, CarveTarget::Bundle, &[], &vocab()).is_err());
    }

    #[test]
    fn roles_for_absent_groups_are_ignored_not_an_error() {
        // Editing a saved layout down to fewer capture groups must still plan, so
        // the UI stays in matching mode (and greys the now-absent roles) instead
        // of erroring back to the plain file tree. Here the roles map still names
        // groups 3 and 4, but the pattern now has only two.
        let spec = spec_of(vec![rule(
            r"[^/]+/([^/]+)/([^/]+)/[^/]+\.stl",
            &[
                ("1", Role::ModelTag),
                ("2", Role::ModelName),
                ("3", Role::VariantTag), // no such group any more
                ("4", Role::VariantTag), // ditto
            ],
        )]);
        let files = vec![file(1, "DownloadAll/1 - Heroes/Gold", "Gold.stl")];
        let plan = analyze(&spec, CarveTarget::Bundle, &files, &vocab())
            .expect("fewer groups than the roles map should not error");
        assert_eq!(
            plan.rules[0].groups.len(),
            2,
            "only the two real groups are reported"
        );
        let gold = plan.models.iter().find(|m| m.name == "Gold").expect("gold");
        assert_eq!(gold.tags, vec!["1 - Heroes"]);
    }

    #[test]
    fn identity_is_resolved_tags_not_raw_captures() {
        // Supported_LYCHEE and Supported_Lychee fold to one mapped value; the
        // two files land on the same variant.
        let files = vec![
            file(
                1,
                "X/1 - Heroes/Set_32mm_Supported_LYCHEE/Gold_32mm_Supported_LYCHEE",
                "a.stl",
            ),
            file(
                2,
                "X/1 - Heroes/Set_32mm_Supported_LYCHEE/Gold_32mm_Supported_Lychee",
                "b.stl",
            ),
        ];
        let plan = analyze(&loot_spec(), CarveTarget::Bundle, &files, &vocab()).unwrap();
        assert_eq!(plan.models.len(), 1);
        assert_eq!(plan.models[0].variants.len(), 1);
        assert_eq!(plan.models[0].variants[0].file_count, 2);
    }

    #[test]
    fn unmapped_values_are_reported_not_guessed() {
        let mut spec = loot_spec();
        spec.rules[0].value_map.clear();
        let files = vec![file(
            1,
            "X/1 - Heroes/Set_32mm_Supported_LYCHEE/Gold_32mm_Supported_LYCHEE",
            "a.stl",
        )];
        let plan = analyze(&spec, CarveTarget::Bundle, &files, &vocab()).unwrap();
        // 32mm resolves through the vocabulary; Supported_LYCHEE does not — and
        // it is reported humanised, the way it shows in the mapping table.
        assert_eq!(plan.unmapped_values(), vec!["Supported LYCHEE"]);
        assert_eq!(plan.annotations[0].variant_tags, vec!["32mm"]);
        assert_eq!(plan.annotations[0].unmapped, vec!["Supported LYCHEE"]);
    }

    #[test]
    fn separator_styles_are_one_captured_value() {
        // The same value spelled three ways — underscored, spaced, camelCase —
        // is one row in the mapping table, keyed by its humanised form, not
        // three rows that all read the same.
        let mut spec = spec_of(vec![rule(
            r"([^/]+)/[^/]+\.stl",
            &[("1", Role::VariantTag)],
        )]);
        spec.rules[0].value_map =
            HashMap::from([("pre supported".into(), vec!["supported".into()])]);
        let files = vec![
            file(1, "Pre_Supported", "a.stl"),
            file(2, "Pre Supported", "b.stl"),
            file(3, "PreSupported", "c.stl"),
        ];
        let plan = analyze(&spec, CarveTarget::Bundle, &files, &vocab()).unwrap();
        assert_eq!(plan.rules[0].values.len(), 1);
        assert_eq!(plan.rules[0].values[0].raw, "Pre Supported");
        assert_eq!(
            plan.rules[0].values[0].tags.as_deref(),
            Some(&["supported".to_string()][..])
        );
        assert!(plan.unmapped_values().is_empty());
    }

    #[test]
    fn ui_edit_beats_a_legacy_spelling_of_the_same_value() {
        // A template that carries both the legacy key and the UI's re-map of the
        // same value, which dropped lychee_project. The removal must stick — the
        // stale spelling cannot bring the tag back.
        let mut spec = loot_spec();
        spec.rules[0]
            .value_map
            .insert("supported lychee".into(), vec!["supported".into()]);
        let files = vec![file(
            1,
            "X/1 - Heroes/Set_32mm_Supported_LYCHEE/Gold_32mm_Supported_LYCHEE",
            "a.stl",
        )];
        let plan = analyze(&spec, CarveTarget::Bundle, &files, &vocab()).unwrap();
        let tags = &plan.annotations[0].variant_tags;
        assert!(tags.contains(&"supported".to_string()));
        assert!(
            !tags.contains(&"lychee_project".to_string()),
            "removed tag came back: {tags:?}"
        );
    }

    #[test]
    fn one_model_target_pools_variants_and_suggests_the_name() {
        let plan = analyze(&loot_spec(), CarveTarget::Model, &loot_files(), &vocab()).unwrap();
        assert_eq!(plan.models.len(), 1);
        assert_eq!(plan.models[0].variants.len(), 2);
        // Two distinct mini names captured -> no single-name suggestion, and
        // the caller can see this is really a bundle.
        assert_eq!(plan.models[0].name, "");
        assert_eq!(plan.model_names, vec!["Gold", "Sanjay"]);
    }

    #[test]
    fn parts_reassemble_the_path_and_mark_roles() {
        let files = loot_files();
        let plan = analyze(&loot_spec(), CarveTarget::Bundle, &files, &vocab()).unwrap();
        let annotation = &plan.annotations[0];
        assert!(annotation.matched);
        let rebuilt: String = annotation.parts.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(rebuilt, files[0].full_path());
        let marked: Vec<(Role, &str)> = annotation
            .parts
            .iter()
            .filter_map(|p| p.role.map(|r| (r, p.text.as_str())))
            .collect();
        assert_eq!(
            marked,
            vec![
                (Role::ModelTag, "Heroes"),
                (Role::ModelName, "Gold"),
                (Role::VariantTag, "32mm"),
                (Role::VariantTag, "Supported_Lychee"),
            ]
        );
        // The unmatched PDF is one plain segment.
        let pdf = &plan.annotations[3];
        assert!(!pdf.matched && pdf.parts.len() == 1 && pdf.parts[0].role.is_none());
    }

    #[test]
    fn lazy_name_group_survives_underscored_names() {
        let files = vec![file(
            1,
            "X/2 - Enemies/Set_32mm_Supported/Twin_Blade_32mm_Supported",
            "a.stl",
        )];
        let plan = analyze(&loot_spec(), CarveTarget::Bundle, &files, &vocab()).unwrap();
        assert_eq!(plan.models[0].name, "Twin_Blade");
    }

    #[test]
    fn bad_input_is_rejected() {
        let mut spec = loot_spec();
        spec.rules[0].pattern = "(".into();
        assert!(analyze(&spec, CarveTarget::Bundle, &[], &vocab()).is_err());

        // A bad pattern on a *disabled* rule is still rejected: it is saved with
        // the layout, and a broken template that only breaks when you switch a
        // rule back on is worse than one that refuses to save.
        let mut spec = loot_spec();
        spec.rules[0].enabled = false;
        spec.rules[0].pattern = "(".into();
        assert!(analyze(&spec, CarveTarget::Bundle, &[], &vocab()).is_err());

        // A role for a group the pattern doesn't have is NOT rejected — it is
        // ignored, so editing a saved layout down to fewer groups still plans (see
        // roles_for_absent_groups_are_ignored_not_an_error).
        let mut spec = loot_spec();
        spec.rules[0].roles.insert("9".into(), Role::ModelTag);
        assert!(analyze(&spec, CarveTarget::Bundle, &[], &vocab()).is_ok());

        let mut spec = loot_spec();
        spec.rules[0].roles.insert("1".into(), Role::ModelName); // second model-name group
        assert!(analyze(&spec, CarveTarget::Bundle, &[], &vocab()).is_err());
    }

    #[test]
    fn fancy_dialect_lookahead_is_allowed() {
        let spec = spec_of(vec![rule(
            // Lookahead: only sets that also ship a lychee edition — needs
            // fancy-regex, would be a syntax error in the plain regex crate.
            r"([^/]+)(?=_32mm)_32mm[^/]*/[^/]+\.stl",
            &[("1", Role::ModelName)],
        )]);
        let files = vec![file(1, "Gold_32mm_Supported", "a.stl")];
        let plan = analyze(&spec, CarveTarget::Bundle, &files, &vocab()).unwrap();
        assert_eq!(plan.matched, 1);
        assert_eq!(plan.models[0].name, "Gold");
    }
}
