#[must_use]
pub fn is_text(bytes: &[u8]) -> bool {
    !bytes.contains(&0) && std::str::from_utf8(bytes).is_ok()
}

#[must_use]
pub fn tokenize(content: &str) -> Vec<&str> {
    if content.is_empty() {
        return Vec::new();
    }
    let mut tokens = Vec::new();
    let mut start = 0;
    for (i, b) in content.bytes().enumerate() {
        if b == b'\n' {
            tokens.push(&content[start..=i]);
            start = i + 1;
        }
    }
    if start < content.len() {
        tokens.push(&content[start..]);
    }
    tokens
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EditOp {
    Retain(usize),
    Delete(usize),
    Insert(Vec<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EditScript {
    ops: Vec<EditOp>,
}

impl EditScript {
    /// # Errors
    /// Returns an error if the script is not valid per §4.4.
    pub fn new(ops: Vec<EditOp>) -> Result<Self, EditScriptError> {
        validate_ops(&ops)?;
        Ok(Self { ops })
    }

    #[must_use]
    pub fn ops(&self) -> &[EditOp] {
        &self.ops
    }

    /// Apply this script to old tokens, producing new tokens.
    ///
    /// # Errors
    /// Returns an error if the script doesn't consume all old tokens.
    pub fn apply(&self, old: &[&str]) -> Result<Vec<String>, EditScriptError> {
        let mut result = Vec::new();
        let mut pos = 0;
        for op in &self.ops {
            match op {
                EditOp::Retain(n) => {
                    if pos + n > old.len() {
                        return Err(EditScriptError::IncompleteConsumption);
                    }
                    for token in &old[pos..pos + n] {
                        result.push((*token).to_owned());
                    }
                    pos += n;
                }
                EditOp::Delete(n) => {
                    if pos + n > old.len() {
                        return Err(EditScriptError::IncompleteConsumption);
                    }
                    pos += n;
                }
                EditOp::Insert(tokens) => {
                    result.extend(tokens.iter().cloned());
                }
            }
        }
        if pos != old.len() {
            return Err(EditScriptError::IncompleteConsumption);
        }
        Ok(result)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum EditScriptError {
    #[error("count must be positive")]
    ZeroCount,
    #[error("adjacent operations of the same kind")]
    AdjacentSameKind,
    #[error("script does not consume all old tokens")]
    IncompleteConsumption,
    #[error("insert contains empty token")]
    EmptyInsertToken,
    #[error("result tokens are not canonical")]
    NonCanonicalResult,
}

fn validate_ops(ops: &[EditOp]) -> Result<(), EditScriptError> {
    for (i, op) in ops.iter().enumerate() {
        match op {
            EditOp::Retain(n) | EditOp::Delete(n) => {
                if *n == 0 {
                    return Err(EditScriptError::ZeroCount);
                }
            }
            EditOp::Insert(tokens) => {
                if tokens.is_empty() {
                    return Err(EditScriptError::ZeroCount);
                }
                for t in tokens {
                    if t.is_empty() {
                        return Err(EditScriptError::EmptyInsertToken);
                    }
                    if t.as_bytes()
                        .iter()
                        .position(|&b| b == b'\n')
                        .is_some_and(|pos| pos < t.len() - 1)
                    {
                        return Err(EditScriptError::NonCanonicalResult);
                    }
                }
            }
        }
        if i > 0 {
            let same_kind = matches!(
                (&ops[i - 1], op),
                (EditOp::Retain(_), EditOp::Retain(_))
                    | (EditOp::Delete(_), EditOp::Delete(_))
                    | (EditOp::Insert(_), EditOp::Insert(_))
            );
            if same_kind {
                return Err(EditScriptError::AdjacentSameKind);
            }
        }
    }
    Ok(())
}

/// Compute the canonical diff edit script from old tokens to new tokens per §5.
#[must_use]
#[expect(
    clippy::many_single_char_names,
    clippy::suspicious_operation_groupings,
    reason = "direct transcription of §5's DP recurrence using its variable names"
)]
pub fn diff(old: &[&str], new: &[&str]) -> EditScript {
    let n = old.len();
    let m = new.len();

    let mut d = vec![vec![0usize; m + 1]; n + 1];

    d[n][m] = 0;
    for i in (0..n).rev() {
        d[i][m] = n - i;
    }
    for j in (0..m).rev() {
        d[n][j] = m - j;
    }
    for i in (0..n).rev() {
        for j in (0..m).rev() {
            if old[i] == new[j] {
                d[i][j] = d[i + 1][j + 1];
            } else {
                d[i][j] = 1 + d[i + 1][j].min(d[i][j + 1]);
            }
        }
    }

    let mut ops: Vec<EditOp> = Vec::new();
    let mut i = 0;
    let mut j = 0;

    while i < n || j < m {
        let tokens_equal = i < n && j < m && old[i] == new[j];
        if tokens_equal {
            coalesce_retain(&mut ops, 1);
            i += 1;
            j += 1;
        } else if i < n && (j >= m || d[i + 1][j] <= d[i][j + 1]) {
            coalesce_delete(&mut ops, 1);
            i += 1;
        } else {
            coalesce_insert(&mut ops, new[j]);
            j += 1;
        }
    }

    EditScript { ops }
}

fn coalesce_retain(ops: &mut Vec<EditOp>, n: usize) {
    if let Some(EditOp::Retain(count)) = ops.last_mut() {
        *count += n;
    } else {
        ops.push(EditOp::Retain(n));
    }
}

fn coalesce_delete(ops: &mut Vec<EditOp>, n: usize) {
    if let Some(EditOp::Delete(count)) = ops.last_mut() {
        *count += n;
    } else {
        ops.push(EditOp::Delete(n));
    }
}

fn coalesce_insert(ops: &mut Vec<EditOp>, token: &str) {
    if let Some(EditOp::Insert(tokens)) = ops.last_mut() {
        tokens.push(token.to_owned());
    } else {
        ops.push(EditOp::Insert(vec![token.to_owned()]));
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum TransformError {
    #[error("unmatched {kind} remaining after transform")]
    UnmatchedOps { kind: &'static str },
}

/// Transform incoming edit `P` so it applies after aggregate context edit `Q` (§6.3).
///
/// # Errors
/// Returns an error if P and Q consume different base token counts or if
/// unmatched retain/delete operations remain.
pub fn transform(
    incoming: &EditScript,
    context: &EditScript,
) -> Result<EditScript, TransformError> {
    use std::collections::VecDeque;

    let mut p: VecDeque<EditOp> = incoming.ops().iter().cloned().collect();
    let mut q: VecDeque<EditOp> = context.ops().iter().cloned().collect();
    let mut out: Vec<EditOp> = Vec::new();

    loop {
        match (p.front(), q.front()) {
            (None, None) => break,

            // Q insert: retain over it, consume Q only
            (_, Some(EditOp::Insert(tokens))) => {
                coalesce_retain(&mut out, tokens.len());
                q.pop_front();
            }

            // P insert: keep it, consume P only
            (Some(EditOp::Insert(tokens)), _) => {
                for t in tokens.clone() {
                    coalesce_insert(&mut out, &t);
                }
                p.pop_front();
            }

            // P retain, Q retain
            (Some(EditOp::Retain(pn)), Some(EditOp::Retain(qn))) => {
                let pn = *pn;
                let qn = *qn;
                let min = pn.min(qn);
                coalesce_retain(&mut out, min);
                split_or_consume(&mut p, pn, min);
                split_or_consume(&mut q, qn, min);
            }

            // P delete, Q retain
            (Some(EditOp::Delete(pn)), Some(EditOp::Retain(qn))) => {
                let pn = *pn;
                let qn = *qn;
                let min = pn.min(qn);
                coalesce_delete(&mut out, min);
                split_or_consume(&mut p, pn, min);
                split_or_consume(&mut q, qn, min);
            }

            // P retain/delete, Q delete: both consumed, no output
            (Some(EditOp::Retain(pn) | EditOp::Delete(pn)), Some(EditOp::Delete(qn))) => {
                let pn = *pn;
                let qn = *qn;
                let min = pn.min(qn);
                split_or_consume(&mut p, pn, min);
                split_or_consume(&mut q, qn, min);
            }

            // One side has retain/delete but other is exhausted → mismatch
            (Some(_), None) | (None, Some(_)) => {
                let kind = if p.front().is_some() { "P" } else { "Q" };
                return Err(TransformError::UnmatchedOps { kind });
            }
        }
    }

    Ok(EditScript { ops: out })
}

fn split_or_consume(deque: &mut std::collections::VecDeque<EditOp>, count: usize, consumed: usize) {
    let remaining = count - consumed;
    if remaining == 0 {
        deque.pop_front();
    } else {
        match deque.front_mut() {
            Some(EditOp::Retain(n) | EditOp::Delete(n)) => *n = remaining,
            _ => unreachable!(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // --- is_text ---

    #[test]
    fn text_empty_bytes() {
        assert!(is_text(b""));
    }

    #[test]
    fn text_valid_utf8() {
        assert!(is_text(b"hello world\n"));
    }

    #[test]
    fn text_rejects_nul() {
        assert!(!is_text(b"hello\x00world"));
    }

    #[test]
    fn text_rejects_invalid_utf8() {
        assert!(!is_text(b"\xff\xfe"));
    }

    #[test]
    fn text_allows_crlf() {
        assert!(is_text(b"line\r\n"));
    }

    // --- tokenize ---

    #[test]
    fn tokenize_empty() {
        let tokens = tokenize("");
        assert!(tokens.is_empty());
    }

    #[test]
    fn tokenize_single_line_with_lf() {
        assert_eq!(tokenize("hello\n"), vec!["hello\n"]);
    }

    #[test]
    fn tokenize_single_line_no_lf() {
        assert_eq!(tokenize("hello"), vec!["hello"]);
    }

    #[test]
    fn tokenize_multiple_lines() {
        assert_eq!(tokenize("a\nb\nc\n"), vec!["a\n", "b\n", "c\n"]);
    }

    #[test]
    fn tokenize_no_final_lf() {
        assert_eq!(tokenize("a\nb"), vec!["a\n", "b"]);
    }

    #[test]
    fn tokenize_crlf() {
        assert_eq!(tokenize("a\r\nb"), vec!["a\r\n", "b"]);
    }

    #[test]
    fn tokenize_spec_example() {
        // §4.4: "a\r\nb" → "a\r\n", "b"
        assert_eq!(tokenize("a\r\nb"), vec!["a\r\n", "b"]);
    }

    #[test]
    fn tokenize_only_lf() {
        assert_eq!(tokenize("\n"), vec!["\n"]);
    }

    #[test]
    fn tokenize_multiple_lf() {
        assert_eq!(tokenize("\n\n"), vec!["\n", "\n"]);
    }

    // --- EditScript validation ---

    #[test]
    fn script_rejects_zero_retain() {
        assert_eq!(
            EditScript::new(vec![EditOp::Retain(0)]),
            Err(EditScriptError::ZeroCount)
        );
    }

    #[test]
    fn script_rejects_zero_delete() {
        assert_eq!(
            EditScript::new(vec![EditOp::Delete(0)]),
            Err(EditScriptError::ZeroCount)
        );
    }

    #[test]
    fn script_rejects_empty_insert_vec() {
        assert_eq!(
            EditScript::new(vec![EditOp::Insert(vec![])]),
            Err(EditScriptError::ZeroCount)
        );
    }

    #[test]
    fn script_rejects_empty_insert_token() {
        assert_eq!(
            EditScript::new(vec![EditOp::Insert(vec![String::new()])]),
            Err(EditScriptError::EmptyInsertToken)
        );
    }

    #[test]
    fn script_rejects_adjacent_retains() {
        assert_eq!(
            EditScript::new(vec![EditOp::Retain(1), EditOp::Retain(1)]),
            Err(EditScriptError::AdjacentSameKind)
        );
    }

    #[test]
    fn script_rejects_adjacent_deletes() {
        assert_eq!(
            EditScript::new(vec![EditOp::Delete(1), EditOp::Delete(1)]),
            Err(EditScriptError::AdjacentSameKind)
        );
    }

    #[test]
    fn script_rejects_adjacent_inserts() {
        assert_eq!(
            EditScript::new(vec![
                EditOp::Insert(vec!["a\n".to_owned()]),
                EditOp::Insert(vec!["b\n".to_owned()])
            ]),
            Err(EditScriptError::AdjacentSameKind)
        );
    }

    #[test]
    fn script_valid_mixed() {
        assert!(
            EditScript::new(vec![
                EditOp::Delete(1),
                EditOp::Retain(2),
                EditOp::Insert(vec!["x\n".to_owned()])
            ])
            .is_ok()
        );
    }

    #[test]
    fn script_rejects_embedded_lf() {
        assert_eq!(
            EditScript::new(vec![EditOp::Insert(vec!["a\nb".to_owned()])]),
            Err(EditScriptError::NonCanonicalResult)
        );
    }

    #[test]
    fn script_allows_trailing_lf() {
        assert!(EditScript::new(vec![EditOp::Insert(vec!["a\n".to_owned()])]).is_ok());
    }

    #[test]
    fn script_allows_no_lf() {
        assert!(EditScript::new(vec![EditOp::Insert(vec!["a".to_owned()])]).is_ok());
    }

    // --- apply ---

    #[test]
    fn apply_empty_to_empty() {
        let script = EditScript::new(vec![]).unwrap();
        assert_eq!(script.apply(&[]).unwrap(), Vec::<String>::new());
    }

    #[test]
    fn apply_retain_all() {
        let script = EditScript::new(vec![EditOp::Retain(2)]).unwrap();
        let result = script.apply(&["a\n", "b\n"]).unwrap();
        assert_eq!(result, vec!["a\n", "b\n"]);
    }

    #[test]
    fn apply_delete_all() {
        let script = EditScript::new(vec![EditOp::Delete(2)]).unwrap();
        let result = script.apply(&["a\n", "b\n"]).unwrap();
        assert!(result.is_empty());
    }

    #[test]
    fn apply_insert_all() {
        let script = EditScript::new(vec![EditOp::Insert(vec![
            "x\n".to_owned(),
            "y\n".to_owned(),
        ])])
        .unwrap();
        let result = script.apply(&[]).unwrap();
        assert_eq!(result, vec!["x\n", "y\n"]);
    }

    #[test]
    fn apply_mixed() {
        // old: ["a\n", "b\n", "c\n"] → delete 1, retain 1, insert ["x\n"], delete 1
        let script = EditScript::new(vec![
            EditOp::Delete(1),
            EditOp::Retain(1),
            EditOp::Insert(vec!["x\n".to_owned()]),
            EditOp::Delete(1),
        ])
        .unwrap();
        let result = script.apply(&["a\n", "b\n", "c\n"]).unwrap();
        assert_eq!(result, vec!["b\n", "x\n"]);
    }

    #[test]
    fn apply_fails_on_incomplete_consumption() {
        let script = EditScript::new(vec![EditOp::Retain(1)]).unwrap();
        assert_eq!(
            script.apply(&["a\n", "b\n"]),
            Err(EditScriptError::IncompleteConsumption)
        );
    }

    // --- diff ---

    #[test]
    fn diff_empty_to_empty() {
        let script = diff(&[], &[]);
        assert!(script.ops().is_empty());
    }

    #[test]
    fn diff_empty_to_nonempty() {
        let script = diff(&[], &["a\n", "b\n"]);
        assert_eq!(
            script.ops(),
            &[EditOp::Insert(vec!["a\n".to_owned(), "b\n".to_owned()])]
        );
    }

    #[test]
    fn diff_nonempty_to_empty() {
        let script = diff(&["a\n", "b\n"], &[]);
        assert_eq!(script.ops(), &[EditOp::Delete(2)]);
    }

    #[test]
    fn diff_identical() {
        let script = diff(&["a\n", "b\n"], &["a\n", "b\n"]);
        assert_eq!(script.ops(), &[EditOp::Retain(2)]);
    }

    #[test]
    fn diff_single_insert() {
        let script = diff(&["a\n"], &["a\n", "b\n"]);
        assert_eq!(
            script.ops(),
            &[EditOp::Retain(1), EditOp::Insert(vec!["b\n".to_owned()])]
        );
    }

    #[test]
    fn diff_single_delete() {
        let script = diff(&["a\n", "b\n"], &["a\n"]);
        assert_eq!(script.ops(), &[EditOp::Retain(1), EditOp::Delete(1)]);
    }

    #[test]
    fn diff_replacement() {
        let script = diff(&["a\n"], &["b\n"]);
        assert_eq!(
            script.ops(),
            &[EditOp::Delete(1), EditOp::Insert(vec!["b\n".to_owned()])]
        );
    }

    #[test]
    fn diff_repeated_lines_golden() {
        // From 05-diff-goldens.yaml:
        // old: "a\nb\na\n" → tokens ["a\n", "b\n", "a\n"]
        // new: "b\na\na"   → tokens ["b\n", "a\n", "a"]
        // Expected script: delete 1, retain 2, insert ["a"]
        let old = tokenize("a\nb\na\n");
        let new = tokenize("b\na\na");
        let script = diff(&old, &new);
        assert_eq!(
            script.ops(),
            &[
                EditOp::Delete(1),
                EditOp::Retain(2),
                EditOp::Insert(vec!["a".to_owned()])
            ]
        );
    }

    #[test]
    fn diff_all_insert_from_empty() {
        // Creating a new file: old empty, new has content
        let new = tokenize("hello\nworld\n");
        let script = diff(&[], &new);
        assert_eq!(
            script.ops(),
            &[EditOp::Insert(vec![
                "hello\n".to_owned(),
                "world\n".to_owned()
            ])]
        );
    }

    #[test]
    fn diff_all_delete_to_empty() {
        let old = tokenize("hello\nworld\n");
        let script = diff(&old, &[]);
        assert_eq!(script.ops(), &[EditOp::Delete(2)]);
    }

    #[test]
    fn diff_no_final_lf() {
        let old = tokenize("a\nb");
        let new = tokenize("a\nc");
        let script = diff(&old, &new);
        assert_eq!(
            script.ops(),
            &[
                EditOp::Retain(1),
                EditOp::Delete(1),
                EditOp::Insert(vec!["c".to_owned()])
            ]
        );
    }

    #[test]
    fn diff_crlf_tokens() {
        let old = tokenize("a\r\nb\r\n");
        let new = tokenize("a\r\nc\r\n");
        let script = diff(&old, &new);
        assert_eq!(
            script.ops(),
            &[
                EditOp::Retain(1),
                EditOp::Delete(1),
                EditOp::Insert(vec!["c\r\n".to_owned()])
            ]
        );
    }

    #[test]
    fn diff_apply_round_trip() {
        let old = tokenize("a\nb\na\n");
        let new = tokenize("b\na\na");
        let script = diff(&old, &new);
        let result = script.apply(&old).unwrap();
        let expected: Vec<String> = new.iter().map(|s| (*s).to_owned()).collect();
        assert_eq!(result, expected);
    }

    #[test]
    fn diff_apply_round_trip_complex() {
        let old = tokenize("line1\nline2\nline3\nline4\n");
        let new = tokenize("line1\nnew\nline3\n");
        let script = diff(&old, &new);
        let result = script.apply(&old).unwrap();
        let expected: Vec<String> = new.iter().map(|s| (*s).to_owned()).collect();
        assert_eq!(result, expected);
    }

    #[test]
    fn diff_delete_on_tie() {
        // When D(i+1,j) == D(i,j+1), spec says choose delete (§5).
        // old=["a\n","b\n"], new=["b\n","a\n"]: at (0,0) "a\n"!="b\n"
        // and D(1,0)=1 == D(0,1)=1, so delete-on-tie gives delete first.
        let old = tokenize("a\nb\n");
        let new = tokenize("b\na\n");
        let script = diff(&old, &new);
        // delete-on-tie: delete 1, retain 1, insert ["a\n"]
        // insert-on-tie would give: insert ["b\n"], retain 1, delete 1
        assert_eq!(
            script.ops(),
            &[
                EditOp::Delete(1),
                EditOp::Retain(1),
                EditOp::Insert(vec!["a\n".to_owned()])
            ]
        );
    }

    #[test]
    fn diff_repeated_equal_lines() {
        // Equal tokens always retain — tie-breaking doesn't apply to matches.
        let old = tokenize("a\na\n");
        let new = tokenize("a\n");
        let script = diff(&old, &new);
        assert_eq!(script.ops(), &[EditOp::Retain(1), EditOp::Delete(1)]);
    }

    #[test]
    fn diff_golden_create_file() {
        // From 05-diff-goldens.yaml: creating added.txt with content "new" (no final LF)
        let script = diff(&[], &["new"]);
        assert_eq!(script.ops(), &[EditOp::Insert(vec!["new".to_owned()])]);
    }

    #[test]
    fn diff_golden_initial_commit() {
        // From 05-diff-goldens.yaml: creating repeated.txt with "a\nb\na\n"
        let old: Vec<&str> = vec![];
        let new = tokenize("a\nb\na\n");
        let script = diff(&old, &new);
        assert_eq!(
            script.ops(),
            &[EditOp::Insert(vec![
                "a\n".to_owned(),
                "b\n".to_owned(),
                "a\n".to_owned()
            ])]
        );
    }

    // --- transform ---

    fn script(ops: Vec<EditOp>) -> EditScript {
        EditScript::new(ops).unwrap()
    }

    #[track_caller]
    fn assert_transform_merge(base_text: &str, p_text: &str, q_text: &str, expected_text: &str) {
        let base = tokenize(base_text);
        let p = diff(&base, &tokenize(p_text));
        let q = diff(&base, &tokenize(q_text));
        let transformed = transform(&p, &q).unwrap();
        let after_q = q.apply(&base).unwrap();
        let after_q_refs: Vec<&str> = after_q.iter().map(String::as_str).collect();
        let final_result = transformed.apply(&after_q_refs).unwrap();
        let expected: Vec<String> = tokenize(expected_text)
            .iter()
            .map(|s| (*s).to_owned())
            .collect();
        assert_eq!(final_result, expected);
    }

    #[test]
    fn transform_both_empty() {
        let result = transform(&script(vec![]), &script(vec![])).unwrap();
        assert!(result.ops().is_empty());
    }

    #[test]
    fn transform_q_insert_only() {
        // Q inserts 2 tokens, P is empty → retain(2)
        let p = script(vec![]);
        let q = script(vec![EditOp::Insert(vec!["x\n".into(), "y\n".into()])]);
        let result = transform(&p, &q).unwrap();
        assert_eq!(result.ops(), &[EditOp::Retain(2)]);
    }

    #[test]
    fn transform_p_insert_only() {
        // P inserts 2 tokens, Q is empty → same insert
        let p = script(vec![EditOp::Insert(vec!["a\n".into(), "b\n".into()])]);
        let q = script(vec![]);
        let result = transform(&p, &q).unwrap();
        assert_eq!(
            result.ops(),
            &[EditOp::Insert(vec!["a\n".into(), "b\n".into()])]
        );
    }

    #[test]
    fn transform_retain_retain_equal() {
        let p = script(vec![EditOp::Retain(3)]);
        let q = script(vec![EditOp::Retain(3)]);
        let result = transform(&p, &q).unwrap();
        assert_eq!(result.ops(), &[EditOp::Retain(3)]);
    }

    #[test]
    fn transform_retain_retain_split() {
        // P: retain(5), Q: retain(3), delete(2)
        // retain(3) vs retain(3) → retain(3)
        // retain(2) vs delete(2) → nothing
        let p = script(vec![EditOp::Retain(5)]);
        let q = script(vec![EditOp::Retain(3), EditOp::Delete(2)]);
        let result = transform(&p, &q).unwrap();
        assert_eq!(result.ops(), &[EditOp::Retain(3)]);
    }

    #[test]
    fn transform_delete_retain_equal() {
        let p = script(vec![EditOp::Delete(3)]);
        let q = script(vec![EditOp::Retain(3)]);
        let result = transform(&p, &q).unwrap();
        assert_eq!(result.ops(), &[EditOp::Delete(3)]);
    }

    #[test]
    fn transform_delete_retain_split() {
        // P: delete(2), retain(1). Q: retain(3).
        // delete(2) vs retain(2) → delete(2), then retain(1) vs retain(1) → retain(1)
        let p = script(vec![EditOp::Delete(2), EditOp::Retain(1)]);
        let q = script(vec![EditOp::Retain(3)]);
        let result = transform(&p, &q).unwrap();
        assert_eq!(result.ops(), &[EditOp::Delete(2), EditOp::Retain(1)]);
    }

    #[test]
    fn transform_retain_delete_equal() {
        // P retains what Q deletes → nothing in output
        let p = script(vec![EditOp::Retain(2)]);
        let q = script(vec![EditOp::Delete(2)]);
        let result = transform(&p, &q).unwrap();
        assert!(result.ops().is_empty());
    }

    #[test]
    fn transform_retain_delete_split() {
        // P: retain(3). Q: delete(2), retain(1).
        // retain(2) vs delete(2) → nothing
        // retain(1) vs retain(1) → retain(1)
        let p = script(vec![EditOp::Retain(3)]);
        let q = script(vec![EditOp::Delete(2), EditOp::Retain(1)]);
        let result = transform(&p, &q).unwrap();
        assert_eq!(result.ops(), &[EditOp::Retain(1)]);
    }

    #[test]
    fn transform_delete_delete_equal() {
        // Both delete the same 2 base tokens → nothing
        let p = script(vec![EditOp::Delete(2)]);
        let q = script(vec![EditOp::Delete(2)]);
        let result = transform(&p, &q).unwrap();
        assert!(result.ops().is_empty());
    }

    #[test]
    fn transform_delete_delete_split() {
        // P: delete(3). Q: delete(2), retain(1).
        // delete(2) vs delete(2) → nothing
        // delete(1) vs retain(1) → delete(1)
        let p = script(vec![EditOp::Delete(3)]);
        let q = script(vec![EditOp::Delete(2), EditOp::Retain(1)]);
        let result = transform(&p, &q).unwrap();
        assert_eq!(result.ops(), &[EditOp::Delete(1)]);
    }

    #[test]
    fn transform_trailing_inserts_both_sides() {
        // P: retain(1), insert(["P\n"]). Q: retain(1), insert(["Q\n"]).
        // Step 1: P retain(1) vs Q retain(1) → retain(1)
        // Step 2: Q insert(["Q\n"]) → retain(1) (Q insert has priority), coalesced to retain(2)
        // Step 3: P insert(["P\n"]) → insert(["P\n"])
        let p = script(vec![EditOp::Retain(1), EditOp::Insert(vec!["P\n".into()])]);
        let q = script(vec![EditOp::Retain(1), EditOp::Insert(vec!["Q\n".into()])]);
        let result = transform(&p, &q).unwrap();
        assert_eq!(
            result.ops(),
            &[EditOp::Retain(2), EditOp::Insert(vec!["P\n".into()]),]
        );
    }

    #[test]
    fn transform_empty_p_nonempty_q() {
        // P is empty (no base), Q operates on base of 2 tokens
        // This is a base count mismatch → error
        let p = script(vec![]);
        let q = script(vec![EditOp::Retain(2)]);
        let err = transform(&p, &q).unwrap_err();
        assert!(matches!(err, TransformError::UnmatchedOps { .. }));
    }

    #[test]
    fn transform_nonempty_p_empty_q() {
        // P operates on 2 base tokens, Q is empty → error
        let p = script(vec![EditOp::Retain(2)]);
        let q = script(vec![]);
        let err = transform(&p, &q).unwrap_err();
        assert!(matches!(err, TransformError::UnmatchedOps { .. }));
    }

    #[test]
    fn transform_p_creates_from_empty_q_inserts() {
        // Both operate on empty base. P inserts, Q inserts.
        // Q insert priority: retain over Q insert, then P insert.
        let p = script(vec![EditOp::Insert(vec!["P\n".into()])]);
        let q = script(vec![EditOp::Insert(vec!["Q\n".into()])]);
        let result = transform(&p, &q).unwrap();
        assert_eq!(
            result.ops(),
            &[EditOp::Retain(1), EditOp::Insert(vec!["P\n".into()])]
        );
    }

    #[test]
    fn transform_concurrent_inserts_q_priority() {
        // Both insert at position 0 with base size 1.
        // Q insert has priority → retain over Q's, then P's insert, then retain base.
        // P: insert(["P\n"]), retain(1). Q: insert(["Q\n"]), retain(1).
        // Step 1: Q insert → retain(1) in output, consume Q only
        // Step 2: P insert → insert(["P\n"]) in output, consume P only
        // Step 3: P retain(1) vs Q retain(1) → retain(1)
        let p = script(vec![EditOp::Insert(vec!["P\n".into()]), EditOp::Retain(1)]);
        let q = script(vec![EditOp::Insert(vec!["Q\n".into()]), EditOp::Retain(1)]);
        let result = transform(&p, &q).unwrap();
        assert_eq!(
            result.ops(),
            &[
                EditOp::Retain(1),
                EditOp::Insert(vec!["P\n".into()]),
                EditOp::Retain(1),
            ]
        );
    }

    #[test]
    fn transform_q_insert_before_p_delete_survives() {
        assert_transform_merge(
            "0\n1\n2\n3\n4\n",
            "0\n2\n3\n4\n",
            "0\nB\n1\n2\n3\n4\n",
            "0\nB\n2\n3\n4\n",
        );
    }

    #[test]
    fn transform_retain_and_append_vs_delete() {
        assert_transform_merge(
            "0\n1\n2\n3\n4\n",
            "0\n1\n2\n3\n4\nA\n",
            "0\n2\n3\n4\n",
            "0\n2\n3\n4\nA\n",
        );
    }

    #[test]
    fn transform_overlapping_deletes_from_yaml() {
        assert_transform_merge("0\n1\n2\n3\n4\n", "0\n3\n4\n", "0\n2\n3\n4\n", "0\n3\n4\n");
    }

    #[test]
    fn transform_complex_mixed_from_yaml() {
        assert_transform_merge(
            "0\n1\n2\n3\n4\n",
            "A\n0\n3\n4\nTAIL\n",
            "0\n1\nB\n3\n4\n",
            "A\n0\nB\n3\n4\nTAIL\n",
        );
    }
}
