//! Import layout templates: a regex whose capture groups carve a staged
//! import into models and variants (docs/plan.md, "Import layout templates").
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

/// The user-editable definition: pattern, what each group means, and how raw
/// captured values translate into variant tags.
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct LayoutSpec {
    /// fancy-regex, matched against the file's full logical path
    /// (`path/filename`), implicitly anchored at both ends.
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

/// A slice of a file's path: plain text, or the text a capture group matched
/// (highlighted in the UI, coloured by the group's role).
#[derive(Serialize, ToSchema)]
pub struct PathPart {
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub group: Option<u32>,
}

/// Everything the UI needs to draw one file row under the active layout.
#[derive(Serialize, ToSchema)]
pub struct FileAnnotation {
    pub id: Uuid,
    pub matched: bool,
    pub parts: Vec<PathPart>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub model_name: Option<String>,
    pub model_tags: Vec<String>,
    pub variant_tags: Vec<String>,
    /// Raw variant-tag captures with no resolution — the mapping table's todo.
    pub unmapped: Vec<String>,
}

/// One planned variant of a planned model. Empty `tags` = the model's
/// unsorted bucket (matched files that resolved no variant tags).
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
    /// First-seen raw spelling; the value-map key is its lowercase.
    pub raw: String,
    pub tags: Option<Vec<String>>,
}

/// The dry-run result: preview for the UI, carve for the commit.
#[derive(Serialize, ToSchema)]
pub struct Plan {
    pub total: usize,
    pub matched: usize,
    /// Files that actually land on a model/variant (a matched file without a
    /// model name still falls to unsorted under a bundle target).
    pub carved: usize,
    pub groups: Vec<GroupInfo>,
    pub models: Vec<PlanModel>,
    pub values: Vec<CapturedValue>,
    /// Distinct model names captured — >1 under a one-model target is the
    /// "this is really a bundle" signal.
    pub model_names: Vec<String>,
    pub annotations: Vec<FileAnnotation>,
}

impl Plan {
    /// Raw values the commit must refuse on: mapping them is the user's call,
    /// not something to guess at.
    pub fn unmapped_values(&self) -> Vec<&str> {
        self.values
            .iter()
            .filter(|v| v.tags.is_none())
            .map(|v| v.raw.as_str())
            .collect()
    }
}

fn fold(s: &str) -> String {
    s.trim().to_lowercase()
}

/// Run a layout over a file list. Pure — `vocab` is the lowercased variant-tag
/// vocabulary, fetched by the caller — so the same call backs the plan
/// endpoint, the commit, and the tests.
pub fn analyze(
    spec: &LayoutSpec,
    target: CarveTarget,
    files: &[PlanFile],
    vocab: &HashSet<String>,
) -> Result<Plan, ApiError> {
    // Implicit anchoring: wrap rather than full-span-check the first match,
    // so a full-length match is found even when an unanchored search would
    // return a shorter one. The non-capturing group keeps group numbers.
    let regex = Regex::new(&format!("^(?:{})$", spec.pattern))
        .map_err(|e| ApiError::BadRequest(format!("bad pattern: {e}")))?;
    let group_count = regex.captures_len(); // includes group 0

    let mut roles: Vec<Role> = vec![Role::Ignore; group_count];
    for (key, role) in &spec.roles {
        let index: usize = key
            .parse()
            .ok()
            .filter(|i| (1..group_count).contains(i))
            .ok_or_else(|| {
                ApiError::BadRequest(format!("no capture group {key} in the pattern"))
            })?;
        roles[index] = *role;
    }
    if roles.iter().filter(|r| **r == Role::ModelName).count() > 1 {
        return Err(ApiError::BadRequest(
            "at most one group can be the model name".into(),
        ));
    }

    let value_map: HashMap<String, &Vec<String>> =
        spec.value_map.iter().map(|(k, v)| (fold(k), v)).collect();
    let resolve = |raw: &str| -> Option<Vec<String>> {
        let key = fold(raw);
        if let Some(tags) = value_map.get(&key) {
            return Some(
                tags.iter()
                    .map(|t| fold(t))
                    .filter(|t| !t.is_empty())
                    .collect(),
            );
        }
        vocab.contains(&key).then(|| vec![key])
    };

    let mut annotations = Vec::with_capacity(files.len());
    let mut groups: Vec<GroupInfo> = (1..group_count)
        .map(|i| GroupInfo {
            index: i as u32,
            examples: Vec::new(),
        })
        .collect();
    let mut values: BTreeMap<String, CapturedValue> = BTreeMap::new();
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
        let caps = regex
            .captures(&full)
            .map_err(|e| ApiError::BadRequest(format!("pattern failed to run: {e}")))?;
        let Some(caps) = caps else {
            annotations.push(FileAnnotation {
                id: file.id,
                matched: false,
                parts: vec![PathPart {
                    text: full,
                    group: None,
                }],
                model_name: None,
                model_tags: vec![],
                variant_tags: vec![],
                unmapped: vec![],
            });
            continue;
        };
        matched += 1;

        // Highlight spans: non-overlapping capture ranges, first-come wins on
        // nesting so the UI never has to deal with overlapping marks.
        let mut spans: Vec<(usize, usize, u32)> = (1..group_count)
            .filter_map(|i| caps.get(i).map(|m| (m.start(), m.end(), i as u32)))
            .filter(|(s, e, _)| s < e)
            .collect();
        spans.sort_unstable();
        let mut parts = Vec::new();
        let mut cursor = 0usize;
        for (start, end, index) in spans {
            if start < cursor {
                continue;
            }
            if start > cursor {
                parts.push(PathPart {
                    text: full[cursor..start].into(),
                    group: None,
                });
            }
            parts.push(PathPart {
                text: full[start..end].into(),
                group: Some(index),
            });
            cursor = end;
        }
        if cursor < full.len() {
            parts.push(PathPart {
                text: full[cursor..].into(),
                group: None,
            });
        }

        let mut model_name: Option<String> = None;
        let mut model_tags: Vec<String> = Vec::new();
        let mut variant_tags: Vec<String> = Vec::new();
        let mut unmapped: Vec<String> = Vec::new();
        for (index, role) in roles.iter().enumerate().skip(1) {
            let Some(m) = caps.get(index) else { continue };
            let raw = m.as_str().trim();
            if raw.is_empty() {
                continue;
            }
            if let Some(examples) = groups.get_mut(index - 1)
                && !examples.examples.iter().any(|e| e == raw)
                && examples.examples.len() < 3
            {
                examples.examples.push(raw.to_string());
            }
            match role {
                Role::ModelName => model_name = Some(raw.to_string()),
                Role::ModelTag => {
                    if !model_tags.iter().any(|t| fold(t) == fold(raw)) {
                        model_tags.push(raw.to_string());
                    }
                }
                Role::VariantTag => {
                    let resolved = resolve(raw);
                    values.entry(fold(raw)).or_insert_with(|| CapturedValue {
                        raw: raw.to_string(),
                        tags: resolved.clone(),
                    });
                    match resolved {
                        Some(tags) => variant_tags.extend(tags),
                        None => unmapped.push(raw.to_string()),
                    }
                }
                Role::Ignore => {}
            }
        }
        variant_tags.sort_unstable();
        variant_tags.dedup();

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
        groups,
        models,
        values: values.into_values().collect(),
        model_names: model_names.into_values().collect(),
        annotations,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const LOOT_PATTERN: &str =
        r"(?i)^(?:[^/]+/)*?\d+ - ([^/]+)/[^/]+/([^/]+?)_(\d+mm)_([^/]+)/[^/]+\.stl$";

    fn loot_spec() -> LayoutSpec {
        LayoutSpec {
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
                ("nosupports".into(), vec!["unsupported".into()]),
            ]),
        }
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
        spec.value_map.clear();
        let files = vec![file(
            1,
            "X/1 - Heroes/Set_32mm_Supported_LYCHEE/Gold_32mm_Supported_LYCHEE",
            "a.stl",
        )];
        let plan = analyze(&spec, CarveTarget::Bundle, &files, &vocab()).unwrap();
        // 32mm resolves through the vocabulary; Supported_LYCHEE does not.
        assert_eq!(plan.unmapped_values(), vec!["Supported_LYCHEE"]);
        assert_eq!(plan.annotations[0].variant_tags, vec!["32mm"]);
        assert_eq!(plan.annotations[0].unmapped, vec!["Supported_LYCHEE"]);
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
    fn parts_reassemble_the_path_and_mark_groups() {
        let files = loot_files();
        let plan = analyze(&loot_spec(), CarveTarget::Bundle, &files, &vocab()).unwrap();
        let annotation = &plan.annotations[0];
        assert!(annotation.matched);
        let rebuilt: String = annotation.parts.iter().map(|p| p.text.as_str()).collect();
        assert_eq!(rebuilt, files[0].full_path());
        let marked: Vec<(u32, &str)> = annotation
            .parts
            .iter()
            .filter_map(|p| p.group.map(|g| (g, p.text.as_str())))
            .collect();
        assert_eq!(
            marked,
            vec![
                (1, "Heroes"),
                (2, "Gold"),
                (3, "32mm"),
                (4, "Supported_Lychee")
            ]
        );
        // The unmatched PDF is one plain segment.
        let pdf = &plan.annotations[3];
        assert!(!pdf.matched && pdf.parts.len() == 1 && pdf.parts[0].group.is_none());
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
        spec.pattern = "(".into();
        assert!(analyze(&spec, CarveTarget::Bundle, &[], &vocab()).is_err());

        let mut spec = loot_spec();
        spec.roles.insert("9".into(), Role::ModelTag);
        assert!(analyze(&spec, CarveTarget::Bundle, &[], &vocab()).is_err());

        let mut spec = loot_spec();
        spec.roles.insert("1".into(), Role::ModelName); // second model-name group
        assert!(analyze(&spec, CarveTarget::Bundle, &[], &vocab()).is_err());
    }

    #[test]
    fn fancy_dialect_lookahead_is_allowed() {
        let spec = LayoutSpec {
            // Lookahead: only sets that also ship a lychee edition — needs
            // fancy-regex, would be a syntax error in the plain regex crate.
            pattern: r"([^/]+)(?=_32mm)_32mm[^/]*/[^/]+\.stl".into(),
            roles: HashMap::from([("1".into(), Role::ModelName)]),
            value_map: HashMap::new(),
        };
        let files = vec![file(1, "Gold_32mm_Supported", "a.stl")];
        let plan = analyze(&spec, CarveTarget::Bundle, &files, &vocab()).unwrap();
        assert_eq!(plan.matched, 1);
        assert_eq!(plan.models[0].name, "Gold");
    }
}
