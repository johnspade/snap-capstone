use std::collections::{HashMap, HashSet};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as BASE64;
use serde::de::{self, MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use thiserror::Error;

use crate::filesystem::{self, Tree};
use crate::text::{self, EditOp, EditScript};
use crate::version::{ContributorId, Version};

const MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

#[derive(Debug, Error)]
pub enum ValidationError {
    #[error("duplicate JSON key {0}")]
    DuplicateJsonKey(String),
    #[error("unsupported format version: {0}")]
    UnsupportedFormat(u64),
    #[error("revision is not a positive safe integer")]
    NotPositiveSafeInteger,
    #[error("invalid contributor id: {0}")]
    InvalidContributorId(String),
    #[error("patch message is empty")]
    EmptyMessage,
    #[error("patch message contains forbidden control character")]
    ForbiddenControlInMessage,
    #[error("commit message exceeds 4096 bytes")]
    CommitMessageTooLong,
    #[error("patch changes is empty")]
    EmptyChanges,
    #[error("changes not sorted by path")]
    ChangesNotSorted,
    #[error("duplicate change path: {0}")]
    DuplicateChangePath(String),
    #[error("path is invalid: {0}")]
    InvalidPath(String),
    #[error("content is not canonical base64")]
    InvalidBase64,
    #[error("patches not sorted by author then revision")]
    PatchesNotSorted,
    #[error("duplicate dot: ({0}, {1})")]
    DuplicateDot(String, u64),
    #[error("missing {0} revision {1}")]
    MissingRevision(String, u64),
    #[error("revision = base[author] + 1 violated for ({0}, {1})")]
    RevisionRuleViolated(String, u64),
    #[error("patch base is not a subset of its causal closure")]
    BaseClosure,
    #[error("cyclic or incomplete patch history")]
    CyclicHistory,
    #[error("edit script does not consume old content")]
    EditNotConsumed,
    #[error("edit script consumes beyond old content")]
    EditOverconsumed,
    #[error("text edit on non-text file: {0}")]
    TextEditOnBinary(String),
    #[error("text create on existing path: {0}")]
    CreateOnExisting(String),
    #[error("delete of absent path: {0}")]
    DeleteAbsent(String),
    #[error("no-op change on path: {0}")]
    NoOpChange(String),
    #[error("tree paths conflict: {path} and {nested}")]
    PrefixConflict { path: String, nested: String },
    #[error("frontier replay mismatch")]
    FrontierReplayMismatch,
    #[error("unreachable patch: ({0}, {1})")]
    UnreachablePatch(String, u64),
    #[error("adjacent insert/delete/retain operations in edit script")]
    AdjacentSameKind,
    #[error("insert token is not canonical")]
    NonCanonicalInsertToken,
    #[error("{0}")]
    Json(String),
}

impl From<serde_json::Error> for ValidationError {
    fn from(e: serde_json::Error) -> Self {
        Self::Json(strip_serde_location(&e.to_string()))
    }
}

fn strip_serde_location(msg: &str) -> String {
    msg.rfind(" at line ")
        .map_or_else(|| msg.to_owned(), |idx| msg[..idx].to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Change {
    Text { path: String, edit: EditScript },
    Put { path: String, content: Vec<u8> },
    Delete { path: String },
}

impl Change {
    #[must_use]
    pub fn path(&self) -> &str {
        match self {
            Self::Text { path, .. } | Self::Put { path, .. } | Self::Delete { path } => path,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Patch {
    pub author: ContributorId,
    pub revision: u64,
    pub base: Version,
    pub message: String,
    pub changes: Vec<Change>,
}

impl Patch {
    #[must_use]
    pub const fn dot(&self) -> (&ContributorId, u64) {
        (&self.author, self.revision)
    }

    /// # Panics
    /// Panics if the base version is invalid (should not happen for validated patches).
    #[must_use]
    pub fn result_version(&self) -> Version {
        let mut components: Vec<(ContributorId, u64)> = self
            .base
            .components()
            .iter()
            .filter(|(id, _)| id != &self.author)
            .cloned()
            .collect();
        components.push((self.author.clone(), self.revision));
        Version::new(components).expect("patch base is valid, so result is valid")
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Repository {
    pub frontier: Version,
    pub patches: Vec<Patch>,
}

impl Repository {
    #[must_use]
    pub const fn empty() -> Self {
        Self {
            frontier: Version::empty(),
            patches: Vec::new(),
        }
    }
}

// ── Validation ──────────────────────────────────────────────────────

/// # Errors
/// Returns `ValidationError` if the message is empty or contains forbidden control chars.
pub fn validate_message(message: &str) -> Result<(), ValidationError> {
    if message.is_empty() {
        return Err(ValidationError::EmptyMessage);
    }
    for byte in message.bytes() {
        if byte.is_ascii_control() && byte != b'\t' && byte != b'\n' {
            return Err(ValidationError::ForbiddenControlInMessage);
        }
    }
    Ok(())
}

/// # Errors
/// Returns `ValidationError` if the message fails basic validation or exceeds 4096 bytes.
pub fn validate_commit_message(message: &str) -> Result<(), ValidationError> {
    validate_message(message)?;
    if message.len() > 4096 {
        return Err(ValidationError::CommitMessageTooLong);
    }
    Ok(())
}

fn validate_changes(changes: &[Change]) -> Result<(), ValidationError> {
    if changes.is_empty() {
        return Err(ValidationError::EmptyChanges);
    }
    for (i, change) in changes.iter().enumerate() {
        filesystem::validate_path(change.path())
            .map_err(|_| ValidationError::InvalidPath(change.path().to_owned()))?;
        if i > 0 && change.path() <= changes[i - 1].path() {
            if change.path() == changes[i - 1].path() {
                return Err(ValidationError::DuplicateChangePath(
                    change.path().to_owned(),
                ));
            }
            return Err(ValidationError::ChangesNotSorted);
        }
    }
    Ok(())
}

fn validate_patch_sorting(patches: &[Patch]) -> Result<(), ValidationError> {
    for i in 1..patches.len() {
        let prev = &patches[i - 1];
        let curr = &patches[i];
        match prev.author.as_str().cmp(curr.author.as_str()) {
            std::cmp::Ordering::Greater => return Err(ValidationError::PatchesNotSorted),
            std::cmp::Ordering::Equal => {
                if prev.revision >= curr.revision {
                    return Err(ValidationError::PatchesNotSorted);
                }
            }
            std::cmp::Ordering::Less => {}
        }
    }
    Ok(())
}

fn validate_unique_dots(patches: &[Patch]) -> Result<(), ValidationError> {
    let mut seen = HashSet::new();
    for patch in patches {
        if !seen.insert((patch.author.as_str(), patch.revision)) {
            return Err(ValidationError::DuplicateDot(
                patch.author.as_str().to_owned(),
                patch.revision,
            ));
        }
    }
    Ok(())
}

fn validate_contiguous_revisions(patches: &[Patch]) -> Result<(), ValidationError> {
    let mut max_revisions: HashMap<&str, u64> = HashMap::new();
    for patch in patches {
        let entry = max_revisions.entry(patch.author.as_str()).or_insert(0);
        *entry = (*entry).max(patch.revision);
    }
    for (author, max_rev) in &max_revisions {
        let present: HashSet<u64> = patches
            .iter()
            .filter(|p| p.author.as_str() == *author)
            .map(|p| p.revision)
            .collect();
        for rev in 1..=*max_rev {
            if !present.contains(&rev) {
                return Err(ValidationError::MissingRevision((*author).to_owned(), rev));
            }
        }
    }
    Ok(())
}

fn validate_revision_rule(patch: &Patch) -> Result<(), ValidationError> {
    let expected = patch.base.get(&patch.author) + 1;
    if patch.revision != expected {
        return Err(ValidationError::RevisionRuleViolated(
            patch.author.as_str().to_owned(),
            patch.revision,
        ));
    }
    Ok(())
}

fn validate_base_closure(patch: &Patch, patches: &[Patch]) -> Result<(), ValidationError> {
    for (id, rev) in patch.base.components() {
        for r in 1..=*rev {
            if !patches.iter().any(|p| p.author == *id && p.revision == r) {
                return Err(ValidationError::MissingRevision(id.as_str().to_owned(), r));
            }
        }
    }
    Ok(())
}

fn validate_acyclic(patches: &[Patch]) -> Result<Vec<usize>, ValidationError> {
    let n = patches.len();
    let mut integrated = vec![false; n];
    let mut order = Vec::with_capacity(n);

    for _ in 0..n {
        let mut ready: Option<usize> = None;
        for (idx, patch) in patches.iter().enumerate() {
            if integrated[idx] {
                continue;
            }
            let base_satisfied = patch.base.components().iter().all(|(id, rev)| {
                (1..=*rev).all(|r| {
                    patches
                        .iter()
                        .enumerate()
                        .any(|(j, p)| integrated[j] && p.author == *id && p.revision == r)
                })
            });
            if base_satisfied {
                match ready {
                    None => ready = Some(idx),
                    Some(prev_idx) => {
                        let prev_result = patches[prev_idx].result_version();
                        let curr_result = patch.result_version();
                        if curr_result.snap_cmp(&prev_result) == std::cmp::Ordering::Less {
                            ready = Some(idx);
                        }
                    }
                }
            }
        }
        match ready {
            Some(idx) => {
                integrated[idx] = true;
                order.push(idx);
            }
            None => return Err(ValidationError::CyclicHistory),
        }
    }

    Ok(order)
}

fn validate_changes_against_bases(
    patches: &[Patch],
    order: &[usize],
) -> Result<(), ValidationError> {
    let mut tree_cache: HashMap<Version, Tree> = HashMap::new();
    tree_cache.insert(Version::empty(), Tree::new());

    for &idx in order {
        let patch = &patches[idx];
        let base_tree = tree_cache
            .get(&patch.base)
            .ok_or(ValidationError::BaseClosure)?
            .clone();

        let result_tree = apply_patch_changes(&base_tree, patch)?;
        let result_version = patch.result_version();
        tree_cache.insert(result_version, result_tree);
    }

    Ok(())
}

fn validate_frontier_version(patches: &[Patch], frontier: &Version) -> Result<(), ValidationError> {
    let mut joined = Version::empty();
    for patch in patches {
        joined = joined.join(&patch.result_version());
    }
    if &joined != frontier {
        return Err(ValidationError::FrontierReplayMismatch);
    }
    Ok(())
}

fn apply_patch_changes(base_tree: &Tree, patch: &Patch) -> Result<Tree, ValidationError> {
    let mut tree = base_tree.clone();

    for change in &patch.changes {
        match change {
            Change::Text { path, edit } => {
                apply_text_change(&mut tree, base_tree, path, edit)?;
            }
            Change::Put { path, content } => {
                apply_put_change(&mut tree, base_tree, path, content)?;
            }
            Change::Delete { path } => {
                if !base_tree.contains_key(path) {
                    return Err(ValidationError::DeleteAbsent(path.clone()));
                }
                tree.remove(path);
            }
        }
    }

    validate_tree_prefix_free(&tree)?;
    Ok(tree)
}

fn apply_text_change(
    tree: &mut Tree,
    base_tree: &Tree,
    path: &str,
    edit: &EditScript,
) -> Result<(), ValidationError> {
    match base_tree.get(path) {
        None => {
            let old_tokens: Vec<&str> = Vec::new();
            let new_tokens = edit
                .apply(&old_tokens)
                .map_err(|_| ValidationError::EditNotConsumed)?;
            validate_result_tokens(&new_tokens)?;
            let new_content: String = new_tokens.concat();
            tree.insert(path.to_owned(), new_content.into_bytes());
        }
        Some(bytes) => {
            if edit.ops().is_empty() {
                return Err(ValidationError::CreateOnExisting(path.to_owned()));
            }
            if !text::is_text(bytes) {
                return Err(ValidationError::TextEditOnBinary(path.to_owned()));
            }
            let content = std::str::from_utf8(bytes)
                .map_err(|_| ValidationError::TextEditOnBinary(path.to_owned()))?;
            let old_tokens = text::tokenize(content);
            let new_tokens = edit.apply(&old_tokens).map_err(|e| match e {
                text::EditScriptError::IncompleteConsumption => {
                    if edit_consumes_more_than(&old_tokens, edit) {
                        ValidationError::EditOverconsumed
                    } else {
                        ValidationError::EditNotConsumed
                    }
                }
                _ => ValidationError::EditNotConsumed,
            })?;
            validate_result_tokens(&new_tokens)?;
            let new_content: String = new_tokens.concat();
            if new_content.as_bytes() == bytes {
                return Err(ValidationError::NoOpChange(path.to_owned()));
            }
            tree.insert(path.to_owned(), new_content.into_bytes());
        }
    }
    Ok(())
}

fn edit_consumes_more_than(old_tokens: &[&str], edit: &EditScript) -> bool {
    let mut consumed = 0usize;
    for op in edit.ops() {
        match op {
            EditOp::Retain(n) | EditOp::Delete(n) => consumed += n,
            EditOp::Insert(_) => {}
        }
    }
    consumed > old_tokens.len()
}

fn validate_result_tokens(tokens: &[String]) -> Result<(), ValidationError> {
    for (i, token) in tokens.iter().enumerate() {
        if i < tokens.len() - 1 && !token.ends_with('\n') {
            return Err(ValidationError::NonCanonicalInsertToken);
        }
    }
    Ok(())
}

fn apply_put_change(
    tree: &mut Tree,
    base_tree: &Tree,
    path: &str,
    content: &[u8],
) -> Result<(), ValidationError> {
    if let Some(existing) = base_tree.get(path) {
        if existing == content {
            return Err(ValidationError::NoOpChange(path.to_owned()));
        }
    }
    tree.insert(path.to_owned(), content.to_vec());
    Ok(())
}

fn validate_tree_prefix_free(tree: &Tree) -> Result<(), ValidationError> {
    let mut paths: Vec<&str> = tree.keys().map(String::as_str).collect();
    paths.sort_unstable();
    for pair in paths.windows(2) {
        let a = pair[0];
        let b = pair[1];
        if b.starts_with(a) && b.as_bytes().get(a.len()) == Some(&b'/') {
            return Err(ValidationError::PrefixConflict {
                path: a.to_owned(),
                nested: b.to_owned(),
            });
        }
    }
    Ok(())
}

fn validate_frontier_reachability(
    frontier: &Version,
    patches: &[Patch],
) -> Result<(), ValidationError> {
    let needed: HashSet<(String, u64)> = frontier
        .components()
        .iter()
        .flat_map(|(id, rev)| (1..=*rev).map(move |r| (id.as_str().to_owned(), r)))
        .collect();

    let present: HashSet<(String, u64)> = patches
        .iter()
        .map(|p| (p.author.as_str().to_owned(), p.revision))
        .collect();

    for dot in &needed {
        if !present.contains(dot) {
            return Err(ValidationError::MissingRevision(dot.0.clone(), dot.1));
        }
    }

    for dot in &present {
        if !needed.contains(dot) {
            return Err(ValidationError::UnreachablePatch(dot.0.clone(), dot.1));
        }
    }

    Ok(())
}

/// # Errors
/// Returns `ValidationError` if any of the 6 validation steps fail.
pub fn validate(repo: &Repository) -> Result<(), ValidationError> {
    for patch in &repo.patches {
        validate_message(&patch.message)?;
        validate_changes(&patch.changes)?;
    }

    validate_patch_sorting(&repo.patches)?;
    validate_unique_dots(&repo.patches)?;
    validate_contiguous_revisions(&repo.patches)?;
    validate_frontier_reachability(&repo.frontier, &repo.patches)?;

    for patch in &repo.patches {
        validate_revision_rule(patch)?;
        validate_base_closure(patch, &repo.patches)?;
    }

    let order = validate_acyclic(&repo.patches)?;
    validate_changes_against_bases(&repo.patches, &order)?;
    validate_frontier_version(&repo.patches, &repo.frontier)?;

    Ok(())
}

// ── JSON parsing with strict validation ─────────────────────────────

/// # Errors
/// Returns `ValidationError` if the JSON is malformed or the repository fails validation.
pub fn parse(json: &str) -> Result<Repository, ValidationError> {
    check_duplicate_keys(json)?;
    let value: serde_json::Value = serde_json::from_str(json)
        .map_err(|e| ValidationError::Json(strip_serde_location(&e.to_string())))?;
    check_unknown_fields(&value)?;
    let raw: RawRepository = serde_json::from_str(json)?;
    let repo = raw.into_repository()?;
    validate(&repo)?;
    Ok(repo)
}

fn check_unknown_fields(value: &serde_json::Value) -> Result<(), ValidationError> {
    const ROOT_KEYS: &[&str] = &["format", "frontier", "patches"];
    const PATCH_KEYS: &[&str] = &["author", "revision", "base", "message", "changes"];

    let obj = value
        .as_object()
        .ok_or_else(|| ValidationError::Json("expected object".to_owned()))?;

    for key in obj.keys() {
        if !ROOT_KEYS.contains(&key.as_str()) {
            return Err(ValidationError::Json(format!(
                "repository has unknown field: {key}"
            )));
        }
    }

    if let Some(patches) = obj.get("patches").and_then(|v| v.as_array()) {
        for patch_val in patches {
            if let Some(p) = patch_val.as_object() {
                for key in p.keys() {
                    if !PATCH_KEYS.contains(&key.as_str()) {
                        return Err(ValidationError::Json(format!(
                            "patch has unknown field: {key}"
                        )));
                    }
                }
                check_change_unknown_fields(p)?;
            }
        }
    }

    Ok(())
}

fn check_change_unknown_fields(
    patch_obj: &serde_json::Map<String, serde_json::Value>,
) -> Result<(), ValidationError> {
    let Some(changes) = patch_obj.get("changes").and_then(|v| v.as_array()) else {
        return Ok(());
    };

    for change_val in changes {
        let Some(c) = change_val.as_object() else {
            continue;
        };
        let change_type = c.get("type").and_then(|v| v.as_str()).unwrap_or("");
        let known_keys: &[&str] = match change_type {
            "text" => &["type", "path", "edit"],
            "put" => &["type", "path", "content"],
            "delete" => &["type", "path"],
            _ => continue,
        };
        for key in c.keys() {
            if !known_keys.contains(&key.as_str()) {
                return Err(ValidationError::Json(format!(
                    "change has unknown field: {key}"
                )));
            }
        }
    }

    Ok(())
}

fn check_duplicate_keys(json: &str) -> Result<(), ValidationError> {
    let bytes = json.as_bytes();
    let mut in_string = false;
    let mut escape = false;
    let mut object_keys_stack: Vec<HashSet<String>> = Vec::new();
    let mut array_depth = 0u32;
    let mut object_depth = 0u32;
    let mut awaiting_key = false;
    let mut current_key_start: Option<usize> = None;

    for (i, &b) in bytes.iter().enumerate() {
        if in_string {
            if escape {
                escape = false;
            } else if b == b'\\' {
                escape = true;
            } else if b == b'"' {
                in_string = false;
                if let Some(start) = current_key_start.take() {
                    let key_json = &bytes[start..=i];
                    let key: String = serde_json::from_slice(key_json)
                        .map_err(|e| ValidationError::Json(e.to_string()))?;
                    if let Some(keys) = object_keys_stack.last_mut() {
                        if !keys.insert(key.clone()) {
                            return Err(ValidationError::DuplicateJsonKey(key));
                        }
                    }
                    awaiting_key = false;
                }
            }
        } else {
            match b {
                b'"' => {
                    in_string = true;
                    if awaiting_key {
                        current_key_start = Some(i);
                    }
                }
                b'{' => {
                    object_depth += 1;
                    object_keys_stack.push(HashSet::new());
                    awaiting_key = true;
                }
                b'}' => {
                    object_depth -= 1;
                    object_keys_stack.pop();
                    awaiting_key = false;
                }
                b'[' => array_depth += 1,
                b']' => array_depth -= 1,
                b',' => {
                    if object_depth as usize == object_keys_stack.len() && array_depth == 0
                        || (object_depth > 0 && object_keys_stack.len() == object_depth as usize)
                    {
                        awaiting_key = true;
                    }
                }
                _ => {}
            }
        }
    }
    Ok(())
}

#[derive(Deserialize)]
struct RawRepository {
    format: u64,
    frontier: Version,
    patches: Vec<RawPatch>,
}

impl RawRepository {
    fn into_repository(self) -> Result<Repository, ValidationError> {
        if self.format != 1 {
            return Err(ValidationError::UnsupportedFormat(self.format));
        }
        let patches = self
            .patches
            .into_iter()
            .map(RawPatch::into_patch)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Repository {
            frontier: self.frontier,
            patches,
        })
    }
}

#[derive(Deserialize)]
struct RawPatch {
    author: String,
    revision: SafeInt,
    base: Version,
    message: String,
    changes: Vec<RawChange>,
}

impl RawPatch {
    fn into_patch(self) -> Result<Patch, ValidationError> {
        let author = ContributorId::new(&self.author)
            .map_err(|_| ValidationError::InvalidContributorId(self.author.clone()))?;
        let changes = self
            .changes
            .into_iter()
            .map(RawChange::into_change)
            .collect::<Result<Vec<_>, _>>()?;
        Ok(Patch {
            author,
            revision: self.revision.0,
            base: self.base,
            message: self.message,
            changes,
        })
    }
}

#[derive(Debug, Clone, Copy)]
struct SafeInt(u64);

impl<'de> Deserialize<'de> for SafeInt {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct SafeIntVisitor;

        impl Visitor<'_> for SafeIntVisitor {
            type Value = SafeInt;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("a positive safe integer")
            }

            fn visit_u64<E: de::Error>(self, v: u64) -> Result<Self::Value, E> {
                if v == 0 || v > MAX_SAFE_INTEGER {
                    return Err(E::custom("revision is not a positive safe integer"));
                }
                Ok(SafeInt(v))
            }

            fn visit_i64<E: de::Error>(self, v: i64) -> Result<Self::Value, E> {
                if v <= 0 {
                    return Err(E::custom("revision is not a positive safe integer"));
                }
                let unsigned = u64::try_from(v)
                    .map_err(|_| E::custom("revision is not a positive safe integer"))?;
                if unsigned > MAX_SAFE_INTEGER {
                    return Err(E::custom("revision is not a positive safe integer"));
                }
                Ok(SafeInt(unsigned))
            }

            fn visit_f64<E: de::Error>(self, _v: f64) -> Result<Self::Value, E> {
                Err(E::custom("revision is not a positive safe integer"))
            }
        }

        deserializer.deserialize_any(SafeIntVisitor)
    }
}

#[derive(Deserialize)]
#[serde(tag = "type")]
enum RawChange {
    #[serde(rename = "text")]
    Text { path: String, edit: Vec<RawEditOp> },
    #[serde(rename = "put")]
    Put { path: String, content: String },
    #[serde(rename = "delete")]
    Delete { path: String },
}

impl RawChange {
    fn into_change(self) -> Result<Change, ValidationError> {
        match self {
            Self::Text { path, edit } => {
                let ops: Vec<EditOp> = edit.into_iter().map(|raw| raw.op).collect();
                let script = EditScript::new(ops).map_err(|e| match e {
                    text::EditScriptError::ZeroCount => ValidationError::NotPositiveSafeInteger,
                    text::EditScriptError::AdjacentSameKind => ValidationError::AdjacentSameKind,
                    text::EditScriptError::EmptyInsertToken
                    | text::EditScriptError::NonCanonicalResult => {
                        ValidationError::NonCanonicalInsertToken
                    }
                    text::EditScriptError::IncompleteConsumption => {
                        ValidationError::EditNotConsumed
                    }
                })?;
                Ok(Change::Text { path, edit: script })
            }
            Self::Put { path, content } => {
                let bytes = BASE64
                    .decode(&content)
                    .map_err(|_| ValidationError::InvalidBase64)?;
                let re_encoded = BASE64.encode(&bytes);
                if re_encoded != content {
                    return Err(ValidationError::InvalidBase64);
                }
                Ok(Change::Put {
                    path,
                    content: bytes,
                })
            }
            Self::Delete { path } => Ok(Change::Delete { path }),
        }
    }
}

#[derive(Debug)]
struct RawEditOp {
    op: EditOp,
}

impl<'de> Deserialize<'de> for RawEditOp {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        struct EditOpVisitor;

        impl<'de> Visitor<'de> for EditOpVisitor {
            type Value = RawEditOp;

            fn expecting(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                formatter.write_str("an edit operation object with exactly one key")
            }

            fn visit_map<A: MapAccess<'de>>(self, mut map: A) -> Result<Self::Value, A::Error> {
                let first_key: String = map
                    .next_key()?
                    .ok_or_else(|| de::Error::custom("edit operation is empty"))?;

                let op = match first_key.as_str() {
                    "retain" => {
                        let n: SafeInt = map.next_value()?;
                        EditOp::Retain(usize::try_from(n.0).map_err(de::Error::custom)?)
                    }
                    "delete" => {
                        let n: SafeInt = map.next_value()?;
                        EditOp::Delete(usize::try_from(n.0).map_err(de::Error::custom)?)
                    }
                    "insert" => {
                        let tokens: Vec<String> = map.next_value()?;
                        if tokens.is_empty() {
                            return Err(de::Error::custom("edit operation insert is empty"));
                        }
                        for token in &tokens {
                            if token.is_empty() {
                                return Err(de::Error::custom("insert token is empty"));
                            }
                        }
                        EditOp::Insert(tokens)
                    }
                    other => {
                        return Err(de::Error::custom(format!(
                            "unknown edit operation: {other}"
                        )));
                    }
                };

                if map.next_key::<String>()?.is_some() {
                    return Err(de::Error::custom("edit operation must have one operation"));
                }

                Ok(RawEditOp { op })
            }
        }

        deserializer.deserialize_map(EditOpVisitor)
    }
}

// ── Serialization / writing ─────────────────────────────────────────

/// Serialize a repository to canonical JSON with 2-space indent and trailing LF.
///
/// # Panics
/// Panics if serde serialization fails (should not happen for valid data).
#[must_use]
pub fn serialize(repo: &Repository) -> String {
    let raw = RawRepositoryOut::from_repository(repo);
    let mut json = serde_json::to_string_pretty(&raw).expect("serialization must not fail");
    json.push('\n');
    json
}

#[derive(Serialize)]
struct RawRepositoryOut<'a> {
    format: u64,
    frontier: &'a Version,
    patches: Vec<RawPatchOut<'a>>,
}

impl<'a> RawRepositoryOut<'a> {
    fn from_repository(repo: &'a Repository) -> Self {
        let mut sorted_patches: Vec<&'a Patch> = repo.patches.iter().collect();
        sorted_patches.sort_by(|a, b| {
            a.author
                .as_str()
                .cmp(b.author.as_str())
                .then(a.revision.cmp(&b.revision))
        });
        Self {
            format: 1,
            frontier: &repo.frontier,
            patches: sorted_patches
                .iter()
                .map(|p| RawPatchOut::from_patch(p))
                .collect(),
        }
    }
}

#[derive(Serialize)]
struct RawPatchOut<'a> {
    author: &'a str,
    revision: u64,
    base: &'a Version,
    message: &'a str,
    changes: Vec<RawChangeOut<'a>>,
}

impl<'a> RawPatchOut<'a> {
    fn from_patch(patch: &'a Patch) -> Self {
        Self {
            author: patch.author.as_str(),
            revision: patch.revision,
            base: &patch.base,
            message: &patch.message,
            changes: patch
                .changes
                .iter()
                .map(RawChangeOut::from_change)
                .collect(),
        }
    }
}

#[derive(Serialize)]
#[serde(tag = "type")]
enum RawChangeOut<'a> {
    #[serde(rename = "text")]
    Text {
        path: &'a str,
        edit: Vec<EditOpOut<'a>>,
    },
    #[serde(rename = "put")]
    Put { path: &'a str, content: String },
    #[serde(rename = "delete")]
    Delete { path: &'a str },
}

impl<'a> RawChangeOut<'a> {
    fn from_change(change: &'a Change) -> Self {
        match change {
            Change::Text { path, edit } => Self::Text {
                path,
                edit: edit.ops().iter().map(EditOpOut::from_op).collect(),
            },
            Change::Put { path, content } => Self::Put {
                path,
                content: BASE64.encode(content),
            },
            Change::Delete { path } => Self::Delete { path },
        }
    }
}

enum EditOpOut<'a> {
    Retain(usize),
    Delete(usize),
    Insert(&'a [String]),
}

impl<'a> EditOpOut<'a> {
    fn from_op(op: &'a EditOp) -> Self {
        match op {
            EditOp::Retain(n) => Self::Retain(*n),
            EditOp::Delete(n) => Self::Delete(*n),
            EditOp::Insert(tokens) => Self::Insert(tokens),
        }
    }
}

impl Serialize for EditOpOut<'_> {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let mut map = serializer.serialize_map(Some(1))?;
        match self {
            Self::Retain(n) => map.serialize_entry("retain", n)?,
            Self::Delete(n) => map.serialize_entry("delete", n)?,
            Self::Insert(tokens) => map.serialize_entry("insert", tokens)?,
        }
        map.end()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_empty_repo_json() -> String {
        r#"{"format":1,"frontier":[],"patches":[]}"#.to_owned()
    }

    #[test]
    fn parse_empty_repository() {
        let repo = parse(&make_empty_repo_json()).unwrap();
        assert!(repo.frontier.is_empty());
        assert!(repo.patches.is_empty());
    }

    #[test]
    fn parse_single_patch() {
        let json = r#"{
          "format": 1,
          "frontier": [["a@x", 1]],
          "patches": [
            {
              "author": "a@x",
              "revision": 1,
              "base": [],
              "message": "hello",
              "changes": [{"type": "text", "path": "f.txt", "edit": [{"insert": ["hello\n"]}]}]
            }
          ]
        }"#;
        let repo = parse(json).unwrap();
        assert_eq!(repo.patches.len(), 1);
        assert_eq!(repo.patches[0].author.as_str(), "a@x");
        assert_eq!(repo.patches[0].revision, 1);
    }

    #[test]
    fn rejects_duplicate_json_keys() {
        let json = r#"{"format":1,"format":1,"frontier":[],"patches":[]}"#;
        let err = parse(json).unwrap_err();
        assert!(
            matches!(err, ValidationError::DuplicateJsonKey(ref k) if k == "format"),
            "got: {err}"
        );
    }

    #[test]
    fn rejects_unknown_field_in_root() {
        let json = r#"{"format":1,"frontier":[],"patches":[],"unknown":true}"#;
        let err = parse(json).unwrap_err();
        assert!(
            matches!(err, ValidationError::Json(ref s) if s.contains("unknown field")),
            "got: {err}"
        );
    }

    #[test]
    fn rejects_fractional_revision() {
        let json = r#"{
          "format": 1,
          "frontier": [["a@x", 1]],
          "patches": [{
            "author": "a@x", "revision": 1.5, "base": [], "message": "x",
            "changes": [{"type": "text", "path": "f", "edit": []}]
          }]
        }"#;
        let err = parse(json).unwrap_err();
        assert!(
            matches!(err, ValidationError::Json(ref s) if s.contains("positive safe integer")),
            "got: {err}"
        );
    }

    #[test]
    fn rejects_empty_message() {
        let json = r#"{
          "format": 1, "frontier": [["a@x", 1]],
          "patches": [{
            "author": "a@x", "revision": 1, "base": [], "message": "",
            "changes": [{"type": "text", "path": "f", "edit": []}]
          }]
        }"#;
        assert!(matches!(
            parse(json).unwrap_err(),
            ValidationError::EmptyMessage
        ));
    }

    #[test]
    fn rejects_empty_changes() {
        let json = r#"{
          "format": 1, "frontier": [["a@x", 1]],
          "patches": [{
            "author": "a@x", "revision": 1, "base": [], "message": "none", "changes": []
          }]
        }"#;
        assert!(matches!(
            parse(json).unwrap_err(),
            ValidationError::EmptyChanges
        ));
    }

    #[test]
    fn rejects_invalid_path() {
        let json = r#"{
          "format": 1, "frontier": [["a@x", 1]],
          "patches": [{
            "author": "a@x", "revision": 1, "base": [], "message": "bad",
            "changes": [{"type": "put", "path": ".snap/secret", "content": "YQ=="}]
          }]
        }"#;
        let err = parse(json).unwrap_err();
        assert!(matches!(err, ValidationError::InvalidPath(ref p) if p == ".snap/secret"));
    }

    #[test]
    fn rejects_invalid_base64() {
        let json = r#"{
          "format": 1, "frontier": [["a@x", 1]],
          "patches": [{
            "author": "a@x", "revision": 1, "base": [], "message": "bad",
            "changes": [{"type": "put", "path": "f", "content": "abc"}]
          }]
        }"#;
        assert!(matches!(
            parse(json).unwrap_err(),
            ValidationError::InvalidBase64
        ));
    }

    #[test]
    fn rejects_patches_not_sorted() {
        let json = r#"{
          "format": 1, "frontier": [["a@x", 1], ["b@x", 1]],
          "patches": [
            {"author": "b@x", "revision": 1, "base": [], "message": "b",
             "changes": [{"type": "text", "path": "b", "edit": []}]},
            {"author": "a@x", "revision": 1, "base": [], "message": "a",
             "changes": [{"type": "text", "path": "a", "edit": []}]}
          ]
        }"#;
        assert!(matches!(
            parse(json).unwrap_err(),
            ValidationError::PatchesNotSorted
        ));
    }

    #[test]
    fn rejects_missing_revision() {
        let json = r#"{
          "format": 1, "frontier": [["a@x", 2]],
          "patches": [{
            "author": "a@x", "revision": 2, "base": [["a@x", 1]], "message": "gap",
            "changes": [{"type": "text", "path": "f", "edit": []}]
          }]
        }"#;
        let err = parse(json).unwrap_err();
        assert!(matches!(err, ValidationError::MissingRevision(ref a, 1) if a == "a@x"));
    }

    #[test]
    fn rejects_cyclic_history() {
        let json = r#"{
          "format": 1, "frontier": [["a@x", 1], ["b@x", 1]],
          "patches": [
            {"author": "a@x", "revision": 1, "base": [["b@x", 1]], "message": "cycle a",
             "changes": [{"type": "text", "path": "a", "edit": []}]},
            {"author": "b@x", "revision": 1, "base": [["a@x", 1]], "message": "cycle b",
             "changes": [{"type": "text", "path": "b", "edit": []}]}
          ]
        }"#;
        assert!(matches!(
            parse(json).unwrap_err(),
            ValidationError::CyclicHistory
        ));
    }

    #[test]
    fn rejects_no_op_change() {
        let json = r#"{
          "format": 1, "frontier": [["a@x", 2]],
          "patches": [
            {"author": "a@x", "revision": 1, "base": [], "message": "base",
             "changes": [{"type": "put", "path": "f", "content": "YQ=="}]},
            {"author": "a@x", "revision": 2, "base": [["a@x", 1]], "message": "no op",
             "changes": [{"type": "put", "path": "f", "content": "YQ=="}]}
          ]
        }"#;
        let err = parse(json).unwrap_err();
        assert!(matches!(err, ValidationError::NoOpChange(ref p) if p == "f"));
    }

    #[test]
    fn rejects_unreachable_patch() {
        let json = r#"{
          "format": 1, "frontier": [],
          "patches": [{
            "author": "a@x", "revision": 1, "base": [], "message": "unreachable",
            "changes": [{"type": "text", "path": "f", "edit": []}]
          }]
        }"#;
        let err = parse(json).unwrap_err();
        assert!(matches!(err, ValidationError::UnreachablePatch(ref a, 1) if a == "a@x"));
    }

    #[test]
    fn rejects_edit_underconsumption() {
        let json = r#"{
          "format": 1, "frontier": [["a@x", 2]],
          "patches": [
            {"author": "a@x", "revision": 1, "base": [], "message": "base",
             "changes": [{"type": "text", "path": "f", "edit": [{"insert": ["one\n", "two\n"]}]}]},
            {"author": "a@x", "revision": 2, "base": [["a@x", 1]], "message": "underconsume",
             "changes": [{"type": "text", "path": "f", "edit": [{"retain": 1}]}]}
          ]
        }"#;
        assert!(matches!(
            parse(json).unwrap_err(),
            ValidationError::EditNotConsumed
        ));
    }

    #[test]
    fn rejects_prefix_conflict() {
        let json = r#"{
          "format": 1, "frontier": [["a@x", 1]],
          "patches": [{
            "author": "a@x", "revision": 1, "base": [], "message": "prefix",
            "changes": [
              {"type": "put", "path": "a", "content": "YQ=="},
              {"type": "put", "path": "a/b", "content": "Yg=="}
            ]
          }]
        }"#;
        assert!(matches!(
            parse(json).unwrap_err(),
            ValidationError::PrefixConflict { .. }
        ));
    }

    #[test]
    fn rejects_edit_op_multiple_keys() {
        let json = r#"{
          "format": 1, "frontier": [["a@x", 1]],
          "patches": [{
            "author": "a@x", "revision": 1, "base": [], "message": "bad op",
            "changes": [{"type": "text", "path": "f", "edit": [{"retain": 1, "delete": 1}]}]
          }]
        }"#;
        let err = parse(json).unwrap_err();
        assert!(matches!(err, ValidationError::Json(ref s) if s.contains("one operation")));
    }

    #[test]
    fn rejects_zero_retain() {
        let json = r#"{
          "format": 1, "frontier": [["a@x", 1]],
          "patches": [{
            "author": "a@x", "revision": 1, "base": [], "message": "bad count",
            "changes": [{"type": "text", "path": "f", "edit": [{"retain": 0}]}]
          }]
        }"#;
        let err = parse(json).unwrap_err();
        assert!(matches!(err, ValidationError::Json(ref s) if s.contains("positive safe integer")));
    }

    #[test]
    fn rejects_empty_insert() {
        let json = r#"{
          "format": 1, "frontier": [["a@x", 1]],
          "patches": [{
            "author": "a@x", "revision": 1, "base": [], "message": "empty insert",
            "changes": [{"type": "text", "path": "f", "edit": [{"insert": []}]}]
          }]
        }"#;
        let err = parse(json).unwrap_err();
        assert!(matches!(err, ValidationError::Json(ref s) if s.contains("insert is empty")));
    }

    #[test]
    fn rejects_adjacent_inserts() {
        let json = r#"{
          "format": 1, "frontier": [["a@x", 1]],
          "patches": [{
            "author": "a@x", "revision": 1, "base": [], "message": "adjacent",
            "changes": [{"type": "text", "path": "f", "edit": [{"insert": ["a\n"]}, {"insert": ["b\n"]}]}]
          }]
        }"#;
        assert!(matches!(
            parse(json).unwrap_err(),
            ValidationError::AdjacentSameKind
        ));
    }

    #[test]
    fn rejects_delete_absent_path() {
        let json = r#"{
          "format": 1, "frontier": [["a@x", 1], ["b@x", 1]],
          "patches": [
            {"author": "a@x", "revision": 1, "base": [], "message": "base",
             "changes": [{"type": "put", "path": "f", "content": "YQ=="}]},
            {"author": "b@x", "revision": 1, "base": [], "message": "absent",
             "changes": [{"type": "delete", "path": "f"}]}
          ]
        }"#;
        let err = parse(json).unwrap_err();
        assert!(matches!(err, ValidationError::DeleteAbsent(ref p) if p == "f"));
    }

    #[test]
    fn rejects_changes_not_sorted() {
        let json = r#"{
          "format": 1, "frontier": [["a@x", 1]],
          "patches": [{
            "author": "a@x", "revision": 1, "base": [], "message": "order",
            "changes": [
              {"type": "text", "path": "z", "edit": []},
              {"type": "text", "path": "a", "edit": []}
            ]
          }]
        }"#;
        assert!(matches!(
            parse(json).unwrap_err(),
            ValidationError::ChangesNotSorted
        ));
    }

    #[test]
    fn rejects_wrong_revision_rule() {
        let json = r#"{
          "format": 1, "frontier": [["a@x", 1]],
          "patches": [{
            "author": "a@x", "revision": 1, "base": [["a@x", 1]], "message": "wrong dot",
            "changes": [{"type": "text", "path": "f", "edit": []}]
          }]
        }"#;
        assert!(matches!(
            parse(json).unwrap_err(),
            ValidationError::RevisionRuleViolated(..)
        ));
    }

    #[test]
    fn rejects_overconsumption() {
        let json = r#"{
          "format": 1, "frontier": [["a@x", 1], ["b@x", 1]],
          "patches": [
            {"author": "a@x", "revision": 1, "base": [], "message": "base",
             "changes": [{"type": "text", "path": "f", "edit": [{"insert": ["one\n"]}]}]},
            {"author": "b@x", "revision": 1, "base": [["a@x", 1]], "message": "overconsume",
             "changes": [{"type": "text", "path": "f", "edit": [{"delete": 2}]}]}
          ]
        }"#;
        let err = parse(json).unwrap_err();
        assert!(matches!(
            err,
            ValidationError::EditOverconsumed | ValidationError::EditNotConsumed
        ));
    }

    #[test]
    fn rejects_create_on_existing() {
        let json = r#"{
          "format": 1, "frontier": [["a@x", 2]],
          "patches": [
            {"author": "a@x", "revision": 1, "base": [], "message": "base",
             "changes": [{"type": "put", "path": "f", "content": "YQ=="}]},
            {"author": "a@x", "revision": 2, "base": [["a@x", 1]], "message": "create present",
             "changes": [{"type": "text", "path": "f", "edit": []}]}
          ]
        }"#;
        let err = parse(json).unwrap_err();
        assert!(matches!(err, ValidationError::CreateOnExisting(ref p) if p == "f"));
    }

    #[test]
    fn rejects_text_edit_on_binary() {
        let json = r#"{
          "format": 1, "frontier": [["a@x", 2]],
          "patches": [
            {"author": "a@x", "revision": 1, "base": [], "message": "binary",
             "changes": [{"type": "put", "path": "f", "content": "AA=="}]},
            {"author": "a@x", "revision": 2, "base": [["a@x", 1]], "message": "text over binary",
             "changes": [{"type": "text", "path": "f", "edit": [{"delete": 1}]}]}
          ]
        }"#;
        let err = parse(json).unwrap_err();
        assert!(matches!(err, ValidationError::TextEditOnBinary(ref p) if p == "f"));
    }

    #[test]
    fn rejects_non_canonical_insert_tokens() {
        let json = r#"{
          "format": 1, "frontier": [["a@x", 1]],
          "patches": [{
            "author": "a@x", "revision": 1, "base": [], "message": "bad token",
            "changes": [{"type": "text", "path": "f", "edit": [{"insert": ["a", "b"]}]}]
          }]
        }"#;
        assert!(matches!(
            parse(json).unwrap_err(),
            ValidationError::NonCanonicalInsertToken
        ));
    }

    #[test]
    fn rejects_unknown_field_in_change() {
        let json = r#"{
          "format": 1, "frontier": [["a@x", 1]],
          "patches": [{
            "author": "a@x", "revision": 1, "base": [], "message": "change field",
            "changes": [{"type": "put", "path": "f", "content": "YQ==", "extra": 1}]
          }]
        }"#;
        let err = parse(json).unwrap_err();
        assert!(matches!(err, ValidationError::Json(ref s) if s.contains("unknown")));
    }

    #[test]
    fn rejects_unknown_field_in_patch() {
        let json = r#"{
          "format": 1, "frontier": [["a@x", 1]],
          "patches": [{
            "author": "a@x", "revision": 1, "base": [], "message": "x",
            "changes": [{"type": "text", "path": "f", "edit": []}],
            "unknown": true
          }]
        }"#;
        let err = parse(json).unwrap_err();
        assert!(matches!(err, ValidationError::Json(ref s) if s.contains("unknown")));
    }

    #[test]
    fn serialize_empty_repo() {
        let repo = Repository::empty();
        let json = serialize(&repo);
        assert!(json.ends_with('\n'));
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["format"], 1);
        assert_eq!(parsed["frontier"], serde_json::json!([]));
        assert_eq!(parsed["patches"], serde_json::json!([]));
    }

    #[test]
    fn serialize_round_trip() {
        let json = r#"{
          "format": 1,
          "frontier": [["a@x", 1]],
          "patches": [{
            "author": "a@x", "revision": 1, "base": [], "message": "hello",
            "changes": [{"type": "text", "path": "f.txt", "edit": [{"insert": ["hello\n"]}]}]
          }]
        }"#;
        let repo = parse(json).unwrap();
        let serialized = serialize(&repo);
        let repo2 = parse(&serialized).unwrap();
        assert_eq!(repo.frontier, repo2.frontier);
        assert_eq!(repo.patches.len(), repo2.patches.len());
    }

    #[test]
    fn patch_result_version() {
        let author = ContributorId::new("a@x").unwrap();
        let patch = Patch {
            author: author.clone(),
            revision: 1,
            base: Version::empty(),
            message: "first".to_owned(),
            changes: vec![Change::Text {
                path: "f".to_owned(),
                edit: EditScript::new(vec![]).unwrap(),
            }],
        };
        let result = patch.result_version();
        assert_eq!(result.get(&author), 1);
    }

    #[test]
    fn patch_result_version_with_base() {
        let a = ContributorId::new("a@x").unwrap();
        let b = ContributorId::new("b@y").unwrap();
        let base = Version::new(vec![(a.clone(), 1), (b.clone(), 2)]).unwrap();
        let patch = Patch {
            author: a.clone(),
            revision: 2,
            base,
            message: "second".to_owned(),
            changes: vec![Change::Text {
                path: "f".to_owned(),
                edit: EditScript::new(vec![EditOp::Retain(1)]).unwrap(),
            }],
        };
        let result = patch.result_version();
        assert_eq!(result.get(&a), 2);
        assert_eq!(result.get(&b), 2);
    }

    #[test]
    fn message_validation_allows_tab_and_lf() {
        assert!(validate_message("hello\tworld\n").is_ok());
    }

    #[test]
    fn message_validation_rejects_nul() {
        assert!(validate_message("hello\x00world").is_err());
    }

    #[test]
    fn message_validation_rejects_other_control() {
        assert!(validate_message("hello\x01world").is_err());
    }

    #[test]
    fn rejects_frontier_not_canonical() {
        let json = r#"{"format":1,"frontier":[["b@x",1],["a@x",1]],"patches":[]}"#;
        let err = parse(json).unwrap_err();
        assert!(
            format!("{err}").contains("canonical") || format!("{err}").contains("duplicate"),
            "got: {err}"
        );
    }

    #[test]
    fn valid_two_concurrent_patches() {
        let json = r#"{
          "format": 1, "frontier": [["a@x", 1], ["b@x", 1]],
          "patches": [
            {"author": "a@x", "revision": 1, "base": [], "message": "a's change",
             "changes": [{"type": "text", "path": "a.txt", "edit": []}]},
            {"author": "b@x", "revision": 1, "base": [], "message": "b's change",
             "changes": [{"type": "text", "path": "b.txt", "edit": []}]}
          ]
        }"#;
        let repo = parse(json).unwrap();
        assert_eq!(repo.patches.len(), 2);
    }

    #[test]
    fn commit_message_rejects_too_long() {
        let long = "a".repeat(4097);
        assert!(matches!(
            validate_commit_message(&long).unwrap_err(),
            ValidationError::CommitMessageTooLong
        ));
    }

    #[test]
    fn commit_message_allows_4096_bytes() {
        let exact = "a".repeat(4096);
        assert!(validate_commit_message(&exact).is_ok());
    }

    #[test]
    fn commit_message_rejects_empty() {
        assert!(matches!(
            validate_commit_message("").unwrap_err(),
            ValidationError::EmptyMessage
        ));
    }

    #[test]
    fn empty_text_edit_creates_empty_file() {
        let json = r#"{
          "format": 1, "frontier": [["a@x", 1]],
          "patches": [{
            "author": "a@x", "revision": 1, "base": [], "message": "create empty",
            "changes": [{"type": "text", "path": "empty.txt", "edit": []}]
          }]
        }"#;
        let repo = parse(json).unwrap();
        assert_eq!(repo.patches.len(), 1);
    }

    #[test]
    fn rejects_adjacent_retains() {
        let json = r#"{
          "format": 1, "frontier": [["a@x", 1]],
          "patches": [{
            "author": "a@x", "revision": 1, "base": [], "message": "adj retain",
            "changes": [{"type": "text", "path": "f", "edit": [{"retain": 1}, {"retain": 1}]}]
          }]
        }"#;
        assert!(matches!(
            parse(json).unwrap_err(),
            ValidationError::AdjacentSameKind
        ));
    }

    #[test]
    fn rejects_invalid_contributor_id() {
        let json = r#"{
          "format": 1, "frontier": [["bad", 1]],
          "patches": [{
            "author": "bad", "revision": 1, "base": [], "message": "no at sign",
            "changes": [{"type": "text", "path": "f", "edit": []}]
          }]
        }"#;
        let err = parse(json).unwrap_err();
        assert!(
            format!("{err}").contains("contributor") || format!("{err}").contains("canonical"),
            "got: {err}"
        );
    }
}
