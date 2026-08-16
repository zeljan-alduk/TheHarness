/// Merge overlapping or touching closed intervals. Input need not be sorted.
pub fn merge(mut v: Vec<(i64, i64)>) -> Vec<(i64, i64)> {
    v.sort_by_key(|p| p.0);
    let mut out: Vec<(i64, i64)> = Vec::new();
    for (a, b) in v {
        if let Some(last) = out.last_mut() {
            if a < last.1 {
                last.1 = b;
                continue;
            }
        }
        out.push((a, b));
    }
    out
}
