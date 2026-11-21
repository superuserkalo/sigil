//! Comparison: two sources side by side. A tree diff says which files only one side has and
//! which differ; a line diff shows how one file's text changed (decoded data included).

use serde::Serialize;

use super::Listing;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Change {
    OnlyLeft,
    OnlyRight,
    Changed,
    Same,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeRow {
    pub path: String,
    pub change: Change,
    pub left_size: Option<u64>,
    pub right_size: Option<u64>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct TreeDiff {
    /// Every path that differs, sorted. Same files are counted, not listed.
    pub rows: Vec<TreeRow>,
    pub only_left: usize,
    pub only_right: usize,
    pub changed: usize,
    pub same: usize,
}

/// Compare two listings by path. Files on both sides are the same when their CRCs match (a VPK
/// has them; a folder file is hashed here) or, failing that, their bytes do.
pub fn compare_trees(left: &Listing, right: &Listing) -> TreeDiff {
    let mut diff = TreeDiff::default();
    for file in &left.files {
        match right.find(&file.path) {
            None => {
                diff.only_left += 1;
                diff.rows.push(TreeRow {
                    path: file.path.clone(),
                    change: Change::OnlyLeft,
                    left_size: Some(file.size),
                    right_size: None,
                });
            }
            Some(other) => {
                let same = file.size == other.size && same_content(left, right, &file.path, file.crc32, other.crc32);
                if same {
                    diff.same += 1;
                } else {
                    diff.changed += 1;
                    diff.rows.push(TreeRow {
                        path: file.path.clone(),
                        change: Change::Changed,
                        left_size: Some(file.size),
                        right_size: Some(other.size),
                    });
                }
            }
        }
    }
    for file in &right.files {
        if left.find(&file.path).is_none() {
            diff.only_right += 1;
            diff.rows.push(TreeRow {
                path: file.path.clone(),
                change: Change::OnlyRight,
                left_size: None,
                right_size: Some(file.size),
            });
        }
    }
    diff.rows.sort_by_key(|row| row.path.to_ascii_lowercase());
    diff
}

fn same_content(left: &Listing, right: &Listing, path: &str, a: Option<u32>, b: Option<u32>) -> bool {
    let crc = |listing: &Listing, known: Option<u32>| {
        known.or_else(|| listing.read(path).ok().map(|bytes| crate::vpk::crc32(&bytes)))
    };
    match (crc(left, a), crc(right, b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum LineOp {
    Same,
    Removed,
    Added,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiffLine {
    pub op: LineOp,
    pub left: Option<usize>,
    pub right: Option<usize>,
    pub text: String,
}

/// Lines past this on either side are not diffed; the tail is reported as replaced.
pub const DIFF_MAX_LINES: usize = 20_000;
/// Edit distance past this gives up on a minimal diff and reports a block replace.
const DIFF_MAX_EDITS: usize = 4_000;

/// A line diff (Myers), with line numbers on each side. Falls back to "all removed, all added"
/// past the size caps so a huge file cannot stall a worker.
pub fn diff_lines(left: &str, right: &str) -> Vec<DiffLine> {
    let a: Vec<&str> = left.lines().take(DIFF_MAX_LINES).collect();
    let b: Vec<&str> = right.lines().take(DIFF_MAX_LINES).collect();
    // Trim the common head and tail; the middle is what Myers has to work on.
    let head = a.iter().zip(&b).take_while(|(x, y)| x == y).count();
    let tail = a[head..]
        .iter()
        .rev()
        .zip(b[head..].iter().rev())
        .take_while(|(x, y)| x == y)
        .count();
    let mid_a = &a[head..a.len() - tail];
    let mid_b = &b[head..b.len() - tail];

    let mut out = Vec::with_capacity(a.len().max(b.len()) + 8);
    for (i, line) in a[..head].iter().enumerate() {
        out.push(DiffLine { op: LineOp::Same, left: Some(i + 1), right: Some(i + 1), text: (*line).to_string() });
    }
    let ops = myers(mid_a, mid_b).unwrap_or_else(|| {
        let mut ops = vec![LineOp::Removed; mid_a.len()];
        ops.extend(vec![LineOp::Added; mid_b.len()]);
        ops
    });
    let (mut i, mut j) = (head, head);
    for op in ops {
        match op {
            LineOp::Same => {
                out.push(DiffLine { op, left: Some(i + 1), right: Some(j + 1), text: a[i].to_string() });
                i += 1;
                j += 1;
            }
            LineOp::Removed => {
                out.push(DiffLine { op, left: Some(i + 1), right: None, text: a[i].to_string() });
                i += 1;
            }
            LineOp::Added => {
                out.push(DiffLine { op, left: None, right: Some(j + 1), text: b[j].to_string() });
                j += 1;
            }
        }
    }
    for k in 0..tail {
        out.push(DiffLine {
            op: LineOp::Same,
            left: Some(i + k + 1),
            right: Some(j + k + 1),
            text: a[i + k].to_string(),
        });
    }
    out
}

/// Myers' O(ND) diff. `None` when the edit distance passes the cap.
fn myers(a: &[&str], b: &[&str]) -> Option<Vec<LineOp>> {
    let n = a.len() as isize;
    let m = b.len() as isize;
    if n == 0 {
        return Some(vec![LineOp::Added; b.len()]);
    }
    if m == 0 {
        return Some(vec![LineOp::Removed; a.len()]);
    }
    let max = (n + m).min(DIFF_MAX_EDITS as isize);
    let offset = max;
    let mut v = vec![0isize; (2 * max + 2) as usize];
    let mut trace: Vec<Vec<isize>> = Vec::new();
    let mut found = false;
    'outer: for d in 0..=max {
        trace.push(v.clone());
        let mut k = -d;
        while k <= d {
            let index = (k + offset) as usize;
            let mut x = if k == -d || (k != d && v[index - 1] < v[index + 1]) {
                v[index + 1]
            } else {
                v[index - 1] + 1
            };
            let mut y = x - k;
            while x < n && y < m && a[x as usize] == b[y as usize] {
                x += 1;
                y += 1;
            }
            v[index] = x;
            if x >= n && y >= m {
                found = true;
                break 'outer;
            }
            k += 2;
        }
    }
    if !found {
        return None;
    }
    // Walk the trace back from (n, m).
    let mut ops = Vec::new();
    let (mut x, mut y) = (n, m);
    for d in (0..trace.len() as isize).rev() {
        let v = &trace[d as usize];
        let k = x - y;
        let prev_k = if k == -d || (k != d && v[(k - 1 + offset) as usize] < v[(k + 1 + offset) as usize]) {
            k + 1
        } else {
            k - 1
        };
        let prev_x = if d == 0 { 0 } else { v[(prev_k + offset) as usize] };
        let prev_y = prev_x - prev_k;
        while x > prev_x.max(0) && y > prev_y.max(0) && x > 0 && y > 0 && a[(x - 1) as usize] == b[(y - 1) as usize] {
            ops.push(LineOp::Same);
            x -= 1;
            y -= 1;
        }
        if d == 0 {
            break;
        }
        if x == prev_x {
            ops.push(LineOp::Added);
            y -= 1;
        } else {
            ops.push(LineOp::Removed);
            x -= 1;
        }
    }
    while x > 0 && y > 0 {
        ops.push(LineOp::Same);
        x -= 1;
        y -= 1;
    }
    ops.reverse();
    Some(ops)
}
