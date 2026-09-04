use std::collections::{BTreeSet, HashMap, HashSet};

use thiserror::Error;

use crate::filesystem::Tree;
use crate::repository::{Change, Patch};
use crate::text;
use crate::version::Version;

#[derive(Debug, Error)]
pub enum ReplayError {
    #[error("cyclic or missing dependency in patch history")]
    CyclicOrMissingDependency,
    #[error("OT transform failed: {0}")]
    TransformFailed(#[from] text::TransformError),
    #[error("edit script application failed")]
    EditApplyFailed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReplayResult {
    pub tree: Tree,
    pub warnings: Vec<(String, String)>,
}

/// Deterministic replay: given patches and a target version, reconstruct the
/// file tree from the empty tree by integrating patches in canonical order (§6).
///
/// # Errors
/// Returns `ReplayError` if the history has a cycle/missing dependency or OT fails.
///
/// # Panics
/// Panics if a base version that was just cached is unexpectedly missing.
pub fn replay(patches: &[Patch], target: &Version) -> Result<ReplayResult, ReplayError> {
    let mut cache: HashMap<Version, Tree> = HashMap::new();
    cache.insert(Version::empty(), Tree::new());

    let selected = select_patches(patches, target);
    let order = integration_order(&selected)?;

    let mut canonical = Tree::new();
    let mut warnings: BTreeSet<(String, String)> = BTreeSet::new();
    let mut joined = Version::empty();

    for patch in &order {
        ensure_base_cached(patches, &patch.base, &mut cache)?;

        let base = cache
            .get(&patch.base)
            .expect("base was just cached")
            .clone();
        let authored = compute_authored_tree(&base, patch);

        integrate_patch(patch, &base, &mut canonical, &authored, &mut warnings)?;

        joined = joined.join(&patch.result_version());
        cache.insert(joined.clone(), canonical.clone());
    }

    Ok(ReplayResult {
        tree: canonical,
        warnings: warnings.into_iter().collect(),
    })
}

fn select_patches<'a>(patches: &'a [Patch], target: &Version) -> Vec<&'a Patch> {
    patches
        .iter()
        .filter(|p| {
            let target_rev = target.get(&p.author);
            target_rev > 0 && p.revision <= target_rev
        })
        .collect()
}

fn integration_order<'a>(patches: &[&'a Patch]) -> Result<Vec<&'a Patch>, ReplayError> {
    let n = patches.len();
    let mut integrated = vec![false; n];
    let mut order = Vec::with_capacity(n);

    for _ in 0..n {
        let mut best: Option<usize> = None;
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
            if !base_satisfied {
                continue;
            }
            best = Some(best.map_or(idx, |prev| {
                if integration_less(patch, patches[prev]) {
                    idx
                } else {
                    prev
                }
            }));
        }
        match best {
            Some(idx) => {
                integrated[idx] = true;
                order.push(patches[idx]);
            }
            None => return Err(ReplayError::CyclicOrMissingDependency),
        }
    }

    Ok(order)
}

fn integration_less(a: &Patch, b: &Patch) -> bool {
    let a_result = a.result_version();
    let b_result = b.result_version();
    let cmp = a_result
        .snap_cmp(&b_result)
        .then_with(|| a.author.as_str().cmp(b.author.as_str()))
        .then_with(|| a.revision.cmp(&b.revision));
    cmp == std::cmp::Ordering::Less
}

fn ensure_base_cached(
    all_patches: &[Patch],
    base_version: &Version,
    cache: &mut HashMap<Version, Tree>,
) -> Result<(), ReplayError> {
    if cache.contains_key(base_version) {
        return Ok(());
    }

    let selected = select_patches(all_patches, base_version);
    let order = integration_order(&selected)?;

    let mut canonical = Tree::new();
    let mut joined = Version::empty();

    for patch in &order {
        ensure_base_cached(all_patches, &patch.base, cache)?;

        let base = cache
            .get(&patch.base)
            .expect("base was just cached")
            .clone();
        let authored = compute_authored_tree(&base, patch);

        let mut sub_warnings: BTreeSet<(String, String)> = BTreeSet::new();
        integrate_patch(patch, &base, &mut canonical, &authored, &mut sub_warnings)?;

        joined = joined.join(&patch.result_version());
        cache.insert(joined.clone(), canonical.clone());
    }

    Ok(())
}

fn compute_authored_tree(base: &Tree, patch: &Patch) -> Tree {
    let mut tree = base.clone();
    for change in &patch.changes {
        match change {
            Change::Text { path, edit } => {
                let old_tokens: Vec<&str> =
                    base.get(path.as_str()).map_or_else(Vec::new, |bytes| {
                        let s = std::str::from_utf8(bytes).expect("validated text content");
                        text::tokenize(s)
                    });
                let new_tokens = edit.apply(&old_tokens).expect("validated edit script");
                let content: String = new_tokens.concat();
                tree.insert(path.clone(), content.into_bytes());
            }
            Change::Put { path, content } => {
                tree.insert(path.clone(), content.clone());
            }
            Change::Delete { path } => {
                tree.remove(path.as_str());
            }
        }
    }
    tree
}

struct TreeDelta {
    removals: HashSet<String>,
    installs: HashMap<String, Vec<u8>>,
    namespace_settled: HashSet<String>,
    warnings: BTreeSet<(String, String)>,
}

impl TreeDelta {
    fn new() -> Self {
        Self {
            removals: HashSet::new(),
            installs: HashMap::new(),
            namespace_settled: HashSet::new(),
            warnings: BTreeSet::new(),
        }
    }
}

fn integrate_patch(
    patch: &Patch,
    base: &Tree,
    canonical: &mut Tree,
    authored: &Tree,
    warnings: &mut BTreeSet<(String, String)>,
) -> Result<(), ReplayError> {
    let mut delta = TreeDelta::new();

    resolve_namespace_conflicts(patch, canonical, authored, &mut delta);

    for change in &patch.changes {
        let changed_path = change.path();
        if delta.namespace_settled.contains(changed_path) {
            continue;
        }

        resolve_path(change, changed_path, base, canonical, authored, &mut delta)?;
    }

    for path in &delta.removals {
        canonical.remove(path.as_str());
    }
    for (path, content) in delta.installs {
        canonical.insert(path, content);
    }
    warnings.append(&mut delta.warnings);

    Ok(())
}

fn resolve_namespace_conflicts(
    patch: &Patch,
    canonical: &Tree,
    authored: &Tree,
    delta: &mut TreeDelta,
) {
    // S = paths that P makes present
    let s: HashSet<&str> = patch
        .changes
        .iter()
        .filter_map(|c| match c {
            Change::Delete { .. } => None,
            _ => Some(c.path()),
        })
        .filter(|path| authored.contains_key(*path))
        .collect();

    // C' = canonical minus paths that P deletes
    let deleted_by_patch: HashSet<&str> = patch
        .changes
        .iter()
        .filter_map(|c| match c {
            Change::Delete { path } => Some(path.as_str()),
            _ => None,
        })
        .collect();

    let c_prime_paths: Vec<&str> = canonical
        .keys()
        .map(String::as_str)
        .filter(|p| !deleted_by_patch.contains(p))
        .collect();

    let mut paths_to_remove: BTreeSet<String> = BTreeSet::new();
    let mut paths_to_install: HashSet<String> = HashSet::new();

    for &s_path in &s {
        for &c_path in &c_prime_paths {
            if is_namespace_conflict(s_path, c_path) {
                paths_to_remove.insert(c_path.to_owned());
                paths_to_install.insert(s_path.to_owned());
            }
        }
    }

    for path in &paths_to_remove {
        delta.removals.insert(path.clone());
        delta
            .warnings
            .insert((path.clone(), "namespace-wins".to_owned()));
    }

    for path in &paths_to_install {
        if let Some(content) = authored.get(path.as_str()) {
            delta.installs.insert(path.clone(), content.clone());
        }
        delta.namespace_settled.insert(path.clone());
    }
}

fn is_namespace_conflict(s_path: &str, c_path: &str) -> bool {
    if s_path == c_path {
        return false;
    }
    // s_path is ancestor of c_path: c_path starts with s_path + "/"
    if c_path.starts_with(s_path) && c_path.as_bytes().get(s_path.len()) == Some(&b'/') {
        return true;
    }
    // c_path is ancestor of s_path: s_path starts with c_path + "/"
    if s_path.starts_with(c_path) && s_path.as_bytes().get(c_path.len()) == Some(&b'/') {
        return true;
    }
    false
}

fn resolve_path(
    change: &Change,
    changed_path: &str,
    base: &Tree,
    canonical: &Tree,
    authored: &Tree,
    delta: &mut TreeDelta,
) -> Result<(), ReplayError> {
    let b = base.get(changed_path);
    let c = canonical.get(changed_path);
    let t = authored.get(changed_path);

    // Branch 1: B == C → apply authored change directly
    if b == c {
        match t {
            Some(content) => {
                delta
                    .installs
                    .insert(changed_path.to_owned(), content.clone());
            }
            None => {
                delta.removals.insert(changed_path.to_owned());
            }
        }
        return Ok(());
    }

    // Branch 2: C == T → keep unchanged
    if c == t {
        return Ok(());
    }

    // Branch 3: B, C, T all present text AND P is a text change → OT
    if let Change::Text { edit, .. } = change {
        if let (Some(b_bytes), Some(c_bytes), Some(t_bytes)) = (b, c, t) {
            if text::is_text(b_bytes) && text::is_text(c_bytes) && text::is_text(t_bytes) {
                let b_str = std::str::from_utf8(b_bytes).expect("validated text");
                let c_str = std::str::from_utf8(c_bytes).expect("validated text");
                let b_tokens = text::tokenize(b_str);
                let c_tokens = text::tokenize(c_str);
                let context_edit = text::diff(&b_tokens, &c_tokens);
                let transformed = text::transform(edit, &context_edit)?;
                let result_tokens = transformed
                    .apply(&c_tokens)
                    .map_err(|_| ReplayError::EditApplyFailed)?;
                let content: String = result_tokens.concat();
                delta
                    .installs
                    .insert(changed_path.to_owned(), content.into_bytes());
                return Ok(());
            }
        }
    }

    // Branch 4: Path-level rules (§6.4)
    path_level_rules(change, changed_path, b, c, t, delta);
    Ok(())
}

fn path_level_rules(
    change: &Change,
    changed_path: &str,
    b: Option<&Vec<u8>>,
    c: Option<&Vec<u8>>,
    t: Option<&Vec<u8>>,
    delta: &mut TreeDelta,
) {
    // Rule 1: C and T identical → keep C, no warning
    if c == t {
        return;
    }

    // Rule 2: T absent → incoming delete wins
    if t.is_none() {
        delta.removals.insert(changed_path.to_owned());
        delta
            .warnings
            .insert((changed_path.to_owned(), "delete-wins".to_owned()));
        return;
    }

    // Rule 3: B present, C absent → earlier concurrent delete wins
    if b.is_some() && c.is_none() {
        delta
            .warnings
            .insert((changed_path.to_owned(), "delete-wins".to_owned()));
        return;
    }

    // Rule 4: B absent, C and T present → later create wins
    if let (None, Some(_), Some(t_content)) = (b, c, t) {
        delta
            .installs
            .insert(changed_path.to_owned(), t_content.clone());
        delta
            .warnings
            .insert((changed_path.to_owned(), "later-create-wins".to_owned()));
        return;
    }

    // Rule 5: incoming change is put → later put wins
    if let Change::Put { .. } = change {
        delta.installs.insert(
            changed_path.to_owned(),
            t.expect("t is present at rule 5").clone(),
        );
        delta
            .warnings
            .insert((changed_path.to_owned(), "later-put-wins".to_owned()));
        return;
    }

    // Rule 6: P is text, C is non-text → current wins
    delta.installs.insert(
        changed_path.to_owned(),
        c.expect("c is present at rule 6").clone(),
    );
    delta
        .warnings
        .insert((changed_path.to_owned(), "put-wins".to_owned()));
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::text::{EditOp, EditScript};
    use crate::version::ContributorId;

    fn cid(s: &str) -> ContributorId {
        ContributorId::new(s).unwrap()
    }

    fn ver(s: &str) -> Version {
        s.parse().unwrap()
    }

    fn text_change(path: &str, ops: Vec<EditOp>) -> Change {
        Change::Text {
            path: path.to_owned(),
            edit: EditScript::new(ops).unwrap(),
        }
    }

    fn put_change(path: &str, content: &[u8]) -> Change {
        Change::Put {
            path: path.to_owned(),
            content: content.to_vec(),
        }
    }

    fn delete_change(path: &str) -> Change {
        Change::Delete {
            path: path.to_owned(),
        }
    }

    fn make_patch(
        author: &str,
        revision: u64,
        base: &str,
        message: &str,
        changes: Vec<Change>,
    ) -> Patch {
        Patch {
            author: cid(author),
            revision,
            base: ver(base),
            message: message.to_owned(),
            changes,
        }
    }

    // ── Patch selection ────────────────────────────────────────────

    #[test]
    fn select_patches_empty() {
        let patches: Vec<Patch> = vec![];
        let selected = select_patches(&patches, &Version::empty());
        assert!(selected.is_empty());
    }

    #[test]
    fn select_patches_filters_by_target() {
        let patches = vec![
            make_patch("a@x", 1, "()", "first", vec![text_change("f", vec![])]),
            make_patch(
                "a@x",
                2,
                "(a@x->1)",
                "second",
                vec![text_change("f", vec![EditOp::Retain(1)])],
            ),
            make_patch("b@y", 1, "()", "b first", vec![text_change("g", vec![])]),
        ];
        // Target includes only a@x rev 1
        let target = ver("(a@x->1)");
        let selected = select_patches(&patches, &target);
        assert_eq!(selected.len(), 1);
        assert_eq!(selected[0].author.as_str(), "a@x");
        assert_eq!(selected[0].revision, 1);
    }

    #[test]
    fn select_patches_includes_all_for_target() {
        let patches = vec![
            make_patch("a@x", 1, "()", "a1", vec![text_change("f", vec![])]),
            make_patch("b@y", 1, "()", "b1", vec![text_change("g", vec![])]),
        ];
        let target = ver("(a@x->1,b@y->1)");
        let selected = select_patches(&patches, &target);
        assert_eq!(selected.len(), 2);
    }

    // ── Integration ordering ───────────────────────────────────────

    #[test]
    fn integration_order_single_patch() {
        let p = make_patch("a@x", 1, "()", "first", vec![text_change("f", vec![])]);
        let patches = vec![&p];
        let order = integration_order(&patches).unwrap();
        assert_eq!(order.len(), 1);
    }

    #[test]
    fn integration_order_causal_chain() {
        let p1 = make_patch("a@x", 1, "()", "first", vec![text_change("f", vec![])]);
        let p2 = make_patch(
            "a@x",
            2,
            "(a@x->1)",
            "second",
            vec![text_change(
                "f",
                vec![EditOp::Insert(vec!["hi\n".to_owned()])],
            )],
        );
        let patches = vec![&p2, &p1]; // reversed
        let order = integration_order(&patches).unwrap();
        assert_eq!(order[0].revision, 1);
        assert_eq!(order[1].revision, 2);
    }

    #[test]
    fn integration_order_concurrent_uses_snap_order() {
        // a@x->1 result: (a@x->1), b@y->1 result: (b@y->1)
        // snap_cmp: a@x: 1 vs 0 → (a@x->1) > (b@y->1), so b@y goes first
        let pa = make_patch("a@x", 1, "()", "a", vec![text_change("a", vec![])]);
        let pb = make_patch("b@y", 1, "()", "b", vec![text_change("b", vec![])]);
        let patches = vec![&pa, &pb];
        let order = integration_order(&patches).unwrap();
        assert_eq!(order[0].author.as_str(), "b@y");
        assert_eq!(order[1].author.as_str(), "a@x");
    }

    #[test]
    fn integration_order_detects_cycle() {
        let pa = make_patch(
            "a@x",
            1,
            "(b@y->1)",
            "cycle a",
            vec![text_change("a", vec![])],
        );
        let pb = make_patch(
            "b@y",
            1,
            "(a@x->1)",
            "cycle b",
            vec![text_change("b", vec![])],
        );
        let patches = vec![&pa, &pb];
        assert!(integration_order(&patches).is_err());
    }

    // ── Replay: no conflicts ───────────────────────────────────────

    #[test]
    fn replay_empty() {
        let result = replay(&[], &Version::empty()).unwrap();
        assert!(result.tree.is_empty());
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn replay_single_text_create() {
        let patches = vec![make_patch(
            "a@x",
            1,
            "()",
            "create",
            vec![text_change(
                "hello.txt",
                vec![EditOp::Insert(vec!["hello\n".to_owned()])],
            )],
        )];
        let result = replay(&patches, &ver("(a@x->1)")).unwrap();
        assert_eq!(
            result.tree.get("hello.txt").map(Vec::as_slice),
            Some(b"hello\n".as_slice())
        );
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn replay_single_put() {
        let patches = vec![make_patch(
            "a@x",
            1,
            "()",
            "put binary",
            vec![put_change("data.bin", &[0, 1, 2, 3])],
        )];
        let result = replay(&patches, &ver("(a@x->1)")).unwrap();
        assert_eq!(result.tree.get("data.bin").unwrap(), &[0, 1, 2, 3]);
    }

    #[test]
    fn replay_sequential_patches() {
        let patches = vec![
            make_patch(
                "a@x",
                1,
                "()",
                "create",
                vec![text_change(
                    "f.txt",
                    vec![EditOp::Insert(vec!["line1\n".to_owned()])],
                )],
            ),
            make_patch(
                "a@x",
                2,
                "(a@x->1)",
                "append",
                vec![text_change(
                    "f.txt",
                    vec![
                        EditOp::Retain(1),
                        EditOp::Insert(vec!["line2\n".to_owned()]),
                    ],
                )],
            ),
        ];
        let result = replay(&patches, &ver("(a@x->2)")).unwrap();
        assert_eq!(
            std::str::from_utf8(result.tree.get("f.txt").unwrap()).unwrap(),
            "line1\nline2\n"
        );
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn replay_concurrent_no_conflict() {
        let patches = vec![
            make_patch(
                "a@x",
                1,
                "()",
                "create a",
                vec![text_change(
                    "a.txt",
                    vec![EditOp::Insert(vec!["a\n".to_owned()])],
                )],
            ),
            make_patch(
                "b@y",
                1,
                "()",
                "create b",
                vec![text_change(
                    "b.txt",
                    vec![EditOp::Insert(vec!["b\n".to_owned()])],
                )],
            ),
        ];
        let result = replay(&patches, &ver("(a@x->1,b@y->1)")).unwrap();
        assert_eq!(result.tree.len(), 2);
        assert!(result.tree.contains_key("a.txt"));
        assert!(result.tree.contains_key("b.txt"));
        assert!(result.warnings.is_empty());
    }

    // ── Branch 1: B == C → apply directly ──────────────────────────

    #[test]
    fn branch1_base_equals_canonical() {
        // A creates f.txt, B (based on A) modifies f.txt
        // Since B sees A and A is already integrated, B == C for f.txt's base
        let patches = vec![
            make_patch(
                "a@x",
                1,
                "()",
                "create",
                vec![text_change(
                    "f.txt",
                    vec![EditOp::Insert(vec!["old\n".to_owned()])],
                )],
            ),
            make_patch(
                "b@y",
                1,
                "(a@x->1)",
                "modify",
                vec![text_change(
                    "f.txt",
                    vec![EditOp::Delete(1), EditOp::Insert(vec!["new\n".to_owned()])],
                )],
            ),
        ];
        let result = replay(&patches, &ver("(a@x->1,b@y->1)")).unwrap();
        assert_eq!(
            std::str::from_utf8(result.tree.get("f.txt").unwrap()).unwrap(),
            "new\n"
        );
        assert!(result.warnings.is_empty());
    }

    // ── Branch 2: C == T → keep unchanged ──────────────────────────

    #[test]
    fn branch2_identical_concurrent_changes() {
        // A and B both create f.txt with identical content concurrently
        let patches = vec![
            make_patch(
                "a@x",
                1,
                "()",
                "create f",
                vec![put_change("f.txt", b"same content")],
            ),
            make_patch(
                "b@y",
                1,
                "()",
                "also create f",
                vec![put_change("f.txt", b"same content")],
            ),
        ];
        let result = replay(&patches, &ver("(a@x->1,b@y->1)")).unwrap();
        assert_eq!(result.tree.get("f.txt").unwrap(), b"same content");
        assert!(result.warnings.is_empty());
    }

    // ── Branch 3: OT for concurrent text changes ───────────────────

    #[test]
    fn branch3_ot_concurrent_text_edits() {
        // Base file: "line1\n"
        // A appends "lineA\n", B appends "lineB\n" — both concurrent from same base
        let patches = vec![
            make_patch(
                "a@x",
                1,
                "()",
                "create base",
                vec![text_change(
                    "f.txt",
                    vec![EditOp::Insert(vec!["line1\n".to_owned()])],
                )],
            ),
            // b@y creates same file concurrently with different content
            // but for OT, both need to be text edits on a shared base
            // A edits from base (a@x->1), B edits from base (a@x->1),
            // but they're concurrent: A is (a@x->2) based on (a@x->1),
            // B is (b@y->1) based on (a@x->1)
            make_patch(
                "a@x",
                2,
                "(a@x->1)",
                "a appends",
                vec![text_change(
                    "f.txt",
                    vec![
                        EditOp::Retain(1),
                        EditOp::Insert(vec!["lineA\n".to_owned()]),
                    ],
                )],
            ),
            make_patch(
                "b@y",
                1,
                "(a@x->1)",
                "b appends",
                vec![text_change(
                    "f.txt",
                    vec![
                        EditOp::Retain(1),
                        EditOp::Insert(vec!["lineB\n".to_owned()]),
                    ],
                )],
            ),
        ];
        // Integration order: A1 (base), B1 ((a@x->1,b@y->1)), A2 ((a@x->2))
        // snap order: A1 result (a@x->1), B1 result (a@x->1,b@y->1), A2 result (a@x->2)
        // Compare B1 result vs A2 result: a@x: 1 vs 2 → B1 < A2. So B1 before A2.
        // After A1: f.txt = "line1\n"
        // After B1: B=f.txt "line1\n", C=f.txt "line1\n" → B==C, apply directly → f.txt = "line1\nlineB\n"
        // After A2: B=f.txt "line1\n" (from base (a@x->1)), C=f.txt "line1\nlineB\n", T=f.txt "line1\nlineA\n"
        //   B≠C, C≠T, all text, text change → OT
        //   Q = diff(B, C) = diff(["line1\n"], ["line1\n", "lineB\n"]) = retain(1), insert(["lineB\n"])
        //   P = retain(1), insert(["lineA\n"])
        //   transform(P, Q):
        //     Q insert ["lineB\n"] → retain(1) in output, consume Q
        //     P retain(1), Q retain(1) → retain(1), consume both
        //     P insert ["lineA\n"] → insert ["lineA\n"], consume P
        //   Result: retain(1), retain(1) → retain(2), insert(["lineA\n"])
        //   Apply to C tokens ["line1\n", "lineB\n"]: retain 2 → keep both, insert "lineA\n"
        //   = "line1\nlineB\nlineA\n"
        let result = replay(&patches, &ver("(a@x->2,b@y->1)")).unwrap();
        assert_eq!(
            std::str::from_utf8(result.tree.get("f.txt").unwrap()).unwrap(),
            "line1\nlineB\nlineA\n"
        );
        assert!(result.warnings.is_empty());
    }

    // ── Path-level rule 1: C == T identical → no warning ───────────

    #[test]
    fn rule1_identical_result() {
        // This is handled by branch 2 (C==T), but §6.4 rule 1 is there as safety
        // We test it goes through without warning
        let patches = vec![
            make_patch("a@x", 1, "()", "a puts", vec![put_change("f", b"same")]),
            make_patch("b@y", 1, "()", "b puts", vec![put_change("f", b"same")]),
        ];
        let result = replay(&patches, &ver("(a@x->1,b@y->1)")).unwrap();
        assert_eq!(result.tree.get("f").unwrap(), b"same");
        assert!(result.warnings.is_empty());
    }

    // ── Path-level rule 2: T absent → delete-wins ──────────────────

    #[test]
    fn rule2_incoming_delete_wins() {
        // A creates f, B modifies f. C (concurrent with B, based on A) deletes f.
        // Order: A, B, C. When integrating C: B!=base, T=absent → delete-wins
        let patches = vec![
            make_patch("a@x", 1, "()", "create", vec![put_change("f", b"hello")]),
            make_patch(
                "b@y",
                1,
                "(a@x->1)",
                "modify",
                vec![put_change("f", b"modified")],
            ),
            make_patch("c@z", 1, "(a@x->1)", "delete", vec![delete_change("f")]),
        ];
        // target includes all three
        let result = replay(&patches, &ver("(a@x->1,b@y->1,c@z->1)")).unwrap();
        assert!(!result.tree.contains_key("f"));
        assert!(
            result
                .warnings
                .contains(&("f".to_owned(), "delete-wins".to_owned()))
        );
    }

    // ── Path-level rule 3: B present, C absent → earlier delete wins

    #[test]
    fn rule3_earlier_delete_wins() {
        // B creates f. z@z (deletes) and a@a (puts) are both based on B, concurrent.
        // z@z result: (b@y->1,z@z->1). a@a result: (a@a->1,b@y->1).
        // snap: a@a: 0(Z) vs 1(A) → Z < A. Z (deleter) integrates first.
        // After B: f = "hello\n"
        // After Z: B==C, apply delete → f gone
        // After A: B[f]=present, C[f]=absent, T[f]=present → rule 3: earlier delete wins
        let patches = vec![
            make_patch(
                "b@y",
                1,
                "()",
                "create",
                vec![text_change(
                    "f",
                    vec![EditOp::Insert(vec!["hello\n".to_owned()])],
                )],
            ),
            make_patch("z@z", 1, "(b@y->1)", "delete", vec![delete_change("f")]),
            make_patch(
                "a@a",
                1,
                "(b@y->1)",
                "put modify",
                vec![put_change("f", b"modified")],
            ),
        ];
        let result = replay(&patches, &ver("(a@a->1,b@y->1,z@z->1)")).unwrap();
        assert!(!result.tree.contains_key("f"));
        assert!(
            result
                .warnings
                .contains(&("f".to_owned(), "delete-wins".to_owned()))
        );
    }

    // ── Path-level rule 4: B absent, C+T present → later-create-wins

    #[test]
    fn rule4_later_create_wins() {
        // A and B both create the same file from empty base, concurrently.
        // The later one (in integration order) wins.
        let patches = vec![
            make_patch(
                "a@x",
                1,
                "()",
                "a creates",
                vec![put_change("f", b"content-a")],
            ),
            make_patch(
                "b@y",
                1,
                "()",
                "b creates",
                vec![put_change("f", b"content-b")],
            ),
        ];
        // a@x result: (a@x->1). b@y result: (b@y->1).
        // snap: a@x: 1 vs 0 → A > B. B goes first.
        // After B: f = "content-b"
        // After A: B[f]=absent(base ()), C[f]="content-b", T[f]="content-a"
        //   B≠C, C≠T, B absent, C+T present → rule 4: later-create-wins
        //   canonical[f] = T = "content-a". Warning: (f, later-create-wins)
        let result = replay(&patches, &ver("(a@x->1,b@y->1)")).unwrap();
        assert_eq!(result.tree.get("f").unwrap(), b"content-a");
        assert!(
            result
                .warnings
                .contains(&("f".to_owned(), "later-create-wins".to_owned()))
        );
    }

    // ── Path-level rule 5: incoming is put → later-put-wins ────────

    #[test]
    fn rule5_later_put_wins() {
        // A creates f. Z and B both put concurrently (based on A).
        // Z integrates first (Z < B in snap). B integrates later → later-put-wins.
        let patches = vec![
            make_patch("a@x", 1, "()", "create", vec![put_change("f", b"original")]),
            // b@y modifies with put (integrates earlier due to b@y < z@z)
            make_patch(
                "b@y",
                1,
                "(a@x->1)",
                "text edit",
                vec![put_change("f", b"edited")],
            ),
            // z@z also puts (integrates later)
            make_patch(
                "z@z",
                1,
                "(a@x->1)",
                "put replace",
                vec![put_change("f", b"replaced")],
            ),
        ];
        // Z integrates first (snap order). After Z: f="replaced". B integrates: put → rule 5.
        let result = replay(&patches, &ver("(a@x->1,b@y->1,z@z->1)")).unwrap();
        assert_eq!(result.tree.get("f").unwrap(), b"edited");
        assert!(
            result
                .warnings
                .contains(&("f".to_owned(), "later-put-wins".to_owned()))
        );
    }

    // ── Path-level rule 6: P text, C non-text → put-wins ──────────

    #[test]
    fn rule6_put_wins() {
        // B creates text file. z@z puts binary, a@a text-edits. Both concurrent, based on B.
        // z@z result (b@y->1,z@z->1) < a@a result (a@a->1,b@y->1) in snap → z@z first.
        // After z@z: f = binary. After a@a: P is text, C is non-text → put-wins.
        let patches = vec![
            make_patch(
                "b@y",
                1,
                "()",
                "create text",
                vec![text_change(
                    "f",
                    vec![EditOp::Insert(vec!["hello\n".to_owned()])],
                )],
            ),
            make_patch(
                "z@z",
                1,
                "(b@y->1)",
                "put binary",
                vec![put_change("f", &[0, 1, 2])],
            ),
            make_patch(
                "a@a",
                1,
                "(b@y->1)",
                "text edit",
                vec![text_change(
                    "f",
                    vec![
                        EditOp::Delete(1),
                        EditOp::Insert(vec!["world\n".to_owned()]),
                    ],
                )],
            ),
        ];
        let result = replay(&patches, &ver("(a@a->1,b@y->1,z@z->1)")).unwrap();
        assert_eq!(result.tree.get("f").unwrap(), &[0, 1, 2]);
        assert!(
            result
                .warnings
                .contains(&("f".to_owned(), "put-wins".to_owned()))
        );
    }

    // ── Namespace conflicts ────────────────────────────────────────

    #[test]
    fn namespace_conflict_file_vs_directory() {
        // A creates "a" as a file. B creates "a/b" (needs "a" as directory).
        // B integrates later → namespace-wins on "a" (removal of file "a")
        let patches = vec![
            make_patch(
                "a@x",
                1,
                "()",
                "create file a",
                vec![put_change("a", b"file content")],
            ),
            make_patch(
                "b@y",
                1,
                "()",
                "create a/b",
                vec![put_change("a/b", b"nested")],
            ),
        ];
        // a@x result: (a@x->1). b@y result: (b@y->1).
        // snap: a@x 1 vs 0 → A > B. B first.
        // After B: a/b = "nested"
        // After A: A creates "a" (file). S = {"a"}. C' = canonical = {a/b: "nested"}.
        //   "a" is ancestor of "a/b" → namespace conflict!
        //   Remove "a/b", install "a". Warning: (a/b, namespace-wins).
        let result = replay(&patches, &ver("(a@x->1,b@y->1)")).unwrap();
        assert!(result.tree.contains_key("a"));
        assert!(!result.tree.contains_key("a/b"));
        assert!(
            result
                .warnings
                .contains(&("a/b".to_owned(), "namespace-wins".to_owned()))
        );
    }

    #[test]
    fn namespace_conflict_directory_vs_file() {
        // A creates "a/b". B creates "a" (file). B integrates later.
        let patches = vec![
            make_patch(
                "a@x",
                1,
                "()",
                "create a/b",
                vec![put_change("a/b", b"nested")],
            ),
            make_patch(
                "b@y",
                1,
                "()",
                "create file a",
                vec![put_change("a", b"file content")],
            ),
        ];
        // a@x result: (a@x->1). b@y result: (b@y->1).
        // snap: a@x 1 vs 0 → A > B. B first.
        // After B: a = "file content"
        // After A: S = {"a/b"}. C' has "a" (file). "a" is ancestor of "a/b" → conflict.
        //   Remove "a", install "a/b". Warning: (a, namespace-wins).
        let result = replay(&patches, &ver("(a@x->1,b@y->1)")).unwrap();
        assert!(result.tree.contains_key("a/b"));
        assert!(!result.tree.contains_key("a"));
        assert!(
            result
                .warnings
                .contains(&("a".to_owned(), "namespace-wins".to_owned()))
        );
    }

    // ── Warning collection ─────────────────────────────────────────

    #[test]
    fn warnings_sorted_and_unique() {
        // Multiple conflicts, verify warnings are sorted by path then reason
        let patches = vec![
            make_patch(
                "a@x",
                1,
                "()",
                "create",
                vec![put_change("b", b"b-a"), put_change("c", b"c-a")],
            ),
            make_patch(
                "b@y",
                1,
                "()",
                "also create",
                vec![put_change("b", b"b-b"), put_change("c", b"c-b")],
            ),
        ];
        let result = replay(&patches, &ver("(a@x->1,b@y->1)")).unwrap();
        // Both b and c have concurrent creates → later-create-wins
        let warning_paths: Vec<&str> = result.warnings.iter().map(|(p, _)| p.as_str()).collect();
        assert!(warning_paths.windows(2).all(|w| w[0] <= w[1]));
    }

    // ── Caching: base lookup for join versions ─────────────────────

    #[test]
    fn replay_with_join_base() {
        // A and B create files concurrently. C is based on join of A and B.
        // This tests the sub-replay caching when the base is a join version.
        let patches = vec![
            make_patch(
                "a@x",
                1,
                "()",
                "create a.txt",
                vec![text_change(
                    "a.txt",
                    vec![EditOp::Insert(vec!["a\n".to_owned()])],
                )],
            ),
            make_patch(
                "b@y",
                1,
                "()",
                "create b.txt",
                vec![text_change(
                    "b.txt",
                    vec![EditOp::Insert(vec!["b\n".to_owned()])],
                )],
            ),
            make_patch(
                "c@z",
                1,
                "(a@x->1,b@y->1)",
                "add c.txt",
                vec![text_change(
                    "c.txt",
                    vec![EditOp::Insert(vec!["c\n".to_owned()])],
                )],
            ),
        ];
        let result = replay(&patches, &ver("(a@x->1,b@y->1,c@z->1)")).unwrap();
        assert_eq!(result.tree.len(), 3);
        assert!(result.tree.contains_key("a.txt"));
        assert!(result.tree.contains_key("b.txt"));
        assert!(result.tree.contains_key("c.txt"));
        assert!(result.warnings.is_empty());
    }

    #[test]
    fn replay_with_join_base_and_interleaved_concurrent() {
        // A1, B1, D1 all concurrent. C1 based on (a@x->1, b@y->1).
        // D1 integrates between A1 and B1. Tests sub-replay for join base.
        let patches = vec![
            make_patch(
                "a@x",
                1,
                "()",
                "a",
                vec![text_change(
                    "a.txt",
                    vec![EditOp::Insert(vec!["a\n".to_owned()])],
                )],
            ),
            make_patch(
                "b@y",
                1,
                "()",
                "b",
                vec![text_change(
                    "b.txt",
                    vec![EditOp::Insert(vec!["b\n".to_owned()])],
                )],
            ),
            make_patch(
                "d@w",
                1,
                "()",
                "d",
                vec![text_change(
                    "d.txt",
                    vec![EditOp::Insert(vec!["d\n".to_owned()])],
                )],
            ),
            make_patch(
                "c@z",
                1,
                "(a@x->1,b@y->1)",
                "c",
                vec![text_change(
                    "c.txt",
                    vec![EditOp::Insert(vec!["c\n".to_owned()])],
                )],
            ),
        ];
        let result = replay(&patches, &ver("(a@x->1,b@y->1,c@z->1,d@w->1)")).unwrap();
        assert_eq!(result.tree.len(), 4);
        assert!(result.tree.contains_key("a.txt"));
        assert!(result.tree.contains_key("b.txt"));
        assert!(result.tree.contains_key("c.txt"));
        assert!(result.tree.contains_key("d.txt"));
    }

    // ── Delete integration ─────────────────────────────────────────

    #[test]
    fn replay_delete_removes_file() {
        let patches = vec![
            make_patch(
                "a@x",
                1,
                "()",
                "create",
                vec![text_change(
                    "f",
                    vec![EditOp::Insert(vec!["hello\n".to_owned()])],
                )],
            ),
            make_patch("a@x", 2, "(a@x->1)", "delete", vec![delete_change("f")]),
        ];
        let result = replay(&patches, &ver("(a@x->2)")).unwrap();
        assert!(!result.tree.contains_key("f"));
        assert!(result.warnings.is_empty());
    }

    // ── Replay purity (same input → same output) ──────────────────

    #[test]
    fn replay_is_deterministic() {
        let patches = vec![
            make_patch("a@x", 1, "()", "a", vec![put_change("f", b"content-a")]),
            make_patch("b@y", 1, "()", "b", vec![put_change("f", b"content-b")]),
        ];
        let target = ver("(a@x->1,b@y->1)");
        let r1 = replay(&patches, &target).unwrap();
        let r2 = replay(&patches, &target).unwrap();
        assert_eq!(r1, r2);
    }

    // ── Partial version replay ─────────────────────────────────────

    #[test]
    fn replay_partial_version() {
        let patches = vec![
            make_patch(
                "a@x",
                1,
                "()",
                "a",
                vec![text_change(
                    "a.txt",
                    vec![EditOp::Insert(vec!["a\n".to_owned()])],
                )],
            ),
            make_patch(
                "b@y",
                1,
                "()",
                "b",
                vec![text_change(
                    "b.txt",
                    vec![EditOp::Insert(vec!["b\n".to_owned()])],
                )],
            ),
        ];
        // Replay only a@x's patches
        let result = replay(&patches, &ver("(a@x->1)")).unwrap();
        assert_eq!(result.tree.len(), 1);
        assert!(result.tree.contains_key("a.txt"));
        assert!(!result.tree.contains_key("b.txt"));
    }
}

#[cfg(all(test, not(miri)))]
mod proptests {
    use super::*;
    use crate::text::{EditOp, EditScript};
    use crate::version::ContributorId;
    use proptest::prelude::*;

    fn arb_contributor_id() -> impl Strategy<Value = ContributorId> {
        prop::sample::select(vec!["a@x", "b@y", "c@z"]).prop_map(|s| ContributorId::new(s).unwrap())
    }

    fn arb_simple_patch_set() -> impl Strategy<Value = (Vec<Patch>, Version)> {
        prop::collection::vec(arb_contributor_id(), 1..=4).prop_filter_map(
            "valid patch set",
            |authors| {
                let mut patches = Vec::new();
                let mut rev_counts: std::collections::HashMap<String, u64> =
                    std::collections::HashMap::new();
                let mut frontier = Version::empty();

                for author in &authors {
                    let count = rev_counts.entry(author.as_str().to_owned()).or_insert(0);
                    *count += 1;
                    let revision = *count;
                    let base = frontier.clone();

                    let file_path = format!("file-{}-{revision}", author.as_str());
                    let change = Change::Text {
                        path: file_path,
                        edit: EditScript::new(vec![EditOp::Insert(vec![format!(
                            "content-{revision}\n"
                        )])])
                        .ok()?,
                    };

                    let entry = Patch {
                        author: author.clone(),
                        revision,
                        base,
                        message: format!("patch {revision}"),
                        changes: vec![change],
                    };
                    frontier = frontier.join(&entry.result_version());
                    patches.push(entry);
                }

                Some((patches, frontier))
            },
        )
    }

    proptest! {
        #[test]
        fn replay_is_pure((patches, target) in arb_simple_patch_set()) {
            let r1 = replay(&patches, &target).unwrap();
            let r2 = replay(&patches, &target).unwrap();
            prop_assert_eq!(r1.tree, r2.tree);
            prop_assert_eq!(r1.warnings, r2.warnings);
        }

        #[test]
        fn replay_produces_nonempty_tree_for_nonempty_patches((patches, target) in arb_simple_patch_set()) {
            let result = replay(&patches, &target).unwrap();
            prop_assert!(!result.tree.is_empty());
        }
    }
}
